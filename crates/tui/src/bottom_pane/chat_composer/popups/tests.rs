// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;

use super::*;
use crate::app_event::AppEventSender;
use crate::bottom_pane::paste_burst::PasteBurst;
use crate::render::renderable::Renderable;

fn composer() -> (ChatComposer, Receiver<AppEvent>) {
    let (tx, rx) = channel::<AppEvent>();
    let composer = ChatComposer::new(
        true,
        AppEventSender::new(tx),
        false,
        "Ask Codex to do anything".to_string(),
    );
    (composer, rx)
}

// Test helper: simulate human typing with a brief delay and flush the paste-burst buffer
fn type_chars_humanlike(composer: &mut ChatComposer, chars: &[char]) {
    for &ch in chars {
        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        std::thread::sleep(PasteBurst::recommended_flush_delay());
        let _ = composer.flush_paste_burst_if_due();
    }
}

fn enter(composer: &mut ChatComposer) -> InputResult {
    composer
        .handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .0
}

#[test]
fn test_current_at_token_basic_cases() {
    let test_cases = vec![
        // Valid @ tokens
        ("@hello", 3, Some("hello".to_string()), "Basic ASCII token"),
        (
            "@file.txt",
            4,
            Some("file.txt".to_string()),
            "ASCII with extension",
        ),
        (
            "hello @world test",
            8,
            Some("world".to_string()),
            "ASCII token in middle",
        ),
        (
            "@test123",
            5,
            Some("test123".to_string()),
            "ASCII with numbers",
        ),
        // Unicode examples
        ("@İstanbul", 3, Some("İstanbul".to_string()), "Turkish text"),
        (
            "@testЙЦУ.rs",
            8,
            Some("testЙЦУ.rs".to_string()),
            "Mixed ASCII and Cyrillic",
        ),
        ("@诶", 2, Some("诶".to_string()), "Chinese character"),
        ("@👍", 2, Some("👍".to_string()), "Emoji token"),
        // Invalid cases (should return None)
        ("hello", 2, None, "No @ symbol"),
        (
            "@",
            1,
            Some("".to_string()),
            "Only @ symbol triggers empty query",
        ),
        ("@ hello", 2, None, "@ followed by space"),
        ("test @ world", 6, None, "@ with spaces around"),
    ];

    for (input, cursor_pos, expected, description) in test_cases {
        let mut textarea = TextArea::new();
        textarea.insert_str(input);
        textarea.set_cursor(cursor_pos);

        let result = ChatComposer::current_at_token(&textarea);
        assert_eq!(
            result, expected,
            "Failed for case: {description} - input: '{input}', cursor: {cursor_pos}"
        );
    }
}

#[test]
fn test_current_at_token_cursor_positions() {
    let test_cases = vec![
        // Different cursor positions within a token
        ("@test", 0, Some("test".to_string()), "Cursor at @"),
        ("@test", 1, Some("test".to_string()), "Cursor after @"),
        ("@test", 5, Some("test".to_string()), "Cursor at end"),
        // Multiple tokens - cursor determines which token
        ("@file1 @file2", 0, Some("file1".to_string()), "First token"),
        (
            "@file1 @file2",
            8,
            Some("file2".to_string()),
            "Second token",
        ),
        // Edge cases
        ("@", 0, Some("".to_string()), "Only @ symbol"),
        ("@a", 2, Some("a".to_string()), "Single character after @"),
        ("", 0, None, "Empty input"),
    ];

    for (input, cursor_pos, expected, description) in test_cases {
        let mut textarea = TextArea::new();
        textarea.insert_str(input);
        textarea.set_cursor(cursor_pos);

        let result = ChatComposer::current_at_token(&textarea);
        assert_eq!(
            result, expected,
            "Failed for cursor position case: {description} - input: '{input}', cursor: {cursor_pos}",
        );
    }
}

#[test]
fn test_current_at_token_whitespace_boundaries() {
    let test_cases = vec![
        // Space boundaries
        (
            "aaa@aaa",
            4,
            None,
            "Connected @ token - no completion by design",
        ),
        (
            "aaa @aaa",
            5,
            Some("aaa".to_string()),
            "@ token after space",
        ),
        (
            "test @file.txt",
            7,
            Some("file.txt".to_string()),
            "@ token after space",
        ),
        // Full-width space boundaries
        (
            "test　@İstanbul",
            8,
            Some("İstanbul".to_string()),
            "@ token after full-width space",
        ),
        (
            "@ЙЦУ　@诶",
            10,
            Some("诶".to_string()),
            "Full-width space between Unicode tokens",
        ),
        // Tab and newline boundaries
        (
            "test\t@file",
            6,
            Some("file".to_string()),
            "@ token after tab",
        ),
    ];

    for (input, cursor_pos, expected, description) in test_cases {
        let mut textarea = TextArea::new();
        textarea.insert_str(input);
        textarea.set_cursor(cursor_pos);

        let result = ChatComposer::current_at_token(&textarea);
        assert_eq!(
            result, expected,
            "Failed for whitespace boundary case: {description} - input: '{input}', cursor: {cursor_pos}",
        );
    }
}

