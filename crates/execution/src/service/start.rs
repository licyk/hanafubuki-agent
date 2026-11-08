use super::*;

impl AgentExecutionService {
    #[cfg(any(test, feature = "test-support"))]
    pub fn start(
        &self,
        scope: AgentExecutionScope,
        request: crate::StartCommandRequest,
        approval: Option<&AuthorizedAction>,
        cancellation: &CancellationToken,
    ) -> Result<AgentExecutionSnapshot, AgentExecutionError> {
        let prepared = crate::prepare_command(&request)?;
        exact_approval(prepared.action(), approval)?;
        if cancellation.is_cancelled() {
            return Err(AgentExecutionError::Cancelled);
        }
        let snapshot = self.start_plan(
            scope.clone(),
            prepared.into_plan(),
            AgentExecutionKind::Command,
            None,
            cancellation,
        )?;
        // A unified exec that was committed survives cancellation as a background
        // terminal; a plain argv command has no such surface, so it is disposed.
        if cancellation.is_cancelled() {
            return match self.dispose(&scope, snapshot.execution_id) {
                Ok(_) => Err(AgentExecutionError::Cancelled),
                Err(error) => Err(AgentExecutionError::CleanupRetained {
                    execution_id: snapshot.execution_id,
                    message: error.to_string(),
                }),
            };
        }
        Ok(snapshot)
    }

    pub fn start_exec_command(
        &self,
        scope: AgentExecutionScope,
        request: ExecCommandRequest,
        approval: Option<&AuthorizedAction>,
        cancellation: &CancellationToken,
    ) -> Result<AgentExecutionSnapshot, AgentExecutionError> {
        let prepared = prepare_exec_command(&request)?;
        exact_approval(prepared.action(), approval)?;
        self.start_plan(
            scope,
            prepared.plan,
            AgentExecutionKind::UnifiedExec,
            Some(bound_utf8(&request.cmd, MAX_COMMAND_DISPLAY_BYTES)),
            cancellation,
        )
    }

    fn start_plan(
        &self,
        scope: AgentExecutionScope,
        plan: ResolvedCommandPlan,
        kind: AgentExecutionKind,
        command_display: Option<String>,
        cancellation: &CancellationToken,
    ) -> Result<AgentExecutionSnapshot, AgentExecutionError> {
        let start_plan_started = Instant::now();
        log::info!(
            "agent_diag stage=execution.start_plan.begin interactive={} io_mode={:?}",
            plan.interactive,
            plan.io_mode
        );
        let cwd_display = bound_utf8(&plan.cwd.to_string_lossy(), MAX_CWD_DISPLAY_BYTES);
        let (execution_id, process_id) =
            self.reserve_record(scope, kind, plan.interactive, command_display, cwd_display)?;
        log::info!(
            "agent_diag stage=execution.record_reserved execution_id={execution_id} elapsed_ms={}",
            start_plan_started.elapsed().as_millis()
        );
        let owner = ProcessOwnerId::generate();
        let ownership_started = Instant::now();
        log::info!(
            "agent_diag stage=execution.process_reservation.begin execution_id={execution_id}"
        );
        let reservation = match self.inner.process.reserve(&process_id, &owner) {
            Ok(reservation) => {
                log::info!(
                    "agent_diag stage=execution.process_reservation.end execution_id={execution_id} outcome=ok elapsed_ms={}",
                    ownership_started.elapsed().as_millis()
                );
                reservation
            }
            Err(error) => {
                log::info!(
                    "agent_diag stage=execution.process_reservation.end execution_id={execution_id} outcome=error elapsed_ms={}",
                    ownership_started.elapsed().as_millis()
                );
                self.remove_uncommitted(execution_id);
                return Err(AgentExecutionError::Process(error));
            }
        };
        if cancellation.is_cancelled() {
            self.inner.process.finalize_reservation(&reservation);
            self.remove_uncommitted(execution_id);
            return Err(AgentExecutionError::Cancelled);
        }
        let request = executor_request(process_id.clone(), plan);
        let spawn_started = Instant::now();
        log::info!("agent_diag stage=execution.process_spawn.begin execution_id={execution_id}");
        let (info, receipt) = match self.inner.process.start_reserved(request, &reservation) {
            Ok(started) => {
                log::info!(
                    "agent_diag stage=execution.process_spawn.end execution_id={execution_id} outcome=ok elapsed_ms={}",
                    spawn_started.elapsed().as_millis()
                );
                started
            }
            Err(error) => {
                log::info!(
                    "agent_diag stage=execution.process_spawn.end execution_id={execution_id} outcome=error elapsed_ms={}",
                    spawn_started.elapsed().as_millis()
                );
                self.inner.process.finalize_reservation(&reservation);
                self.remove_uncommitted(execution_id);
                return Err(AgentExecutionError::Process(error));
            }
        };
        let commit_started = Instant::now();
        log::info!("agent_diag stage=execution.receipt_commit.begin execution_id={execution_id}");
        if let Err(commit_error) = self.commit_receipt(execution_id, receipt, &info) {
            log::info!(
                "agent_diag stage=execution.receipt_commit.end execution_id={execution_id} outcome=error elapsed_ms={}",
                commit_started.elapsed().as_millis()
            );
            let scope = self.scope(execution_id)?;
            return match self.dispose(&scope, execution_id) {
                Ok(_) => Err(commit_error),
                Err(cleanup_error) => Err(AgentExecutionError::CleanupRetained {
                    execution_id,
                    message: format!("{commit_error}; cleanup failed: {cleanup_error}"),
                }),
            };
        }
        log::info!(
            "agent_diag stage=execution.receipt_commit.end execution_id={execution_id} outcome=ok elapsed_ms={}",
            commit_started.elapsed().as_millis()
        );
        let result = self.snapshot_by_id(execution_id);
        log::info!(
            "agent_diag stage=execution.start_plan.end execution_id={execution_id} outcome={} elapsed_ms={}",
            if result.is_ok() { "ok" } else { "error" },
            start_plan_started.elapsed().as_millis()
        );
        result
    }
}

pub(super) fn exact_approval(
    action: &hanafubuki_agent_approval::ApprovalAction,
    approval: Option<&AuthorizedAction>,
) -> Result<(), AgentExecutionError> {
    let approved = approval.ok_or(AgentExecutionError::ApprovalRequired)?;
    if approved.action() != action {
        return Err(AgentExecutionError::ApprovalMismatch);
    }
    Ok(())
}
