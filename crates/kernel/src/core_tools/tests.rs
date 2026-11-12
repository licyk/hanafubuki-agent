use serde_json::{json, Value};

use super::{AgentCoreTool, CoreToolCall, UpdateTaskPlanTool};
use crate::AgentSessionMode;

fn parse(tool: AgentCoreTool, arguments: Value) -> Result<CoreToolCall, String> {
    CoreToolCall::parse(tool, &arguments).map_err(|error| error.to_string())
}

#[test]
fn every_core_tool_uses_codex_style_non_strict_schema() {
    for tool in AgentCoreTool::ALL {
        let spec = tool.model_spec();
        assert!(!spec.strict);
        assert_eq!(spec.name, tool.name());
    }
}

#[test]
fn plan_mode_keeps_only_the_question_to_the_user_and_the_context_tools() {
    for tool in AgentCoreTool::ALL {
        assert!(tool.allowed_in_mode(AgentSessionMode::Default));
        assert_eq!(
            tool.allowed_in_mode(AgentSessionMode::Plan),
            tool == AgentCoreTool::RequestUserInput || tool.is_compression(),
            "{}",
            tool.name()
        );
    }
}

#[test]
fn task_plan_and_user_input_tools_have_typed_bounded_contracts() {
    let task_plan = parse(
        AgentCoreTool::UpdatePlan,
        json!({
            "explanation": "Inspect first",
            "plan": [
                { "step": "Inspect the current state", "status": "in_progress" },
                { "step": "Implement the change", "status": "pending" }
            ]
        }),
    )
    .expect("bounded task checklist should parse");
    assert!(matches!(task_plan, CoreToolCall::UpdatePlan(_)));
    for plan in [
        json!([{ "step": "Not started", "status": "pending" }]),
        json!([
            { "step": "Inspected", "status": "completed" },
            { "step": "Validated", "status": "completed" }
        ]),
    ] {
        assert!(parse(
            AgentCoreTool::UpdatePlan,
            json!({ "explanation": null, "plan": plan }),
        )
        .is_ok());
    }

    let question = parse(
        AgentCoreTool::RequestUserInput,
        json!({
            "questions": [{
                "id": "scope",
                "header": "Scope",
                "question": "Which scope should the plan use?",
                "options": [
                    { "id": "focused", "label": "Focused", "description": "Change only this feature." },
                    { "id": "broad", "label": "Broad", "description": "Include adjacent cleanup." }
                ]
            }]
        }),
    )
    .expect("bounded question should parse");
    assert!(matches!(question, CoreToolCall::RequestUserInput(_)));

    assert!(parse(
        AgentCoreTool::RequestUserInput,
        json!({ "questions": [], "confirmed": true }),
    )
    .is_err());
    assert!(parse(
        AgentCoreTool::UpdatePlan,
        json!({
            "explanation": null,
            "plan": [
                { "step": "A", "status": "in_progress" },
                { "step": "B", "status": "in_progress" }
            ]
        }),
    )
    .is_err());
    // `explanation` is optional and an unknown field is ignored.
    assert!(matches!(
        parse(
            AgentCoreTool::UpdatePlan,
            json!({
                "plan": [{ "step": "No explanation", "status": "pending", "extra": true }]
            }),
        ),
        Ok(CoreToolCall::UpdatePlan(UpdateTaskPlanTool {
            explanation: None,
            ..
        }))
    ));
    // An error names the field it is about.
    assert_eq!(
        parse(
            AgentCoreTool::UpdatePlan,
            json!({ "plan": [{ "step": "A", "status": "pending" }, { "step": "B", "status": "doing" }] }),
        )
        .unwrap_err()
        .to_string(),
        "invalid tool argument plan[1].status: unknown variant `doing`, expected one of `pending`, `in_progress`, `completed`"
    );
    for arguments in [
        json!({
            "explanation": null,
            "plan": [{ "step": "   ", "status": "pending" }]
        }),
        json!({
            "explanation": "x".repeat(2049),
            "plan": [{ "step": "Bounded", "status": "pending" }]
        }),
        json!({
            "explanation": null,
            "plan": [{ "step": "x".repeat(1025), "status": "pending" }]
        }),
        json!({
            "explanation": null,
            "plan": (0..21).map(|index| json!({
                "step": format!("Step {index}"),
                "status": "pending"
            })).collect::<Vec<_>>()
        }),
        json!({
            "explanation": null,
            "items": [{ "id": "proposal", "text": "Wrong contract", "status": "pending" }]
        }),
    ] {
        assert!(parse(AgentCoreTool::UpdatePlan, arguments).is_err());
    }
}

#[test]
fn new_context_takes_no_arguments() {
    assert_eq!(
        parse(AgentCoreTool::NewContext, json!({})),
        Ok(CoreToolCall::NewContext)
    );
    // An unknown key is ignored.
    assert_eq!(
        parse(AgentCoreTool::NewContext, json!({ "reason": "full" })),
        Ok(CoreToolCall::NewContext)
    );
    assert!(parse(AgentCoreTool::NewContext, json!([])).is_err());
}

#[test]
fn a_compress_call_names_its_ranges_and_shows_its_receipt_then_each_summary() {
    use hanafubuki_agent_contracts::{AgentToolDisplay, AgentToolPresenter};
    use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};

    let arguments = json!({ "content": [
        { "startId": "m00002", "endId": "m00004", "summary": "The setup ran.", "topic": "Setup" },
        { "startId": "m00006", "endId": "m00007", "summary": "Read the logs.\n" }
    ] });
    let receipt = ToolOutput {
        content: vec![ToolOutputContent::Text {
            text: "[Compressed m00002–m00004 → 1 block(s).]\n".to_string(),
        }],
        structured_content: None,
        is_error: false,
    };

    let presenter = super::CoreToolPresenter;
    assert_eq!(
        presenter.subject("compress", &arguments).as_deref(),
        Some("m00002–m00004 Setup, m00006–m00007")
    );
    assert_eq!(
        presenter.display("compress", &arguments, &receipt),
        Some(AgentToolDisplay::Text {
            text: "[Compressed m00002–m00004 → 1 block(s).]\nm00002–m00004 Setup\nThe setup ran.\nm00006–m00007\nRead the logs.".to_string()
        })
    );
    assert_eq!(presenter.subject("compress", &json!({})), None);
}