#[test]
fn slash_popup_model_first_for_mo_ui() {
    let (mut composer, _rx) = composer();

    // Type "/mo" humanlike so paste-burst doesn’t interfere.
    type_chars_humanlike(&mut composer, &['/', 'm', 'o']);

    let mut terminal = match Terminal::new(TestBackend::new(60, 5)) {
        Ok(t) => t,
        Err(e) => panic!("Failed to create terminal: {e}"),
    };
    terminal
        .draw(|f| composer.render(f.area(), f.buffer_mut()))
        .unwrap_or_else(|e| panic!("Failed to draw composer: {e}"));

    // Visual snapshot should show the slash popup with /model as the first entry.
    insta::assert_snapshot!("slash_popup_mo", terminal.backend());
}

#[test]
fn slash_popup_model_first_for_mo_logic() {
    let (mut composer, _rx) = composer();
    type_chars_humanlike(&mut composer, &['/', 'm', 'o']);

    match &composer.active_popup {
        ActivePopup::Command(popup) => {
            assert_eq!(popup.selected_item(), Some(SlashCommand::Model))
        }
        _ => panic!("slash popup not active after typing '/mo'"),
    }
}

#[test]
fn slash_popup_resume_for_res_ui() {
    let (mut composer, _rx) = composer();

    // Type "/res" humanlike so paste-burst doesn’t interfere.
    type_chars_humanlike(&mut composer, &['/', 'r', 'e', 's']);

    let mut terminal = Terminal::new(TestBackend::new(60, 6)).expect("terminal");
    terminal
        .draw(|f| composer.render(f.area(), f.buffer_mut()))
        .expect("draw composer");

    // Snapshot should show /resume as the first entry for /res.
    insta::assert_snapshot!("slash_popup_res", terminal.backend());
}

#[test]
fn slash_popup_resume_for_res_logic() {
    let (mut composer, _rx) = composer();
    type_chars_humanlike(&mut composer, &['/', 'r', 'e', 's']);

    match &composer.active_popup {
        ActivePopup::Command(popup) => {
            assert_eq!(popup.selected_item(), Some(SlashCommand::Resume))
        }
        _ => panic!("slash popup not active after typing '/res'"),
    }
}

#[test]
fn slash_compact_dispatches_command_and_does_not_submit_literal_text() {
    let (mut composer, _rx) = composer();

    // Type the slash command.
    type_chars_humanlike(&mut composer, &['/', 'c', 'o', 'm', 'p', 'a', 'c', 't']);

    // When a slash command is dispatched, the composer should return a
    // Command result (not submit literal text) and clear its textarea.
    assert_eq!(
        enter(&mut composer),
        InputResult::Command(SlashCommand::Compact)
    );
    assert!(composer.textarea.is_empty(), "composer should be cleared");
}

#[test]
fn slash_tab_completion_moves_cursor_to_end() {
    let (mut composer, _rx) = composer();

    type_chars_humanlike(&mut composer, &['/', 'c']);

    let (_result, _needs_redraw) =
        composer.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

    assert_eq!(composer.textarea.text(), "/compact ");
    assert_eq!(composer.textarea.cursor(), composer.textarea.text().len());
}

#[test]
fn slash_tab_then_enter_dispatches_builtin_command() {
    let (mut composer, _rx) = composer();

    // Type a prefix and complete with Tab, which inserts a trailing space
    // and moves the cursor beyond the '/name' token (hides the popup).
    type_chars_humanlike(&mut composer, &['/', 'd', 'i']);
    let (_res, _redraw) =
        composer.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(composer.textarea.text(), "/diff ");

    // Press Enter: should dispatch the command, not submit literal text.
    assert_eq!(
        enter(&mut composer),
        InputResult::Command(SlashCommand::Diff)
    );
    assert!(composer.textarea.is_empty());
}

#[test]
fn slash_mention_dispatches_command_and_inserts_at() {
    let (mut composer, _rx) = composer();

    type_chars_humanlike(&mut composer, &['/', 'm', 'e', 'n', 't', 'i', 'o', 'n']);

    assert_eq!(
        enter(&mut composer),
        InputResult::Command(SlashCommand::Mention)
    );
    assert!(composer.textarea.is_empty(), "composer should be cleared");
    composer.insert_str("@");
    assert_eq!(composer.textarea.text(), "@");
}

#[test]
fn slash_path_input_submits_without_command_error() {
    let (mut composer, rx) = composer();

    composer
        .textarea
        .set_text_clearing_elements("/Users/example/project/src/main.rs");

    assert_eq!(
        enter(&mut composer),
        InputResult::Submitted("/Users/example/project/src/main.rs".to_string())
    );
    assert!(composer.textarea.is_empty());
    assert!(rx.try_recv().is_err(), "unexpected event");
}

#[test]
fn slash_with_leading_space_submits_as_text() {
    let (mut composer, rx) = composer();

    composer
        .textarea
        .set_text_clearing_elements(" /this-looks-like-a-command");

    assert_eq!(
        enter(&mut composer),
        InputResult::Submitted("/this-looks-like-a-command".to_string())
    );
    assert!(composer.textarea.is_empty());
    assert!(rx.try_recv().is_err(), "unexpected event");
}

