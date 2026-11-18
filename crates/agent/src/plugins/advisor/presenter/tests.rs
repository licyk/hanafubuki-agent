use super::*;
use serde_json::json;

#[test]
fn settings_proposals_and_outcomes_show_what_the_executor_did() {
    let proposal = json!({"pendingUserConfirmation":{"id":"proposal-1","changes":[{"key":"scout","before":false,"after":true}]}});
    for (tool, args, result, expected) in [
        (
            "advisor_config",
            json!({}),
            json!({"model":"reviewer", "reviewStages":["failure"]}),
            vec!["model: \"reviewer\"", "reviewStages: [\"failure\"]"],
        ),
        (
            "advisor_config",
            json!({"changes":{"scout":true}}),
            proposal,
            vec![
                "Awaiting user confirmation",
                "scout: false → true",
                "Proposal ID: proposal-1",
            ],
        ),
        (
            "record_advisor_outcome",
            json!({"adviceId":"advice-1","adoption":"followed","validationStatus":"passed"}),
            json!({"recorded":true}),
            vec!["Adoption: followed", "Validation: passed"],
        ),
    ] {
        let output = ToolOutput::structured(result, true);
        assert!(AdvisorPresenter.display(tool, &args, &output).is_none());
        let Some(AgentToolDisplay::Text { text }) = AdvisorPresenter.display(
            tool,
            &args,
            &ToolOutput {
                is_error: false,
                ..output
            },
        ) else {
            panic!("expected {tool} display")
        };
        for line in expected {
            assert!(text.contains(line), "{text}");
        }
    }
}
