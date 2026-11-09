use super::*;
use hanafubuki_agent_approval::{ApprovalOrigin, ApprovalService};
use hanafubuki_agent_config::AgentProviderHeaderSettings;
use hanafubuki_agent_execution::{AgentExecutionScope, AgentExecutionSnapshot, ExecCommandRequest};
use hanafubuki_agent_kernel::{
    AgentActivityAction, AgentExecutionProgressStatus, AgentToolProgressUpdate,
};

/// A complete Provider draft: endpoint and credentials. Which model to run,
/// and how, belongs to the shared policy rather than to this draft.
fn provider_draft(base_url: &str) -> AgentProviderDraft {
    AgentProviderDraft {
        display_name: "Fixture Provider".into(),
        enabled: true,
        protocol: AgentModelProtocol::Responses,
        base_url: base_url.into(),
        api_token: None,
        request_headers: Vec::new(),
    }
}

/// Replace the fixture Provider's credentials through the ordinary write path.
async fn write_fixture_credentials(
    service: &AgentApplicationService,
    base_url: &str,
    api_token: &str,
    headers: Vec<(&str, &str)>,
) -> AgentProviderRegistrySnapshot {
    let current = service.provider_registry_snapshot().await.unwrap();
    let mut provider = provider_draft(base_url);
    provider.api_token = Some(api_token.into());
    provider.request_headers = headers
        .into_iter()
        .map(|(name, value)| AgentProviderHeaderSettings {
            name: name.into(),
            value: value.into(),
        })
        .collect();
    service
        .write_provider(AgentProviderWriteRequest {
            expected_revision: current.revision,
            provider_id: Some("provider-1".into()),
            provider,
        })
        .await
        .unwrap()
}

pub(super) fn install_fake_background_execution(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
    command: &str,
) -> AgentExecutionSnapshot {
    use hanafubuki_agent_approval::test_support::ManualApprovalClock;
    use hanafubuki_agent_approval::{ApprovalContext, ApprovalDecisionSource};

    let request = ExecCommandRequest {
        cmd: command.to_string(),
        shell: None,
        login: true,
        tty: false,
        yield_time_ms: 5_000,
        max_output_bytes: Some(32 * 1024),
        workdir: Some(std::env::current_dir().unwrap()),
        environment: BTreeMap::new(),
        origin: ApprovalOrigin::Model,
    };
    let action = service
        .test_executions()
        .prepare_exec_command_action(&request)
        .unwrap();
    let context = ApprovalContext::from_canonical_bytes("call-1".to_string(), b"scope-1").unwrap();
    let approval = ApprovalService::new(Arc::new(ManualApprovalClock::new(1)))
        .authorize(
            action,
            &context,
            Duration::from_secs(1),
            ApprovalDecisionSource::FullAccess,
        )
        .unwrap();
    let scope = AgentExecutionScope::for_agent_session(session_id.value()).unwrap();
    let snapshot = service
        .test_executions()
        .start_exec_command(
            scope,
            request,
            Some(&approval),
            &CancellationToken::default(),
        )
        .unwrap();
    service
        .core
        .report_session_execution_progress(
            session_id,
            AgentToolProgressUpdate::ExecutionStarted {
                execution_id: snapshot.execution_id.value(),
                generation: snapshot.generation.value(),
                action: AgentActivityAction::fixed("exec_command"),
                status: AgentExecutionProgressStatus::Running,
            },
        )
        .unwrap();
    snapshot
}

#[cfg(unix)]
pub(super) fn approved_command() -> String {
    "printf approved-agent-command".to_string()
}

#[cfg(windows)]
pub(super) fn approved_command() -> String {
    "Write-Output approved-agent-command".to_string()
}

mod background_execution;
mod children;
mod edit;
mod export;
mod lifecycle;
mod permissions;
mod provider;
