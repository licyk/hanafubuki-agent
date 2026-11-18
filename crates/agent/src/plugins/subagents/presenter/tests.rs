use super::*;
use serde_json::json;

fn text(tool: &str, arguments: Value, value: Value) -> String {
    let output = ToolOutput::structured(value, false);
    let Some(AgentToolDisplay::Text { text }) =
        SubagentPresenter.display(tool, &arguments, &output)
    else {
        panic!("missing presentation")
    };
    text
}

#[test]
fn spawn_uses_the_resolved_model_and_preserves_the_full_task() {
    let arguments = json!({"task_name": "reviewer", "message": "Review the parser.\nCheck Unicode and report every finding.", "model": "requested-model", "reasoning_effort": "low", "agent_type": "review", "fork_turns": "none"});
    let output = json!({"task_name": "reviewer", "agent_id": "42", "provider": "provider-2", "model": "resolved-model", "reasoning_effort": "high", "future": true});
    let shown = text("spawn_agent", arguments, output);
    assert_eq!(shown, "Agent: reviewer (agent 42)\nModel: resolved-model · Reasoning: high\nTask: Review the parser.\nCheck Unicode and report every finding.\nProvider: provider-2\nRole: review\nConversation context: none");
    let old = text(
        "spawn_agent",
        json!({"message": "Old task"}),
        json!({"task_name": "old", "agent_id": "1"}),
    );
    assert!(old.contains("Model: not recorded · Reasoning: unspecified"));
}

#[test]
fn interactions_show_delivery_task_messages_and_previous_status() {
    for delivery in [
        "delivered to its running turn",
        "queued for its next turn",
        "started a new turn",
    ] {
        for (tool, label) in [("send_message", "Message"), ("followup_task", "Task")] {
            let shown = text(
                tool,
                json!({"target": "reviewer", "message": "First line\nFull follow-up"}),
                json!({"delivery": delivery}),
            );
            assert_eq!(
                shown,
                format!("Delivery: {delivery}\n{label}: First line\nFull follow-up")
            );
        }
    }
    assert_eq!(
        text(
            "interrupt_agent",
            json!({"target": "reviewer"}),
            json!({"previous_status": "completed"})
        ),
        "Previous status: Completed"
    );
}

#[test]
fn lists_show_model_details_and_empty_waits_keep_their_outcome() {
    let shown = text(
        "list_agents",
        json!({}),
        json!({"agents": [
            {"agent_name": "reviewer", "agent_id": "42", "agent_status": "running", "model": "fast-model", "reasoning_effort": "medium", "provider": "local", "agent_type": "review"},
            null,
            {"agent_name": "tester", "agent_id": "43", "agent_status": "waiting", "model": "test-model", "reasoning_effort": null}
        ]}),
    );
    assert!(shown.contains("reviewer (agent 42) · Running\nModel: fast-model · Reasoning: medium\nProvider: local\nRole: review"));
    assert!(
        shown.contains("tester (agent 43) · Waiting\nModel: test-model · Reasoning: unspecified")
    );
    assert_eq!(
        text("list_agents", json!({}), json!({"agents": []})),
        "No subagents."
    );
    for message in ["Wait timed out.", "None of your agents is still working."] {
        assert_eq!(
            text(
                "wait_agent",
                json!({}),
                json!({"message": message, "agents": []})
            ),
            message
        );
    }
}

#[test]
fn errors_use_the_failure_display_and_text_only_history_remains_readable() {
    for tool in SubagentPresenter.tools() {
        assert!(SubagentPresenter
            .display(
                tool,
                &json!({}),
                &ToolOutput::structured(json!({"error": "unavailable"}), true)
            )
            .is_none());
    }
    let output = ToolOutput {
        content: vec![ToolOutputContent::Text {
            text: r#"{"delivery":"queued for its next turn"}"#.into(),
        }],
        structured_content: None,
        is_error: false,
    };
    assert!(
        matches!(SubagentPresenter.display("send_message", &json!({"message": "Check the logs"}), &output), Some(AgentToolDisplay::Text { text }) if text.contains("Check the logs"))
    );
}

#[test]
fn malformed_statuses_warn_without_hiding_other_agents_or_unknown_fields() {
    let output = ToolOutput::json(
        json!({"future": true, "agents": [
            {"agent_name": "first", "agent_status": {"completed": "Answer one"}},
            {"agent_name": "broken", "agent_status": 7},
            {"agent_name": "missing"},
            {"agent_name": "last", "agent_status": {"completed": "Answer two"}, "extra": "ignored"}
        ]}),
        false,
    )
    .unwrap();
    let Some(AgentToolDisplay::Text { text }) =
        SubagentPresenter.display("wait_agent", &json!({}), &output)
    else {
        panic!("collaboration display");
    };
    assert!(text.contains("first · Completed\nAnswer one"));
    assert!(text.contains("broken · Warning"));
    assert!(text.contains("missing · Warning"));
    assert!(text.contains("last · Completed\nAnswer two"));
}
