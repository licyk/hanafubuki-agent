use super::*;
use serde_json::json;

#[test]
fn a_recorded_outcome_shows_adoption_and_validation() {
    let args = json!({"adviceId":"advice-1","adoption":"followed","validationStatus":"passed"});
    let output = ToolOutput::structured(json!({"recorded":true}), true);
    assert!(AdvisorPresenter
        .display("record_advisor_outcome", &args, &output)
        .is_none());
    let display = AdvisorPresenter.display(
        "record_advisor_outcome",
        &args,
        &ToolOutput {
            is_error: false,
            ..output
        },
    );
    assert!(
        matches!(display, Some(AgentToolDisplay::Text { text }) if text == "Adoption: followed\nValidation: passed")
    );
}
