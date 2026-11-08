use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentSessionId, AgentToolProgressSink, AgentToolProgressUpdate,
};
use hanafubuki_agent_execution::{
    AgentExecutionError, AgentExecutionGeneration, AgentExecutionOutput, AgentExecutionScope,
    AgentExecutionService, AgentExecutionSnapshot,
};
use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde_json::json;

use super::*;

#[derive(Default)]
struct RecordingProgressSink {
    updates: Mutex<Vec<AgentToolProgressUpdate>>,
}

impl AgentToolProgressSink for RecordingProgressSink {
    fn report(&self, update: AgentToolProgressUpdate) {
        self.updates.lock().unwrap().push(update);
    }
}

fn test_activity(progress: Arc<RecordingProgressSink>) -> ExecutionActivity {
    ExecutionActivity {
        session_id: AgentSessionId::new(1),
        progress,
    }
}

#[test]
fn unified_exec_watcher_publishes_before_wait_without_consuming_model_output() {
    use hanafubuki_agent_approval::test_support::ManualApprovalClock;
    use hanafubuki_agent_approval::{
        ApprovalContext, ApprovalDecisionSource, ApprovalOrigin, ApprovalService,
    };
    use hanafubuki_agent_execution::test_support::FakeAgentExecutionProcessPort;
    use hanafubuki_agent_execution::ExecCommandRequest;
    use std::collections::BTreeMap;

    let process = FakeAgentExecutionProcessPort::shared();
    let executions = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(1).unwrap();
    let request = ExecCommandRequest {
        cmd: "long-running-command".to_string(),
        shell: None,
        login: true,
        tty: false,
        yield_time_ms: 5_000,
        max_output_bytes: Some(32 * 1024),
        workdir: Some(std::env::current_dir().unwrap()),
        environment: BTreeMap::new(),
        origin: ApprovalOrigin::Model,
    };
    let action = executions.prepare_exec_command_action(&request).unwrap();
    let approval_context =
        ApprovalContext::from_canonical_bytes("call-1".to_string(), b"scope-1").unwrap();
    let approval = ApprovalService::new(Arc::new(ManualApprovalClock::new(1)))
        .authorize(
            action,
            &approval_context,
            Duration::from_secs(1),
            ApprovalDecisionSource::FullAccess,
        )
        .unwrap();
    let snapshot = executions
        .start_exec_command(
            scope.clone(),
            request.clone(),
            Some(&approval),
            &CancellationToken::default(),
        )
        .unwrap();
    let process_id = process.starts()[0].executor_id.clone().unwrap();
    process.set_output(&process_id, b"visible-before-yield\n".to_vec());

    let sink = Arc::new(RecordingProgressSink::default());
    let activity = test_activity(sink.clone());
    let workers = BackgroundWorkerGroup::default();
    start_execution_activity(
        &executions,
        &workers,
        &activity,
        scope.clone(),
        &snapshot,
        AgentActivityAction::fixed("exec_command"),
    )
    .unwrap();

    for _ in 0..100 {
        if sink.updates.lock().unwrap().iter().any(|update| {
            matches!(
                update,
                AgentToolProgressUpdate::ExecutionOutput { text, .. }
                    if text.contains("visible-before-yield")
            )
        }) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let updates = sink.updates.lock().unwrap().clone();
    assert!(matches!(
        updates.first(),
        Some(AgentToolProgressUpdate::ExecutionStarted { .. })
    ));
    assert!(updates.iter().any(|update| matches!(
        update,
        AgentToolProgressUpdate::ExecutionOutput { text, .. }
            if text.contains("visible-before-yield")
    )));

    process.finish("visible-before-yield\nterminal\n", 0);
    let result = executions
        .wait_unified_execution(
            &scope,
            snapshot.execution_id,
            request.yield_time_ms,
            request.max_output_bytes,
            &CancellationToken::default(),
        )
        .unwrap();
    assert!(result.output.contains("visible-before-yield"));
    assert!(result.output.contains("terminal"));
    assert_eq!(result.session_id, None);
    // Disposal waited for the monitor: the final output was reported, and
    // then the terminal status, before the result returned.
    let updates = sink.updates.lock().unwrap().clone();
    let final_output = updates
        .iter()
        .position(|update| {
            matches!(
                update,
                AgentToolProgressUpdate::ExecutionOutput { text, .. } if text.contains("terminal")
            )
        })
        .expect("final output reported before disposal");
    let terminal_status = updates
        .iter()
        .position(|update| {
            matches!(
                update,
                AgentToolProgressUpdate::ExecutionStatus { status, .. } if status.is_terminal()
            )
        })
        .expect("terminal status reported before disposal");
    assert!(final_output < terminal_status);
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(1))
        .is_empty());
}

