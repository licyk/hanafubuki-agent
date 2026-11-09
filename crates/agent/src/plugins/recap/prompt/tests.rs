use super::*;

#[test]
fn a_recap_is_read_tolerantly() {
    assert_eq!(
        parse_recap(r#"{"summary":"  Fixed the login.  ","next_action":" Deploy it? ","mood":1}"#)
            .unwrap(),
        AgentRecap {
            summary: "Fixed the login.".to_string(),
            next_action: Some("Deploy it?".to_string()),
        }
    );
    for encoded in [
        r#"{"summary":"Done"}"#,
        r#"{"summary":"Done","next_action":null}"#,
        r#"{"summary":"Done","next_action":"  "}"#,
    ] {
        assert_eq!(parse_recap(encoded).unwrap().next_action, None, "{encoded}");
    }
    let long = parse_recap(&format!(
        r#"{{"summary":"{}","next_action":"{}"}}"#,
        "界".repeat(RECAP_SUMMARY_MAX_CHARS + 5),
        "x".repeat(RECAP_NEXT_ACTION_MAX_CHARS + 5)
    ))
    .unwrap();
    assert_eq!(long.summary.chars().count(), RECAP_SUMMARY_MAX_CHARS);
    assert_eq!(
        long.next_action.unwrap().chars().count(),
        RECAP_NEXT_ACTION_MAX_CHARS
    );
}

#[test]
fn an_empty_or_malformed_answer_is_no_recap() {
    assert!(parse_recap(r#"{"summary":"   ","next_action":null}"#).is_err());
    assert!(parse_recap(r#"{"next_action":"Ask"}"#).is_err());
    assert!(parse_recap("Summary: done").is_err());
}

/// The whole prompt, instructions included, stays within the recap's
/// token budget.
#[test]
fn the_request_fits_the_prompt_budget() {
    let history = "问".repeat(history_max_bytes());
    let request = recap_request("model".to_string(), &history);
    assert!(request.input.starts_with("Conversation:\n"));
    assert!(
        request.instructions.len() + request.input.len()
            <= token_budget_bytes(RECAP_PROMPT_MAX_TOKENS)
    );
}
