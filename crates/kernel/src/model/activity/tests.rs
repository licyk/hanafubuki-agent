use hanafubuki_agent_contracts::AgentActivityAction;

use super::*;

fn snapshot_json(action: &str) -> String {
    format!(
        r#"{{
            "id": 7,
            "revision": 2,
            "kind": "tool_invocation",
            "phase": "succeeded",
            "presentation": {{ "action": "{action}", "subject": "notes.txt" }},
            "attempt": null,
            "startedAt": 10,
            "updatedAt": 20,
            "finishedAt": 30,
            "execution": null,
            "output": null
        }}"#
    )
}

/// A retained session outlives the tool catalog. Actions are open names a
/// tool chooses, so a session that used a removed tool, or one recorded by a
/// later build, keeps its action name; Vue labels names it does not know as a
/// generic tool.
#[test]
fn an_action_name_of_a_removed_or_later_tool_loads_as_itself() {
    for name in ["write_file", "some_tool_added_later"] {
        let snapshot: AgentActivitySnapshot =
            serde_json::from_str(&snapshot_json(name)).expect("an unknown tool still loads");

        assert_eq!(snapshot.presentation.action.as_str(), name);
        assert_eq!(snapshot.presentation.subject.as_deref(), Some("notes.txt"));
        assert_eq!(snapshot.kind, AgentActivityKind::ToolInvocation);
        assert_eq!(snapshot.phase, AgentActivityPhase::Succeeded);
        assert_eq!(snapshot.finished_at, Some(30));
    }
}

/// The label is presentation only, so a value that is not an action name
/// degrades the display of past work instead of making the conversation
/// unloadable.
#[test]
fn a_value_that_is_not_an_action_name_loads_as_unknown() {
    for value in ["Write File", "", "x".repeat(65).as_str()] {
        let snapshot: AgentActivitySnapshot =
            serde_json::from_str(&snapshot_json(value)).expect("still loads");
        assert_eq!(snapshot.presentation.action, AgentActivityAction::UNKNOWN);
    }
}

#[test]
fn known_action_names_are_unaffected() {
    for (name, expected) in [
        ("apply_patch", AgentActivityAction::fixed("apply_patch")),
        ("exec_command", AgentActivityAction::fixed("exec_command")),
        (
            "generating_response",
            AgentActivityAction::GENERATING_RESPONSE,
        ),
        ("list_tasks", AgentActivityAction::fixed("list_tasks")),
        ("read_task_log", AgentActivityAction::fixed("read_task_log")),
    ] {
        let snapshot: AgentActivitySnapshot =
            serde_json::from_str(&snapshot_json(name)).expect("known action");
        assert_eq!(snapshot.presentation.action, expected);
    }
}

/// The fallback is reachable by name too, so a session written by this build
/// and read back by it round-trips rather than failing on its own output.
#[test]
fn the_unknown_label_round_trips() {
    let encoded = serde_json::to_string(&AgentActivityAction::UNKNOWN).unwrap();
    assert_eq!(encoded, r#""unknown""#);
    assert_eq!(
        serde_json::from_str::<AgentActivityAction>(&encoded).unwrap(),
        AgentActivityAction::UNKNOWN
    );
}

/// Lifecycle enums are deliberately *not* tolerant: they describe this state
/// machine rather than the tool catalog, so an unreadable value is a real
/// inconsistency and silently defaulting one could strand an activity as
/// non-terminal.
#[test]
fn lifecycle_values_stay_strict() {
    assert!(serde_json::from_str::<AgentActivityPhase>(r#""not_a_phase""#).is_err());
    assert!(serde_json::from_str::<AgentActivityKind>(r#""not_a_kind""#).is_err());
}