#[test]
fn watcher_registration_failure_is_explicit_and_confirms_process_cleanup() {
    use hanafubuki_agent_approval::test_support::ManualApprovalClock;
    use hanafubuki_agent_approval::{
        ApprovalContext, ApprovalDecisionSource, ApprovalOrigin, ApprovalService,
    };
    use hanafubuki_agent_execution::test_support::FakeAgentExecutionProcessPort;
    use hanafubuki_agent_execution::ExecCommandRequest;
    use std::collections::BTreeMap;

    let process = FakeAgentExecutionProcessPort::shared();
    let executions = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(1).unwrap();
    let request = ExecCommandRequest {
        cmd: "watcher-registration".to_string(),
        shell: None,
        login: true,
        tty: false,
        yield_time_ms: 5_000,
        max_output_bytes: Some(4096),
        workdir: Some(std::env::current_dir().unwrap()),
        environment: BTreeMap::new(),
        origin: ApprovalOrigin::Model,
    };
    let action = executions.prepare_exec_command_action(&request).unwrap();
    let approval_context =
        ApprovalContext::from_canonical_bytes("call-1".to_string(), b"scope-1").unwrap();
    let approval = ApprovalService::new(Arc::new(ManualApprovalClock::new(1)))
        .authorize(
            action,
            &approval_context,
            Duration::from_secs(1),
            ApprovalDecisionSource::FullAccess,
        )
        .unwrap();
    let snapshot = executions
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approval),
            &CancellationToken::default(),
        )
        .unwrap();
    let workers = BackgroundWorkerGroup::default();
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(1))
        .is_empty());
    let sink = Arc::new(RecordingProgressSink::default());
    let activity = test_activity(sink.clone());

    assert!(matches!(
        start_execution_activity(
            &executions,
            &workers,
            &activity,
            scope,
            &snapshot,
            AgentActivityAction::fixed("exec_command"),
        ),
        Err(AgentExecutionError::Process(_))
    ));
    assert_eq!(process.session_count(), 0);
    assert_eq!(executions.registry_count(), 0);
    let updates = sink.updates.lock().unwrap();
    assert!(matches!(
        updates.first(),
        Some(AgentToolProgressUpdate::ExecutionStarted { .. })
    ));
    assert!(matches!(
        updates.last(),
        Some(AgentToolProgressUpdate::ExecutionDisposed { .. })
    ));
}

#[test]
fn retained_unified_exec_cleanup_is_published_as_a_resource_obligation() {
    let sink = Arc::new(RecordingProgressSink::default());
    let activity = test_activity(sink.clone());
    let snapshot: AgentExecutionSnapshot = serde_json::from_value(json!({
        "executionId": 17,
        "generation": 3,
        "kind": "unified_exec",
        "status": "exited",
        "exitCode": 0,
        "outputOffset": 4,
        "createdAtMs": 1,
        "updatedAtMs": 2,
        "cleanupRequired": true,
        "interactive": false,
        "lastError": "cleanup retained"
    }))
    .unwrap();
    let workers = BackgroundWorkerGroup::default();
    schedule_execution_cleanup_retry(
        &AgentExecutionService::with_process_port(
            hanafubuki_agent_execution::test_support::FakeAgentExecutionProcessPort::shared(),
        ),
        &workers,
        &activity,
        AgentExecutionScope::for_agent_session(1).unwrap(),
        &snapshot,
        AgentActivityAction::fixed("exec_command"),
        true,
    );

    let updates = sink.updates.lock().unwrap();
    assert!(matches!(
        updates.first(),
        Some(AgentToolProgressUpdate::ExecutionStarted {
            execution_id: 17,
            generation: 3,
            action,
            ..
        }) if *action == AgentActivityAction::fixed("exec_command")
    ));
    assert!(matches!(
        updates.get(1),
        Some(AgentToolProgressUpdate::ExecutionStatus {
            execution_id: 17,
            generation: 3,
            cleanup_required: true,
            ..
        })
    ));
    drop(updates);
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(1))
        .is_empty());
}

#[test]
fn execution_output_progress_preserves_content_is_bounded_and_capability_free() {
    let sink = Arc::new(RecordingProgressSink::default());
    let activity = test_activity(sink.clone());
    let output: AgentExecutionOutput = serde_json::from_value(json!({
        "executionId": 17,
        "status": "running",
        "exitCode": null,
        "chunks": [{
            "stream": "stdout",
            "text": format!("{}\nvisible\npassword=do-not-publish\n", "x".repeat(12 * 1024))
        }],
        "startOffset": 0,
        "nextOffset": 12345,
        "truncated": true
    }))
    .unwrap();

    report_execution_output_to(
        activity.progress.as_ref(),
        AgentExecutionGeneration::from_untrusted(3).unwrap(),
        &output,
    );

    let updates = sink.updates.lock().unwrap();
    let AgentToolProgressUpdate::ExecutionOutput {
        execution_id,
        generation,
        text,
        next_offset,
        truncated,
    } = &updates[0]
    else {
        panic!("execution output progress")
    };
    assert_eq!((*execution_id, *generation, *next_offset), (17, 3, 12345));
    assert!(*truncated);
    assert!(text.contains("visible"));
    assert!(text.contains("do-not-publish"));
    assert!(text.len() <= 8 * 1024);
    let serialized = serde_json::to_string(&updates[0]).unwrap();
    for forbidden in ["receipt", "owner", "executorId", "environment", "cwd"] {
        assert!(!serialized.contains(forbidden));
    }
}
