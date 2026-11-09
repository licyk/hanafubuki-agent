use super::evidence::{current_millis, render_history_evidence};
use super::*;
use hanafubuki_agent_history::AgentMemoryHistoryCandidate;

const MEMORY_LEASE_MILLIS: u64 = 5 * 60 * 1_000;
const MEMORY_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
const PHASE_ONE_TOTAL_DEADLINE: Duration = Duration::from_secs(4 * 60);
const PHASE_TWO_TOTAL_DEADLINE: Duration = Duration::from_secs(30 * 60);
const CANDIDATE_PAGE: usize = 100;
/// The idle sessions one pass looks through for ones to extract.
const MAX_CANDIDATES_PER_PASS: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeaseBoundFailureKind {
    Cancelled,
    DeadlineExceeded,
    LeaseLost,
    Failed,
}

#[derive(Debug)]
struct LeaseBoundFailure {
    kind: LeaseBoundFailureKind,
    message: String,
    lease: MemoryLease,
}

impl LeaseBoundFailure {
    fn new(kind: LeaseBoundFailureKind, message: impl Into<String>, lease: MemoryLease) -> Self {
        Self {
            kind,
            message: message.into(),
            lease,
        }
    }
}

async fn run_lease_bound_operation<T, E, Operation, Renew>(
    initial_lease: MemoryLease,
    supervisor_cancellation: CancellationToken,
    operation_cancellation: CancellationToken,
    total_deadline: Duration,
    heartbeat_interval: Duration,
    mut renew: Renew,
    operation: Operation,
) -> Result<(T, MemoryLease), LeaseBoundFailure>
where
    E: std::fmt::Display,
    Operation: std::future::Future<Output = Result<T, E>>,
    Renew: FnMut(&MemoryLease, u64) -> Result<MemoryLease, AgentMemoryError>,
{
    let mut current_lease = initial_lease;
    let deadline = tokio::time::sleep(total_deadline);
    let first_heartbeat = tokio::time::Instant::now() + heartbeat_interval;
    let mut heartbeat = tokio::time::interval_at(first_heartbeat, heartbeat_interval);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let cancellation = wait_for_memory_cancellation(supervisor_cancellation);
    tokio::pin!(deadline);
    tokio::pin!(cancellation);
    tokio::pin!(operation);

    loop {
        tokio::select! {
            biased;
            result = &mut operation => {
                return result
                    .map(|value| (value, current_lease.clone()))
                    .map_err(|error| LeaseBoundFailure::new(
                        LeaseBoundFailureKind::Failed,
                        error.to_string(),
                        current_lease.clone(),
                    ));
            }
            _ = &mut cancellation => {
                operation_cancellation.cancel();
                return Err(LeaseBoundFailure::new(
                    LeaseBoundFailureKind::Cancelled,
                    "Agent memory operation was cancelled",
                    current_lease,
                ));
            }
            _ = &mut deadline => {
                operation_cancellation.cancel();
                return Err(LeaseBoundFailure::new(
                    LeaseBoundFailureKind::DeadlineExceeded,
                    "Agent memory operation exceeded its total deadline",
                    current_lease,
                ));
            }
            _ = heartbeat.tick() => {
                let now = match current_millis() {
                    Ok(now) => now,
                    Err(error) => {
                        operation_cancellation.cancel();
                        return Err(LeaseBoundFailure::new(
                            LeaseBoundFailureKind::Failed,
                            error,
                            current_lease,
                        ));
                    }
                };
                match renew(&current_lease, now) {
                    Ok(renewed) => current_lease = renewed,
                    Err(AgentMemoryError::LeaseLost) => {
                        operation_cancellation.cancel();
                        return Err(LeaseBoundFailure::new(
                            LeaseBoundFailureKind::LeaseLost,
                            AgentMemoryError::LeaseLost.to_string(),
                            current_lease,
                        ));
                    }
                    Err(error) => {
                        operation_cancellation.cancel();
                        return Err(LeaseBoundFailure::new(
                            LeaseBoundFailureKind::Failed,
                            format!("Agent memory lease renewal failed: {error}"),
                            current_lease,
                        ));
                    }
                }
            }
        }
    }
}

