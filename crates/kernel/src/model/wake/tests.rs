use serde_json::json;

use super::*;
use crate::{AgentHistoryItem, AgentHistoryItemId, AgentHistoryMutation, ConversationItem};

fn subagent(id: u64, name: &str, event: AgentSubagentWake) -> AgentWakeSource {
    AgentWakeSource::Subagent {
        session_id: AgentSessionId::new(id),
        name: name.to_string(),
        event,
    }
}

fn terminal(id: u64, command: &str, event: AgentTerminalWake) -> AgentWakeSource {
    AgentWakeSource::Terminal {
        execution_id: id,
        command: command.to_string(),
        event,
    }
}

fn ended(
    status: AgentExecutionProgressStatus,
    exit_code: Option<i32>,
    output: &str,
) -> AgentTerminalWake {
    AgentTerminalWake::Ended {
        status,
        exit_code,
        duration_millis: 252_400,
        output: output.to_string(),
        truncated: false,
    }
}

fn queued(sources: impl IntoIterator<Item = AgentWakeSource>) -> AgentWakeQueue {
    let mut queue = AgentWakeQueue::default();
    for source in sources {
        queue.push(source, 7);
    }
    queue
}

/// The plan's text (§3.2), block for block: every kind of event.
#[test]
fn renders_each_kind_of_event() {
    let queue = queued([
        subagent(
            41,
            "explore_api",
            AgentSubagentWake::Completed {
                answer: Some("The API has three endpoints.".to_string()),
                truncated: false,
            },
        ),
        subagent(
            42,
            "fix_tests",
            AgentSubagentWake::Errored {
                error: "model remote error status=Some(500)".to_string(),
            },
        ),
        subagent(43, "docs", AgentSubagentWake::ClosedByUser),
        subagent(44, "lint", AgentSubagentWake::Lost),
        subagent(
            41,
            "explore_api",
            AgentSubagentWake::Message {
                text: "Found the schema.".to_string(),
            },
        ),
        terminal(
            12,
            "cargo test --workspace",
            ended(
                AgentExecutionProgressStatus::Exited,
                Some(101),
                "test result: FAILED",
            ),
        ),
        terminal(
            13,
            "npm run dev",
            ended(AgentExecutionProgressStatus::TimedOut, None, ""),
        ),
        terminal(
            14,
            "python -m http.server",
            AgentTerminalWake::StoppedByUser { exit_code: None },
        ),
        terminal(15, "make bench", AgentTerminalWake::Lost),
        terminal(
            16,
            "./missing",
            ended(AgentExecutionProgressStatus::Failed, None, ""),
        ),
    ]);
    assert_eq!(
        wake_events_record(&queue.events),
        "<background_events>
Updates on work you started in the background: your sub-agents and your background terminals. The user did not write this.

Sub-agent `explore_api` (agent id 41) completed. Its final answer:
<answer>
The API has three endpoints.
</answer>

Sub-agent `fix_tests` (agent id 42) failed: model remote error status=Some(500)

Sub-agent `docs` (agent id 43) was closed by the user: its turn, its terminals and its own sub-agents were stopped. Do not start it again unless the user asks.

Sub-agent `lint` (agent id 44) stopped when the Agent restarted. It keeps its conversation and takes follow-up tasks.

Message from sub-agent `explore_api` (agent id 41):
<message>
Found the schema.
</message>

Terminal session 12 exited with code 101 after 252 seconds:
<command>
cargo test --workspace
</command>
Its last output (poll session 12 for the rest):
<output>
test result: FAILED
</output>

Terminal session 13 timed out after 252 seconds and was stopped:
<command>
npm run dev
</command>

Terminal session 14 was stopped by the user:
<command>
python -m http.server
</command>

Terminal session 15 stopped when the Agent restarted:
<command>
make bench
</command>

Terminal session 16 failed to run after 252 seconds:
<command>
./missing
</command>
</background_events>"
    );
    assert_eq!(
        queue.events[0].source.summary(),
        "Sub-agent `explore_api` (agent id 41) completed: The API has three endpoints."
    );
    assert_eq!(
        queue.events[5].source.summary(),
        "Terminal session 12 (`cargo test --workspace`) exited with code 101 after 252 seconds: test result: FAILED"
    );
    assert_eq!(
        queue.events[7].source.summary(),
        "Terminal session 14 (`python -m http.server`) was stopped by the user"
    );
}

