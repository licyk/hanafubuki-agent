// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use super::*;

fn filtered(text: &str) -> Vec<&'static str> {
    filtered_without(text, Vec::new())
}

fn filtered_without(text: &str, disabled: Vec<&'static str>) -> Vec<&'static str> {
    let mut popup = CommandPopup::new(disabled);
    popup.on_composer_text_change(text.to_string());
    popup
        .filtered_items()
        .into_iter()
        .map(|matched| matched.name)
        .collect()
}

#[test]
fn filter_includes_compact_when_typing_prefix() {
    assert!(
        filtered("/co").contains(&"compact"),
        "expected '/compact' to appear among filtered commands"
    );
}

#[test]
fn selecting_compact_by_exact_match() {
    let mut popup = CommandPopup::new(Vec::new());
    popup.on_composer_text_change("/compact".to_string());

    // When an exact match exists, the selected command should be that
    // command by default.
    assert_eq!(
        popup.selected_item().map(|matched| matched.command.name),
        Some("compact")
    );
}

#[test]
fn model_is_first_suggestion_for_mo() {
    assert_eq!(filtered("/mo").first(), Some(&"model"));
}

#[test]
fn filtered_commands_keep_presentation_order_for_prefix() {
    assert_eq!(filtered("/m"), vec!["model", "mode", "mention", "mcp"]);
}

#[test]
fn prefix_filter_limits_matches_for_ac() {
    let cmds = filtered("/ac");
    assert!(
        !cmds.contains(&"compact"),
        "expected prefix search for '/ac' to exclude 'compact', got {cmds:?}"
    );
}

#[test]
fn quit_hidden_in_empty_filter_but_shown_for_prefix() {
    assert!(!filtered("/").contains(&"quit"));
    assert!(filtered("/qu").contains(&"quit"));
}

#[test]
fn the_unfiltered_list_is_the_registry_in_order() {
    assert_eq!(
        filtered("/"),
        vec![
            "model", "mode", "skills", "new", "resume", "compact", "diff", "mention", "status",
            "mcp", "exit"
        ]
    );
}

#[test]
fn a_command_turned_off_is_left_out() {
    assert_eq!(
        filtered_without("/m", vec!["mode"]),
        vec!["model", "mention", "mcp"]
    );
    assert!(!filtered_without("/", vec!["mode"]).contains(&"mode"));
    assert!(filtered_without("/qu", vec!["exit"]).is_empty());
}