async fn wait_for_memory_cancellation(cancellation: CancellationToken) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn fail_source_after_operation(
    repository: &AgentMemoryRepository,
    failure: LeaseBoundFailure,
    exact_secrets: &[String],
) -> Result<(), String> {
    if failure.kind == LeaseBoundFailureKind::LeaseLost {
        return Ok(());
    }
    let error = hanafubuki_agent_memory::redact_memory_text(&failure.message, exact_secrets);
    match repository.fail_source(&failure.lease, current_millis()?, &error) {
        Ok(()) | Err(AgentMemoryError::LeaseLost) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn fail_consolidation_after_operation(
    repository: &AgentMemoryRepository,
    failure: LeaseBoundFailure,
    workspace: &hanafubuki_agent_memory::MemoryConsolidationWorkspace,
    exact_secrets: &[String],
) -> Result<(), String> {
    let release = release_consolidation_after_operation(repository, &failure, exact_secrets);
    let cleanup = repository
        .discard_consolidation_workspace(workspace)
        .map_err(|error| error.to_string());
    match (release, cleanup) {
        (Err(release), Err(cleanup)) => Err(format!(
            "{release}; Agent memory staging cleanup failed: {cleanup}"
        )),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn release_consolidation_after_operation(
    repository: &AgentMemoryRepository,
    failure: &LeaseBoundFailure,
    exact_secrets: &[String],
) -> Result<(), String> {
    if failure.kind == LeaseBoundFailureKind::LeaseLost {
        Ok(())
    } else {
        let error = hanafubuki_agent_memory::redact_memory_text(&failure.message, exact_secrets);
        match repository.fail_consolidation(&failure.lease, current_millis()?, &error) {
            Ok(()) | Err(AgentMemoryError::LeaseLost) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// The idle history sessions a pass can extract, claimed one at a time.
///
/// History lists the idle sessions newest first, while whether one may be
/// extracted (contributing, a revision not yet extracted, no backoff or
/// lease) is the memory store's to decide. So the listing is paged until a
/// session is claimed: newer sessions that were already extracted never
/// keep an older one waiting.
struct SourceClaims<'a> {
    history: &'a FileAgentHistoryStore,
    repository: &'a AgentMemoryRepository,
    query: AgentMemoryHistoryQuery,
    page: std::vec::IntoIter<AgentMemoryHistoryCandidate>,
    looked_at: usize,
}

impl<'a> SourceClaims<'a> {
    fn new(
        history: &'a FileAgentHistoryStore,
        repository: &'a AgentMemoryRepository,
        query: AgentMemoryHistoryQuery,
    ) -> Self {
        Self {
            history,
            repository,
            query,
            page: Vec::new().into_iter(),
            looked_at: 0,
        }
    }

    fn claim_next(&mut self) -> Result<Option<(AgentMemoryHistoryCandidate, MemoryLease)>, String> {
        while let Some(candidate) = self.next_candidate()? {
            let session_id = candidate.session_id.to_string();
            let revision = candidate.source_revision.value();
            self.repository
                .upsert_source_revision(&session_id, revision)
                .map_err(|error| error.to_string())?;
            if let Some(lease) = self
                .repository
                .claim_source(
                    &session_id,
                    revision,
                    current_millis()?,
                    MEMORY_LEASE_MILLIS,
                )
                .map_err(|error| error.to_string())?
            {
                return Ok(Some((candidate, lease)));
            }
        }
        Ok(None)
    }

    fn next_candidate(&mut self) -> Result<Option<AgentMemoryHistoryCandidate>, String> {
        if self.looked_at == MAX_CANDIDATES_PER_PASS {
            return Ok(None);
        }
        let candidate = match self.page.next() {
            Some(candidate) => Some(candidate),
            None => {
                self.page = self
                    .history
                    .list_memory_candidates(self.query)
                    .map_err(|error| error.to_string())?
                    .into_iter();
                self.page.next()
            }
        };
        if candidate.is_some() {
            self.looked_at += 1;
            self.query.older_than = candidate;
        }
        Ok(candidate)
    }
}

pub(super) fn run_memory_supervisor(
    application: AgentMemoryApplication,
    history: Arc<FileAgentHistoryStore>,
    receiver: mpsc::Receiver<MemoryWork>,
    cancellation: CancellationToken,
) {
    let runtime = match super::super::supervisor_runtime::build_agent_supervisor_runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            log::error!("Agent memory supervisor runtime failed: {error}");
            return;
        }
    };
    while !cancellation.is_cancelled() {
        let work = match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(work) => work,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        match work {
            MemoryWork::Scan(current_session_id) => {
                if let Err(error) = run_memory_pass(
                    &application,
                    history.as_ref(),
                    current_session_id,
                    &runtime,
                    cancellation.clone(),
                ) {
                    log::warn!("Agent memory background pass failed: {error}");
                }
            }
            MemoryWork::RecordCitationUsage { source_session_ids } => match current_millis() {
                Ok(now) => {
                    if let Err(error) = application
                        .repository
                        .record_citation_usage(&source_session_ids, now)
                    {
                        log::warn!("Agent memory citation usage update failed: {error}");
                    }
                }
                Err(error) => log::warn!("Agent memory citation usage clock failed: {error}"),
            },
        }
    }
}

fn run_memory_pass(
    application: &AgentMemoryApplication,
    history: &FileAgentHistoryStore,
    current_session_id: Option<AgentSessionId>,
    runtime: &tokio::runtime::Runtime,
    cancellation: CancellationToken,
) -> Result<(), String> {
    if cancellation.is_cancelled() {
        return Ok(());
    }
    let memory = application.config.settings()?.memory;
    if !memory.enabled || !memory.generate_memories {
        return Ok(());
    }
    let registry = application.config.providers()?;
    let provider_id = registry
        .default_provider_id
        .as_deref()
        .ok_or_else(|| "Agent memory generation requires a default Agent Provider".to_string())?;
    let provider = registry
        .providers
        .iter()
        .find(|provider| provider.id == provider_id && provider.enabled)
        .cloned()
        .ok_or_else(|| {
            "Agent memory generation requires an enabled default Agent Provider".to_string()
        })?;
    let provider_model = registry.model_policy.model.clone();
    let extraction_model = memory
        .extraction_model
        .clone()
        .unwrap_or_else(|| provider_model.clone());
    let resolved = resolve_provider_definition(provider, &registry.model_policy)?;
    let evidence_token_limit = application
        .core
        .model_context_window(
            registry.model_policy.context_window_tokens,
            &resolved.definition.base_url,
            &extraction_model,
        )
        .unwrap_or(160_000)
        .saturating_mul(70)
        .saturating_div(100)
        .min(120_000) as usize;
    let exact_secrets = resolved.credential.clone().into_iter().collect::<Vec<_>>();
    let model_session = AgentService::create_private_model_session(
        model_api_protocol(resolved.definition.protocol),
        &resolved.definition.base_url,
        resolved.credential,
        resolved.headers,
        hanafubuki_agent_kernel::ModelRetryPolicy::default(),
    )
    .map_err(|error| error.to_string())?;
    let mut sources = SourceClaims::new(
        history,
        &application.repository,
        AgentMemoryHistoryQuery {
            current_session_id,
            now_millis: u128::from(current_millis()?),
            minimum_idle_millis: u64::from(memory.minimum_idle_hours)
                .saturating_mul(60 * 60 * 1000),
            maximum_age_millis: u64::from(memory.max_source_age_days)
                .saturating_mul(24 * 60 * 60 * 1000),
            limit: CANDIDATE_PAGE,
            older_than: None,
        },
    );
    for _ in 0..memory.max_sessions_per_pass {
        if cancellation.is_cancelled() {
            return Ok(());
        }
        let Some((candidate, lease)) = sources.claim_next()? else {
            break;
        };
        let evidence = match history
            .load_memory_evidence(candidate, 2_000, 1_800_000)
            .map(|evidence| render_history_evidence(evidence, evidence_token_limit))
            .map_err(|error| error.to_string())
        {
            Ok(evidence) => evidence,
            Err(error) => {
                fail_source_after_operation(
                    &application.repository,
                    LeaseBoundFailure::new(LeaseBoundFailureKind::Failed, error, lease),
                    &exact_secrets,
                )?;
                continue;
            }
        };
        let operation_cancellation = CancellationToken::default();
        let outcome = runtime.block_on(run_lease_bound_operation(
            lease,
            cancellation.clone(),
            operation_cancellation.clone(),
            PHASE_ONE_TOTAL_DEADLINE,
            MEMORY_HEARTBEAT_INTERVAL,
            |lease, heartbeat_now| {
                application
                    .repository
                    .renew_source(lease, heartbeat_now, MEMORY_LEASE_MILLIS)
            },
            generate_stage_one(
                model_session.clone(),
                extraction_model.clone(),
                evidence,
                &exact_secrets,
                operation_cancellation,
            ),
        ));
        match outcome {
            Ok((result, current_lease)) => application
                .repository
                .complete_source(&current_lease, result.value, current_millis()?)
                .map_err(|error| error.to_string())?,
            Err(failure) => {
                fail_source_after_operation(&application.repository, failure, &exact_secrets)?
            }
        }
    }

    let consolidation_now = current_millis()?;
    application
        .repository
        .expire_unused_stage_one(consolidation_now, memory.max_unused_days)
        .map_err(|error| error.to_string())?;
    let Some(lease) = application
        .repository
        .claim_consolidation(consolidation_now, MEMORY_LEASE_MILLIS)
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    let inputs = match application
        .repository
        .consolidation_inputs(memory.max_stage1_outputs)
    {
        Ok(inputs) => inputs,
        Err(error) => {
            let failure =
                LeaseBoundFailure::new(LeaseBoundFailureKind::Failed, error.to_string(), lease);
            let message =
                hanafubuki_agent_memory::redact_memory_text(&failure.message, &exact_secrets);
            release_consolidation_after_operation(
                &application.repository,
                &failure,
                &exact_secrets,
            )?;
            return Err(message);
        }
    };
    let workspace = match application
        .repository
        .prepare_consolidation_workspace(lease.source_revision.saturating_add(1), &inputs)
    {
        Ok(workspace) => workspace,
        Err(error) => {
            let failure =
                LeaseBoundFailure::new(LeaseBoundFailureKind::Failed, error.to_string(), lease);
            let message =
                hanafubuki_agent_memory::redact_memory_text(&failure.message, &exact_secrets);
            release_consolidation_after_operation(
                &application.repository,
                &failure,
                &exact_secrets,
            )?;
            return Err(message);
        }
    };
    if !workspace.has_semantic_changes {
        return match application.repository.complete_consolidation_noop(
            &lease,
            &workspace,
            current_millis()?,
        ) {
            Ok(_) => application
                .repository
                .discard_consolidation_workspace(&workspace)
                .map_err(|error| error.to_string()),
            Err(error) => fail_consolidation_after_operation(
                &application.repository,
                LeaseBoundFailure::new(
                    if error == AgentMemoryError::LeaseLost {
                        LeaseBoundFailureKind::LeaseLost
                    } else {
                        LeaseBoundFailureKind::Failed
                    },
                    error.to_string(),
                    lease,
                ),
                &workspace,
                &exact_secrets,
            ),
        };
    }
    let operation_cancellation = CancellationToken::default();
    let outcome = runtime.block_on(run_lease_bound_operation(
        lease,
        cancellation,
        operation_cancellation.clone(),
        PHASE_TWO_TOTAL_DEADLINE,
        MEMORY_HEARTBEAT_INTERVAL,
        |lease, heartbeat_now| {
            application
                .repository
                .renew_consolidation(lease, heartbeat_now, MEMORY_LEASE_MILLIS)
        },
        run_memory_workspace_agent(
            model_session,
            memory.consolidation_model.clone().unwrap_or(provider_model),
            workspace.path.clone(),
            &exact_secrets,
            operation_cancellation,
        ),
    ));
    match outcome {
        Ok((_, current_lease)) => match application.repository.publish_consolidation_workspace(
            &current_lease,
            &workspace,
            current_millis()?,
        ) {
            Ok(_) => Ok(()),
            Err(error) => fail_consolidation_after_operation(
                &application.repository,
                LeaseBoundFailure::new(
                    if error == AgentMemoryError::LeaseLost {
                        LeaseBoundFailureKind::LeaseLost
                    } else {
                        LeaseBoundFailureKind::Failed
                    },
                    error.to_string(),
                    current_lease,
                ),
                &workspace,
                &exact_secrets,
            ),
        },
        Err(failure) => fail_consolidation_after_operation(
            &application.repository,
            failure,
            &workspace,
            &exact_secrets,
        ),
    }
}

#[cfg(test)]
mod tests;
