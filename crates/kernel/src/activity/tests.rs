use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalDecisionSource, ApprovalOrigin, ApprovalRequest,
    ApprovalResolution, ApprovalRiskClass,
};

use super::*;
use crate::AgentToolPresentation;
use crate::AgentTurnTerminalReason;

fn tool_event(call_id: &str, action: AgentActivityAction) -> AgentEventKind {
    AgentEventKind::ToolStarted {
        call_id: call_id.to_string(),
        name: "not-presented".to_string(),
        presentation: AgentToolPresentation {
            action,
            subject: None,
        },
    }
}

#[test]
fn stdin_and_duplicate_start_keep_the_execution_initiating_call() {
    let mut tracker = AgentActivityTracker::default();
    let start = AgentToolProgressUpdate::ExecutionStarted {
        execution_id: 7,
        generation: 2,
        action: AgentActivityAction::fixed("exec_command"),
        status: AgentExecutionProgressStatus::Running,
    };
    tracker.observe(
        AgentEventSequence::new(1),
        &AgentEventKind::ExecutionProgress {
            call_id: Some("original".into()),
            update: start.clone(),
        },
        10,
    );
    assert!(tracker
        .observe(
            AgentEventSequence::new(2),
            &AgentEventKind::ExecutionProgress {
                call_id: Some("later-stdin".into()),
                update: start
            },
            20
        )
        .is_none());
    let updated = tracker
        .observe(
            AgentEventSequence::new(3),
            &AgentEventKind::ExecutionProgress {
                call_id: Some("later-stdin".into()),
                update: AgentToolProgressUpdate::ExecutionOutput {
                    execution_id: 7,
                    generation: 2,
                    text: "still running".into(),
                    next_offset: 13,
                    truncated: false,
                },
            },
            30,
        )
        .unwrap();
    assert_eq!(
        updated.execution.unwrap().initiating_call_id.as_deref(),
        Some("original")
    );
    let restored = AgentActivityTracker::restore(tracker.persisted_snapshots());
    assert_eq!(
        restored.active_snapshots()[0]
            .execution
            .as_ref()
            .unwrap()
            .initiating_call_id
            .as_deref(),
        Some("original")
    );
}

#[test]
fn concurrent_tools_complete_independently() {
    let mut tracker = AgentActivityTracker::default();
    let first = tracker
        .observe(
            AgentEventSequence::new(1),
            &tool_event("a", AgentActivityAction::fixed("list_instances")),
            10,
        )
        .unwrap();
    let second = tracker
        .observe(
            AgentEventSequence::new(2),
            &tool_event("b", AgentActivityAction::fixed("read_file")),
            20,
        )
        .unwrap();
    assert_eq!(tracker.active_snapshots().len(), 2);

    let completed = tracker
        .observe(
            AgentEventSequence::new(3),
            &AgentEventKind::ToolExecutionFinished {
                call_id: "b".to_string(),
                is_error: false,
            },
            30,
        )
        .unwrap();
    assert_eq!(completed.id, second.id);
    assert_eq!(completed.phase, AgentActivityPhase::Succeeded);
    assert_eq!(tracker.active_snapshots(), vec![first]);
}

#[test]
fn restored_activity_ids_do_not_reuse_reset_presentation_sequences() {
    let mut original = AgentActivityTracker::default();
    let retained = original
        .observe(
            AgentEventSequence::new(1),
            &tool_event("retained", AgentActivityAction::fixed("read_file")),
            10,
        )
        .unwrap();
    original.observe(
        AgentEventSequence::new(2),
        &AgentEventKind::ToolCompleted {
            call_id: "retained".to_string(),
            is_error: false,
        },
        20,
    );

    let mut restored = AgentActivityTracker::restore(original.persisted_snapshots());
    let next = restored
        .observe(
            AgentEventSequence::new(1),
            &tool_event(
                "after-restart",
                AgentActivityAction::fixed("list_instances"),
            ),
            30,
        )
        .unwrap();

    assert_eq!(retained.id.value(), 1);
    assert_eq!(next.id.value(), 2);
    let ids = restored
        .persisted_snapshots()
        .into_iter()
        .map(|activity| activity.id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), restored.persisted_snapshots().len());
}

