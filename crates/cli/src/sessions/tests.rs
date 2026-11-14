use hanafubuki_agent::{AgentSubagentWake, AgentTerminalWake, AgentWakeEvent, AgentWakeSource};

use super::*;

/// `sessions show` prints what background work reported, one line per
/// event.
#[test]
fn wake_events_are_shown_one_line_per_event() {
    let event = |sequence, source| AgentWakeEvent {
        sequence,
        queued_at: 0,
        source,
    };
    let item = ConversationItem::WakeEvents {
        events: vec![
            event(
                1,
                AgentWakeSource::Subagent {
                    session_id: AgentSessionId::new(41),
                    name: "explore_api".to_string(),
                    event: AgentSubagentWake::Completed {
                        answer: Some("Three endpoints.\nThe rest follows.".to_string()),
                        truncated: false,
                    },
                },
            ),
            event(
                2,
                AgentWakeSource::Terminal {
                    execution_id: 12,
                    command: "make bench".to_string(),
                    event: AgentTerminalWake::Lost,
                },
            ),
        ],
    };
    assert_eq!(
        transcript_line(&item).unwrap(),
        "background: Sub-agent `explore_api` (agent id 41) completed: Three endpoints.\n\
         background: Terminal session 12 (`make bench`) stopped when the Agent restarted"
    );
    assert_eq!(
        transcript_line(&ConversationItem::WakeEvents { events: Vec::new() }),
        None
    );
}