#[test]
fn an_unrecognized_command_is_reported_and_kept() {
    let (mut composer, rx) = composer();

    composer.textarea.set_text_clearing_elements("/frobnicate");

    assert_eq!(enter(&mut composer), InputResult::None);
    assert_eq!(composer.textarea.text(), "/frobnicate");
    match rx.try_recv() {
        Ok(AppEvent::InsertHistoryCell(cell)) => {
            let text: String = cell.display_lines(80)[0]
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect();
            assert!(
                text.contains("Unrecognized command '/frobnicate'"),
                "{text}"
            );
        }
        _ => panic!("expected the unrecognized command to be reported"),
    }
}

#[test]
fn slash_popup_not_activated_for_slash_space_text_history_like_input() {
    let (mut composer, _rx) = composer();

    // Simulate history-like content: "/ test"
    composer.set_text_content("/ test".to_string());

    // After set_text_content -> sync_popups is called; popup should NOT be Command.
    assert!(
        matches!(composer.active_popup, ActivePopup::None),
        "expected no slash popup for '/ test'"
    );

    // Up should be handled by history navigation path, not slash popup handler.
    let (result, _redraw) =
        composer.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(result, InputResult::None);
}

#[test]
fn slash_popup_activated_for_bare_slash_and_valid_prefixes() {
    let (mut composer, _rx) = composer();

    // Case 1: bare "/"
    composer.set_text_content("/".to_string());
    assert!(
        matches!(composer.active_popup, ActivePopup::Command(_)),
        "bare '/' should activate slash popup"
    );

    // Case 2: valid prefix "/re" (matches /resume)
    composer.set_text_content("/re".to_string());
    assert!(
        matches!(composer.active_popup, ActivePopup::Command(_)),
        "'/re' should activate slash popup via prefix match"
    );

    // Case 3: fuzzy match "/ac" (subsequence of /compact)
    composer.set_text_content("/ac".to_string());
    assert!(
        matches!(composer.active_popup, ActivePopup::Command(_)),
        "'/ac' should activate slash popup via fuzzy match"
    );

    // Case 4: invalid prefix "/zzz" – matches no built-in command, so the
    // popup does not open.
    composer.set_text_content("/zzz".to_string());
    assert!(
        matches!(composer.active_popup, ActivePopup::None),
        "'/zzz' should not activate slash popup because it is not a prefix of any built-in command"
    );
}

fn file_match(path: &str, root: &str, primary: bool) -> FileMatch {
    FileMatch {
        path: PathBuf::from(path),
        root: PathBuf::from(root),
        primary,
        indices: vec![0, 1],
    }
}

/// The queries the composer asked the file search for.
fn searches(rx: &Receiver<AppEvent>) -> Vec<String> {
    std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|event| match event {
            AppEvent::StartFileSearch(query) => Some(query),
            _ => None,
        })
        .collect()
}

#[test]
fn an_at_token_searches_files_and_enter_inserts_the_mention() {
    let (mut composer, rx) = composer();
    composer.set_text_content("look at @ma".to_string());
    assert!(matches!(composer.active_popup, ActivePopup::File(_)));
    assert_eq!(searches(&rx), vec!["ma".to_string()]);

    composer.on_file_search_result(
        "ma".to_string(),
        vec![
            file_match("src/main.rs", "/work", true),
            file_match("lib/map.rs", "/other", false),
        ],
    );
    let mut terminal = Terminal::new(TestBackend::new(60, 6)).expect("terminal");
    terminal
        .draw(|f| composer.render(f.area(), f.buffer_mut()))
        .expect("draw composer");
    insta::assert_snapshot!("file_popup_matches", terminal.backend());

    assert_eq!(enter(&mut composer), InputResult::None);
    assert_eq!(composer.textarea.text(), "look at src/main.rs ");
    assert!(matches!(composer.active_popup, ActivePopup::None));
    // Leaving the token ends the search.
    assert_eq!(searches(&rx), vec![String::new()]);
}

#[test]
fn a_file_in_another_root_is_mentioned_by_its_whole_path() {
    let (mut composer, _rx) = composer();
    composer.set_text_content("@ma".to_string());
    composer.on_file_search_result(
        "ma".to_string(),
        vec![file_match("lib/map.rs", "/other", false)],
    );
    let (_result, _redraw) =
        composer.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    let whole = PathBuf::from("/other").join("lib/map.rs");
    assert_eq!(
        composer.textarea.text(),
        format!("{} ", whole.to_string_lossy())
    );
}

#[test]
fn esc_hides_the_file_popup_until_the_token_changes() {
    let (mut composer, _rx) = composer();
    composer.set_text_content("@ma".to_string());
    let (_result, _redraw) =
        composer.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(matches!(composer.active_popup, ActivePopup::None));
    assert_eq!(composer.textarea.text(), "@ma");

    type_chars_humanlike(&mut composer, &['i']);
    assert!(matches!(composer.active_popup, ActivePopup::File(_)));
}