#[test]
fn terminal_failure_closes_every_active_activity() {
    let mut tracker = AgentActivityTracker::default();
    tracker.observe(
        AgentEventSequence::new(1),
        &AgentEventKind::SamplingStarted { step: 1 },
        10,
    );
    tracker.observe(
        AgentEventSequence::new(2),
        &tool_event("a", AgentActivityAction::fixed("exec_command")),
        20,
    );
    tracker.observe(
        AgentEventSequence::new(3),
        &AgentEventKind::TurnFailed {
            reason: AgentTurnTerminalReason::InternalFailure,
            message: "failed".to_string(),
        },
        30,
    );
    assert!(tracker.active_snapshots().is_empty());
    assert!(tracker
        .recent_snapshots()
        .iter()
        .all(|activity| activity.phase == AgentActivityPhase::Failed));
}

#[test]
fn approval_uses_only_the_safe_action_summary() {
    let mut tracker = AgentActivityTracker::default();
    let action = ApprovalAction::from_canonical_bytes(
        ApprovalActionKind::DomainMutation,
        ApprovalRiskClass::High,
        ApprovalOrigin::Model,
        "Start the selected instance".to_string(),
        b"opaque canonical approval action",
    )
    .unwrap();
    let request: ApprovalRequest = serde_json::from_value(serde_json::json!({
        "requestId": 9,
        "operationId": "opaque-operation",
        "action": action,
        "requestedAtMs": 10,
        "expiresAtMs": 20,
        "offersAllowForSession": true,
    }))
    .unwrap();
    let activity = tracker
        .observe(
            AgentEventSequence::new(1),
            &AgentEventKind::ApprovalRequested {
                request: request.clone(),
            },
            10,
        )
        .unwrap();
    assert_eq!(
        activity.presentation.subject.as_deref(),
        Some("Start the selected instance")
    );
    tracker.observe(
        AgentEventSequence::new(2),
        &AgentEventKind::ApprovalResolved {
            request_id: request.request_id,
            resolution: serde_json::from_value::<ApprovalResolution>(serde_json::json!({
                "requestId": 9,
                "decision": ApprovalDecision::AllowOnce,
                "source": ApprovalDecisionSource::User,
                "resolvedAtMs": 15,
            }))
            .unwrap(),
        },
        15,
    );
    assert_eq!(
        tracker.recent_snapshots()[0].phase,
        AgentActivityPhase::Succeeded
    );
}

#[test]
fn execution_outlives_turn_and_waits_for_confirmed_disposal() {
    let mut tracker = AgentActivityTracker::default();
    tracker.observe(
        AgentEventSequence::new(1),
        &AgentEventKind::ExecutionProgress {
            call_id: None,
            update: AgentToolProgressUpdate::ExecutionStarted {
                execution_id: 7,
                generation: 2,
                action: AgentActivityAction::fixed("exec_command"),
                status: AgentExecutionProgressStatus::Running,
            },
        },
        10,
    );
    tracker.observe(
        AgentEventSequence::new(2),
        &AgentEventKind::TurnCompleted {
            final_answer_message_id: Some(crate::AgentMessageId::new(1)),
        },
        20,
    );
    assert_eq!(tracker.active_snapshots().len(), 1);

    tracker.observe(
        AgentEventSequence::new(3),
        &AgentEventKind::ExecutionProgress {
            call_id: None,
            update: AgentToolProgressUpdate::ExecutionStatus {
                execution_id: 7,
                generation: 2,
                status: AgentExecutionProgressStatus::Exited,
                exit_code: Some(0),
                cleanup_required: true,
            },
        },
        30,
    );
    assert_eq!(
        tracker.active_snapshots()[0].phase,
        AgentActivityPhase::WaitingForCleanup
    );

    tracker.observe(
        AgentEventSequence::new(4),
        &AgentEventKind::ExecutionProgress {
            call_id: None,
            update: AgentToolProgressUpdate::ExecutionDisposed {
                execution_id: 7,
                generation: 2,
            },
        },
        40,
    );
    assert!(tracker.active_snapshots().is_empty());
    assert_eq!(
        tracker.recent_snapshots().last().unwrap().phase,
        AgentActivityPhase::Succeeded
    );
}

