use super::*;

#[test]
fn the_excerpt_reads_oldest_first_with_labels() {
    let excerpt = conversation_excerpt(vec![
        ConversationMessage::Assistant(" Done. ".to_string()),
        ConversationMessage::User("Fix the login timeout".to_string()),
    ]);
    assert_eq!(excerpt, "User: Fix the login timeout\n\nAssistant: Done.");
}

#[test]
fn the_excerpt_fits_the_title_prompt_on_character_boundaries() {
    let messages = (0..SUGGESTION_MESSAGES)
        .map(|index| {
            let text = "问".repeat(TITLE_PROMPT_MAX_BYTES);
            if index % 2 == 0 {
                ConversationMessage::User(text)
            } else {
                ConversationMessage::Assistant(text)
            }
        })
        .collect();
    let excerpt = conversation_excerpt(messages);
    assert!(excerpt.len() <= TITLE_PROMPT_MAX_BYTES, "{}", excerpt.len());
    assert_eq!(excerpt.matches("User: ").count(), SUGGESTION_MESSAGES / 2);
    assert_eq!(
        excerpt.matches("Assistant: ").count(),
        SUGGESTION_MESSAGES / 2
    );
}