#[test]
fn an_answer_a_command_and_an_output_are_bounded() {
    let answer = format!("{}é", "a".repeat(MAXIMUM_WAKE_ANSWER_BYTES - 1));
    let output = format!("{}\nkept line\n{}", "x".repeat(200), "y".repeat(4_000));
    let queue = queued([
        subagent(
            41,
            "explore_api",
            AgentSubagentWake::Completed {
                answer: Some(answer),
                truncated: false,
            },
        ),
        terminal(
            12,
            &"c".repeat(MAXIMUM_WAKE_COMMAND_BYTES + 10),
            ended(AgentExecutionProgressStatus::Exited, Some(0), &output),
        ),
    ]);
    let AgentWakeSource::Subagent {
        event: AgentSubagentWake::Completed { answer, truncated },
        ..
    } = &queue.events[0].source
    else {
        panic!("{queue:?}");
    };
    // Cut at a character boundary: the two bytes of `é` do not fit.
    assert_eq!(
        answer.as_deref().unwrap().len(),
        MAXIMUM_WAKE_ANSWER_BYTES - 1
    );
    assert!(*truncated);
    assert!(wake_events_record(&queue.events[..1])
        .contains("a… (cut; the rest is in the sub-agent's conversation)\n</answer>"));
    // A person's line takes the answer's first 200 characters.
    assert_eq!(
        queue.events[0].source.summary(),
        format!(
            "Sub-agent `explore_api` (agent id 41) completed: {}…",
            "a".repeat(200)
        )
    );
    let AgentWakeSource::Terminal {
        command,
        event: AgentTerminalWake::Ended {
            output, truncated, ..
        },
        ..
    } = &queue.events[1].source
    else {
        panic!("{queue:?}");
    };
    assert_eq!(command.len(), MAXIMUM_WAKE_COMMAND_BYTES);
    // The tail, from the line it starts on: the cut line is left out.
    assert_eq!(*output, format!("kept line\n{}", "y".repeat(4_000)));
    assert!(*truncated);
}

#[test]
fn a_final_event_replaces_the_one_queued_for_the_same_agent_or_terminal() {
    let completed = AgentSubagentWake::Completed {
        answer: Some("done".to_string()),
        truncated: false,
    };
    let message = AgentSubagentWake::Message {
        text: "progress".to_string(),
    };
    let mut queue = queued([
        subagent(41, "explore_api", message.clone()),
        subagent(
            41,
            "explore_api",
            AgentSubagentWake::Errored {
                error: "boom".to_string(),
            },
        ),
        terminal(
            12,
            "sleep 1",
            ended(AgentExecutionProgressStatus::Exited, Some(0), ""),
        ),
        subagent(42, "docs", completed.clone()),
        subagent(41, "explore_api", message.clone()),
    ]);
    queue.push(subagent(41, "explore_api", completed.clone()), 8);
    queue.push(
        terminal(
            12,
            "sleep 1",
            AgentTerminalWake::StoppedByUser { exit_code: None },
        ),
        9,
    );
    let sources = queue
        .events
        .iter()
        .map(|event| event.source.clone())
        .collect::<Vec<_>>();
    // Each final event keeps its place at the end; messages are never
    // replaced.
    assert_eq!(
        sources,
        vec![
            subagent(41, "explore_api", message.clone()),
            subagent(42, "docs", completed.clone()),
            subagent(41, "explore_api", message),
            subagent(41, "explore_api", completed),
            terminal(
                12,
                "sleep 1",
                AgentTerminalWake::StoppedByUser { exit_code: None }
            ),
        ]
    );
    let sequences = queue
        .events
        .iter()
        .map(|event| event.sequence)
        .collect::<Vec<_>>();
    assert_eq!(sequences, vec![1, 4, 5, 6, 7]);
    assert_eq!(queue.last_sequence, 7);
}

#[test]
fn messages_are_bounded_and_taking_filters_by_source() {
    let mut queue = AgentWakeQueue::default();
    for index in 0..MAXIMUM_QUEUED_WAKE_MESSAGES {
        assert!(!queue.messages_full());
        queue.push(
            subagent(
                41,
                "explore_api",
                AgentSubagentWake::Message {
                    text: index.to_string(),
                },
            ),
            1,
        );
    }
    assert!(queue.messages_full());
    queue.push(subagent(42, "docs", AgentSubagentWake::Lost), 2);
    queue.push(terminal(12, "sleep 1", AgentTerminalWake::Lost), 3);
    queue.push(terminal(13, "sleep 2", AgentTerminalWake::Lost), 4);

    let taken = queue.take(AgentWakeFilter::Terminal { execution_id: 12 });
    assert_eq!(taken.len(), 1);
    assert_eq!(
        taken[0].source,
        terminal(12, "sleep 1", AgentTerminalWake::Lost)
    );
    let taken = queue.take(AgentWakeFilter::SubagentStatuses);
    assert_eq!(taken.len(), 1);
    assert_eq!(
        taken[0].source,
        subagent(42, "docs", AgentSubagentWake::Lost)
    );
    assert_eq!(queue.len(), MAXIMUM_QUEUED_WAKE_MESSAGES + 1);
    assert!(queue.take(AgentWakeFilter::SubagentStatuses).is_empty());

    // One sub-agent's status, never its messages nor another's status.
    queue.push(subagent(42, "docs", AgentSubagentWake::Lost), 5);
    queue.push(subagent(41, "explore_api", AgentSubagentWake::Lost), 6);
    let taken = queue.take(AgentWakeFilter::SubagentStatus {
        session_id: AgentSessionId::new(41),
    });
    assert_eq!(
        taken
            .into_iter()
            .map(|event| event.source)
            .collect::<Vec<_>>(),
        vec![subagent(41, "explore_api", AgentSubagentWake::Lost)]
    );
    assert_eq!(queue.len(), MAXIMUM_QUEUED_WAKE_MESSAGES + 2);
}