#[test]
fn execution_output_is_head_tail_bounded_and_late_updates_do_not_revive_it() {
    let mut tracker = AgentActivityTracker::default();
    tracker.observe(
        AgentEventSequence::new(1),
        &AgentEventKind::ExecutionProgress {
            call_id: None,
            update: AgentToolProgressUpdate::ExecutionStarted {
                execution_id: 8,
                generation: 1,
                action: AgentActivityAction::fixed("exec_command"),
                status: AgentExecutionProgressStatus::Running,
            },
        },
        10,
    );
    let text = (0..20)
        .map(|index| format!("line-{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    tracker.observe(
        AgentEventSequence::new(2),
        &AgentEventKind::ExecutionProgress {
            call_id: None,
            update: AgentToolProgressUpdate::ExecutionOutput {
                execution_id: 8,
                generation: 1,
                text,
                next_offset: 200,
                truncated: true,
            },
        },
        20,
    );
    let output = tracker.active_snapshots()[0].output.clone().unwrap();
    assert_eq!(output.head, vec!["line-0", "line-1", "line-2", "line-3"]);
    assert_eq!(output.tail.first().map(String::as_str), Some("line-12"));
    assert_eq!(output.tail.last().map(String::as_str), Some("line-19"));
    assert_eq!(output.omitted_lines, 8);
    assert!(output.truncated);

    tracker.observe(
        AgentEventSequence::new(3),
        &AgentEventKind::ExecutionProgress {
            call_id: None,
            update: AgentToolProgressUpdate::ExecutionDisposed {
                execution_id: 8,
                generation: 1,
            },
        },
        30,
    );
    tracker.observe(
        AgentEventSequence::new(4),
        &AgentEventKind::ExecutionProgress {
            call_id: None,
            update: AgentToolProgressUpdate::ExecutionOutput {
                execution_id: 8,
                generation: 1,
                text: "late".to_string(),
                next_offset: 204,
                truncated: false,
            },
        },
        40,
    );
    assert!(tracker.active_snapshots().is_empty());
    assert!(!tracker
        .recent_snapshots()
        .last()
        .unwrap()
        .output
        .as_ref()
        .unwrap()
        .tail
        .iter()
        .any(|line| line == "late"));
}

#[test]
fn execution_updates_are_isolated_by_generation_across_turns() {
    let mut tracker = AgentActivityTracker::default();
    for (sequence, generation) in [(1, 1), (2, 2)] {
        tracker.observe(
            AgentEventSequence::new(sequence),
            &AgentEventKind::ExecutionProgress {
                call_id: None,
                update: AgentToolProgressUpdate::ExecutionStarted {
                    execution_id: 8,
                    generation,
                    action: AgentActivityAction::fixed("exec_command"),
                    status: AgentExecutionProgressStatus::Running,
                },
            },
            10 + u128::from(sequence),
        );
    }
    tracker.observe(
        AgentEventSequence::new(3),
        &AgentEventKind::ExecutionProgress {
            call_id: None,
            update: AgentToolProgressUpdate::ExecutionStatus {
                execution_id: 8,
                generation: 1,
                status: AgentExecutionProgressStatus::Exited,
                exit_code: Some(0),
                cleanup_required: true,
            },
        },
        20,
    );

    let active = tracker.active_snapshots();
    assert_eq!(active.len(), 2);
    assert_eq!(active[0].phase, AgentActivityPhase::WaitingForCleanup);
    assert_eq!(active[0].execution.as_ref().unwrap().generation, 1);
    assert_eq!(active[1].phase, AgentActivityPhase::Running);
    assert_eq!(active[1].execution.as_ref().unwrap().generation, 2);
}

fn execution_update(
    tracker: &mut AgentActivityTracker,
    sequence: u64,
    update: AgentToolProgressUpdate,
) -> AgentActivityOutputPreview {
    tracker
        .observe(
            AgentEventSequence::new(sequence),
            &AgentEventKind::ExecutionProgress {
                call_id: None,
                update,
            },
            sequence as u128,
        )
        .and_then(|snapshot| snapshot.output)
        .unwrap()
}

fn execution_text(text: &str, next_offset: u64) -> AgentToolProgressUpdate {
    AgentToolProgressUpdate::ExecutionOutput {
        execution_id: 9,
        generation: 1,
        text: text.to_string(),
        next_offset,
        truncated: false,
    }
}

#[test]
fn execution_output_lines_are_assembled_across_updates_as_plain_text() {
    let mut tracker = AgentActivityTracker::default();
    execution_update(
        &mut tracker,
        1,
        AgentToolProgressUpdate::ExecutionStarted {
            execution_id: 9,
            generation: 1,
            action: AgentActivityAction::fixed("exec_command"),
            status: AgentExecutionProgressStatus::Running,
        },
    );

    // A line split between two reads stays one line, and the unfinished part
    // is shown until it is replaced.
    let output = execution_update(&mut tracker, 2, execution_text("\u{1b}[32mbuil", 10));
    assert_eq!(output.head, vec!["buil"]);
    let output = execution_update(&mut tracker, 3, execution_text("ding\u{1b}[0m\r\n", 22));
    assert_eq!(output.head, vec!["building"]);

    // A redrawn progress line keeps only what it last displayed, and an
    // escape sequence cut between two reads is not printed as text.
    let output = execution_update(&mut tracker, 4, execution_text("10%\r50%\u{1b}[", 33));
    assert_eq!(output.head, vec!["building", "50%"]);
    let output = execution_update(&mut tracker, 5, execution_text("K\r100%\n", 40));
    assert_eq!(output.head, vec!["building", "100%"]);
    assert_eq!(output.next_offset, 40);

    // The terminal status makes the unfinished line final.
    execution_update(&mut tracker, 6, execution_text("done", 44));
    let output = execution_update(
        &mut tracker,
        7,
        AgentToolProgressUpdate::ExecutionStatus {
            execution_id: 9,
            generation: 1,
            status: AgentExecutionProgressStatus::Exited,
            exit_code: Some(0),
            cleanup_required: false,
        },
    );
    assert_eq!(output.head, vec!["building", "100%", "done"]);
}

#[test]
fn an_unfinished_line_never_grows_the_bounded_tail() {
    let mut tracker = AgentActivityTracker::default();
    execution_update(
        &mut tracker,
        1,
        AgentToolProgressUpdate::ExecutionStarted {
            execution_id: 9,
            generation: 1,
            action: AgentActivityAction::fixed("exec_command"),
            status: AgentExecutionProgressStatus::Running,
        },
    );
    let lines = (0..12)
        .map(|index| format!("line-{index}\n"))
        .collect::<String>();
    execution_update(&mut tracker, 2, execution_text(&lines, 84));
    let output = execution_update(&mut tracker, 3, execution_text("partial", 91));
    assert_eq!(output.tail.len(), 8);
    assert_eq!(output.tail.first().map(String::as_str), Some("line-5"));
    assert_eq!(output.tail.last().map(String::as_str), Some("partial"));
    assert_eq!(output.omitted_lines, 1);

    // Replacing the unfinished line restores the line it displaced.
    let output = execution_update(&mut tracker, 4, execution_text(" more", 96));
    assert_eq!(output.tail.first().map(String::as_str), Some("line-5"));
    assert_eq!(output.tail.last().map(String::as_str), Some("partial more"));
    assert_eq!(output.omitted_lines, 1);
    let output = execution_update(&mut tracker, 5, execution_text("\n", 97));
    assert_eq!(output.tail.first().map(String::as_str), Some("line-5"));
    assert_eq!(output.omitted_lines, 1);
}
