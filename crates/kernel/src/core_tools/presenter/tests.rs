use super::*;
use serde_json::json;

#[test]
fn questions_pair_answers_by_identity_and_show_labels() {
    let arguments = json!({"questions": [
        {"id": "scope", "header": "Scope", "question": "修改哪些部分？", "options": [
            {"id": "focused", "label": "仅此功能", "description": "Keep scope small"}
        ]},
        {"id": "tests", "header": "Tests", "question": "运行测试？", "options": [
            {"id": "yes", "label": "运行", "description": "Run tests"}
        ]}
    ]});
    let output = hanafubuki_agent_contracts::ToolResult::success(json!({"answers": [
        {"questionId": "tests", "optionId": "yes", "future": true},
        {"questionId": "scope", "optionId": "focused"}
    ]}))
    .output;
    assert!(CoreToolPresenter.tools().contains(&"request_user_input"));
    assert_eq!(
        CoreToolPresenter
            .subject("request_user_input", &arguments)
            .as_deref(),
        Some("Scope, Tests")
    );
    assert_eq!(
        CoreToolPresenter.display("request_user_input", &arguments, &output),
        Some(AgentToolDisplay::Text {
            text: "修改哪些部分？ → 仅此功能\n运行测试？ → 运行".into()
        })
    );
    let mut failed = output;
    failed.is_error = true;
    assert_eq!(
        CoreToolPresenter.display("request_user_input", &arguments, &failed),
        None
    );
    let missing = hanafubuki_agent_contracts::ToolResult::success(json!({"answers": []})).output;
    assert_eq!(
        CoreToolPresenter.display("request_user_input", &arguments, &missing),
        None
    );
}
