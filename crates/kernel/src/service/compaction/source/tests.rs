use super::*;
use serde_json::json;

#[test]
fn compaction_groups_parallel_tool_results_and_keeps_turn_trim_boundaries() {
    let call = |id| {
        json!({
            "type": "tool_call", "callId": id, "name": "skills_read",
            "catalogRevision": vec![0; 32], "providerId": "builtin",
            "toolId": "skills_read", "toolGeneration": vec![0; 32], "arguments": {}
        })
    };
    let output = |id| {
        json!({
            "type": "tool_output", "callId": id,
            "output": {"content": [{"type": "text", "text": "done"}], "isError": false}
        })
    };
    let source = vec![
        json!({"type": "turn_start", "turnId": 1}),
        call("skill"),
        call("plan"),
        output("skill"),
        json!({"type": "skill_context", "skill": "user:workflow", "name": "Workflow",
            "content": "<skill>\nworkflow instructions\n</skill>"}),
        output("plan"),
        json!({"type": "turn_start", "turnId": 2}),
        call("interrupted"),
        json!({"type": "turn_start", "turnId": 3}),
        json!({"type": "user_message", "text": "continue"}),
    ]
    .into_iter()
    .map(|value| serde_json::from_value(value).unwrap())
    .collect::<Vec<_>>();
    let (input, ends) = build_compaction_input(
        &source,
        &LoadedImages::default(),
        hanafubuki_agent_model_api::ModelApiProtocol::Responses,
    )
    .unwrap();
    assert_eq!(ends, [5, 7]);
    assert!(matches!(&input[2], ModelInputItem::ToolOutput { call_id, .. } if call_id == "skill"));
    assert!(matches!(&input[3], ModelInputItem::ToolOutput { call_id, .. } if call_id == "plan"));
    assert!(
        matches!(&input[4], ModelInputItem::ContextFragment { content, .. } if content.contains("workflow instructions"))
    );
    assert!(
        matches!(&input[6], ModelInputItem::ToolOutput { call_id, output } if call_id == "interrupted" && output.is_error)
    );
}