/// An event that does not decode is dropped, never the rest: in the
/// persisted queue and in a committed item.
#[test]
fn an_event_that_does_not_decode_is_dropped_alone() {
    let good = json!({
        "sequence": 3,
        "queuedAt": 9,
        "source": {
            "type": "subagent",
            "sessionId": 41,
            "name": "explore_api",
            "event": { "type": "lost" },
            "laterField": true,
        },
    });
    let unknown_kind = json!({
        "sequence": 4,
        "queuedAt": 9,
        "source": { "type": "web_hook", "url": "https://example.test" },
    });
    let bad_field = json!({
        "sequence": "five",
        "queuedAt": 9,
        "source": { "type": "terminal", "executionId": 12, "command": "x", "event": { "type": "lost" } },
    });
    let queue: AgentWakeQueue = serde_json::from_value(json!({
        "events": [unknown_kind, good, bad_field],
        "lastSequence": 2,
    }))
    .unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(
        queue.events[0].source,
        subagent(41, "explore_api", AgentSubagentWake::Lost)
    );
    // The counter is never behind a queued event.
    assert_eq!(queue.last_sequence, 3);
    let lost: AgentWakeQueue = serde_json::from_value(json!("not a queue")).unwrap();
    assert_eq!(lost, AgentWakeQueue::default());
    // A counter that does not decode costs no event.
    let counter_lost: AgentWakeQueue = serde_json::from_value(json!({
        "events": [good],
        "lastSequence": "three",
    }))
    .unwrap();
    assert_eq!(counter_lost, queue);
    let no_events: ConversationItem =
        serde_json::from_value(json!({ "type": "wake_events" })).unwrap();
    assert_eq!(
        no_events,
        ConversationItem::WakeEvents { events: Vec::new() }
    );

    let item: ConversationItem = serde_json::from_value(json!({
        "type": "wake_events",
        "events": [good, unknown_kind],
    }))
    .unwrap();
    assert_eq!(
        item,
        ConversationItem::WakeEvents {
            events: queue.events.clone()
        }
    );
    let nothing_left: ConversationItem = serde_json::from_value(json!({
        "type": "wake_events",
        "events": [bad_field],
    }))
    .unwrap();
    assert_eq!(nothing_left.as_model_input(), None);
}

/// The item as the history store writes it, inside a mutation, reads back
/// the same; the queue a session never used is left out of its record.
#[test]
fn the_item_and_the_queue_round_trip() {
    let queue = queued([
        subagent(
            41,
            "explore_api",
            AgentSubagentWake::Completed {
                answer: Some("done".to_string()),
                truncated: false,
            },
        ),
        terminal(
            12,
            "cargo test",
            ended(AgentExecutionProgressStatus::Exited, Some(0), "ok"),
        ),
    ]);
    let mutation = AgentHistoryMutation::AppendItem {
        item: AgentHistoryItem {
            source_session_id: AgentSessionId::new(1),
            item_id: AgentHistoryItemId::new(4),
            item_revision: 1,
            turn_id: None,
            logical_segment: 0,
            logical_order: 4,
            turn_working_millis: None,
            content: ConversationItem::WakeEvents {
                events: queue.events.clone(),
            },
        },
    };
    let encoded = serde_json::to_string(&mutation).unwrap();
    assert_eq!(
        serde_json::from_str::<AgentHistoryMutation>(&encoded).unwrap(),
        mutation
    );
    let encoded = serde_json::to_value(&queue).unwrap();
    assert_eq!(
        serde_json::from_value::<AgentWakeQueue>(encoded).unwrap(),
        queue
    );
    assert!(AgentWakeQueue::default().is_unused());
    assert!(!queue.is_unused());
}

#[test]
fn the_item_is_user_role_context() {
    let queue = queued([subagent(44, "lint", AgentSubagentWake::Lost)]);
    let item = ConversationItem::WakeEvents {
        events: queue.events.clone(),
    };
    assert_eq!(
        item.as_model_input(),
        Some(
            hanafubuki_agent_model_api::ModelInputItem::ContextFragment {
                role: hanafubuki_agent_model_api::ModelContextRole::User,
                kind: "background_events".to_string(),
                content: wake_events_record(&queue.events),
            }
        )
    );
}
