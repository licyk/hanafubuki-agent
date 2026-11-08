use super::*;

#[test]
fn generation_prompt_truncation_preserves_utf8_boundaries() {
    let prompt = format!("{}tail", "问".repeat(TITLE_PROMPT_MAX_BYTES));
    let bounded = bounded_prompt(&prompt);
    assert!(bounded.len() <= TITLE_PROMPT_MAX_BYTES);
    assert!(bounded.is_char_boundary(bounded.len()));
}

#[test]
fn generated_title_enforces_the_automatic_bound() {
    assert_eq!(
        normalize_generated_title("  Rename   conversations  ".to_string()).unwrap(),
        "Rename conversations"
    );
    assert!(normalize_generated_title(String::new()).is_err());
    assert!(normalize_generated_title("x".repeat(AUTOMATIC_AGENT_TITLE_MAX_CHARS + 1)).is_err());
}
