use super::*;
use serde_json::json;

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
