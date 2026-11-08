// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use super::*;
use std::time::Duration;

#[test]
fn footer_hint_row_is_separated_from_composer() {
    let composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    let area = Rect::new(0, 0, 40, 6);
    let mut buf = Buffer::empty(area);
    composer.render(area, &mut buf);

    let row_to_string = |y: u16| {
        let mut row = String::new();
        for x in 0..area.width {
            row.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
        }
        row
    };

    let mut hint_row: Option<(u16, String)> = None;
    for y in 0..area.height {
        let row = row_to_string(y);
        if row.contains("? for shortcuts") {
            hint_row = Some((y, row));
            break;
        }
    }

    let (hint_row_idx, hint_row_contents) =
        hint_row.expect("expected footer hint row to be rendered");
    assert_eq!(
        hint_row_idx,
        area.height - 1,
        "hint row should occupy the bottom line: {hint_row_contents:?}",
    );

    assert!(
        hint_row_idx > 0,
        "expected a spacing row above the footer hints",
    );

    let spacing_row = row_to_string(hint_row_idx - 1);
    assert_eq!(
        spacing_row.trim(),
        "",
        "expected blank spacing row above hints but saw: {spacing_row:?}",
    );
}

fn snapshot_composer_state_with_width<F>(
    name: &str,
    width: u16,
    enhanced_keys_supported: bool,
    setup: F,
) where
    F: FnOnce(&mut ChatComposer),
{
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let mut composer = ChatComposer::new(
        true,
        enhanced_keys_supported,
        "Ask Codex to do anything".to_string(),
    );
    setup(&mut composer);
    let footer_props = composer.footer_props();
    let footer_lines = footer_height(footer_props);
    let height = footer_lines + 8;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| composer.render(f.area(), f.buffer_mut()))
        .unwrap();
    insta::assert_snapshot!(name, terminal.backend());
}

fn snapshot_composer_state<F>(name: &str, enhanced_keys_supported: bool, setup: F)
where
    F: FnOnce(&mut ChatComposer),
{
    snapshot_composer_state_with_width(name, 100, enhanced_keys_supported, setup);
}

#[test]
fn footer_mode_snapshots() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    snapshot_composer_state("footer_mode_shortcut_overlay", true, |composer| {
        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    });

    snapshot_composer_state("footer_mode_ctrl_c_quit", true, |composer| {
        composer.show_quit_shortcut_hint(key_hint::ctrl(KeyCode::Char('c')), true);
    });

    snapshot_composer_state("footer_mode_ctrl_c_interrupt", true, |composer| {
        composer.set_task_running(true);
        composer.show_quit_shortcut_hint(key_hint::ctrl(KeyCode::Char('c')), true);
    });

    snapshot_composer_state("footer_mode_hidden_while_typing", true, |composer| {
        type_chars_humanlike(composer, &['h']);
    });
}

#[test]
fn base_footer_mode_tracks_empty_state_after_quit_hint_expires() {
    use ratatui::crossterm::event::KeyCode;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    type_chars_humanlike(&mut composer, &['d']);
    composer.show_quit_shortcut_hint(key_hint::ctrl(KeyCode::Char('c')), true);
    composer.quit_shortcut_expires_at = Some(Instant::now() - std::time::Duration::from_secs(1));

    assert_eq!(composer.footer_mode(), FooterMode::ComposerHasDraft);

    composer.set_text_content(String::new());
    assert_eq!(composer.footer_mode(), FooterMode::ComposerEmpty);
}

#[test]
fn clear_for_ctrl_c_records_cleared_draft() {
    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    composer.set_text_content("draft text".to_string());
    assert_eq!(composer.clear_for_ctrl_c(), Some("draft text".to_string()));
    assert!(composer.is_empty());

    assert_eq!(
        composer.history.navigate_up(),
        Some("draft text".to_string())
    );
}

/// Behavior: `?` toggles the shortcut overlay only when the composer is otherwise empty. After
/// any typing has occurred, `?` should be inserted as a literal character.

#[test]
fn question_mark_only_toggles_on_first_char() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    let (result, needs_redraw) =
        composer.handle_key_event(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    assert_eq!(result, InputResult::None);
    assert!(needs_redraw, "toggling overlay should request redraw");
    assert_eq!(composer.footer_mode, FooterMode::ShortcutOverlay);

    // Toggle back to prompt mode so subsequent typing captures characters.
    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    assert_eq!(composer.footer_mode, FooterMode::ComposerEmpty);

    type_chars_humanlike(&mut composer, &['h']);
    assert_eq!(composer.textarea.text(), "h");
    assert_eq!(composer.footer_mode(), FooterMode::ComposerHasDraft);

    let (result, needs_redraw) =
        composer.handle_key_event(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    assert_eq!(result, InputResult::None);
    assert!(needs_redraw, "typing should still mark the view dirty");
    let _ = flush_after_paste_burst(&mut composer);
    assert_eq!(composer.textarea.text(), "h?");
    assert_eq!(composer.footer_mode, FooterMode::ComposerEmpty);
    assert_eq!(composer.footer_mode(), FooterMode::ComposerHasDraft);
}

/// Behavior: while a paste-like burst is being captured, `?` must not toggle the shortcut
/// overlay; it should be treated as part of the pasted content.

#[test]
fn question_mark_does_not_toggle_during_paste_burst() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    // Force an active paste burst so this test doesn't depend on tight timing.
    composer
        .paste_burst
        .begin_with_retro_grabbed(String::new(), Instant::now());

    for ch in ['h', 'i', '?', 't', 'h', 'e', 'r', 'e'] {
        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    assert!(composer.is_in_paste_burst());
    assert_eq!(composer.textarea.text(), "");

    let _ = flush_after_paste_burst(&mut composer);

    assert_eq!(composer.textarea.text(), "hi?there");
    assert_ne!(composer.footer_mode, FooterMode::ShortcutOverlay);
}

#[test]
fn shortcut_overlay_persists_while_task_running() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    assert_eq!(composer.footer_mode, FooterMode::ShortcutOverlay);

    composer.set_task_running(true);

    assert_eq!(composer.footer_mode, FooterMode::ShortcutOverlay);
    assert_eq!(composer.footer_mode(), FooterMode::ShortcutOverlay);
}

#[test]
fn ascii_prefix_survives_non_ascii_followup() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
    assert!(composer.is_in_paste_burst());

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('あ'), KeyModifiers::NONE));

    let (result, _) = composer.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    match result {
        InputResult::Submitted(text) => assert_eq!(text, "1あ"),
        _ => panic!("expected Submitted"),
    }
}

/// Behavior: a single non-ASCII char should be inserted immediately (IME-friendly) and should
/// not create any paste-burst state.

#[test]
fn non_ascii_char_inserts_immediately_without_burst_state() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('あ'), KeyModifiers::NONE));

    assert_eq!(composer.textarea.text(), "あ");
    assert!(!composer.is_in_paste_burst());
}

/// Behavior: while we're capturing a paste-like burst, Enter should be treated as a newline
/// within the burst (not as "submit"), and the whole payload should flush as one paste.

#[test]
fn non_ascii_burst_buffers_enter_and_flushes_multiline() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    composer
        .paste_burst
        .begin_with_retro_grabbed(String::new(), Instant::now());

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('你'), KeyModifiers::NONE));
    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('好'), KeyModifiers::NONE));
    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));

    assert!(composer.textarea.text().is_empty());
    let _ = flush_after_paste_burst(&mut composer);
    assert_eq!(composer.textarea.text(), "你好\nhi");
}

/// Behavior: a paste-like burst may include a full-width/ideographic space (U+3000). It should
/// still be captured as a single paste payload and preserve the exact Unicode content.

#[test]
fn non_ascii_burst_preserves_ideographic_space_and_ascii() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    composer
        .paste_burst
        .begin_with_retro_grabbed(String::new(), Instant::now());

    for ch in ['你', '　', '好'] {
        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    for ch in ['h', 'i'] {
        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }

    assert!(composer.textarea.text().is_empty());
    let _ = flush_after_paste_burst(&mut composer);
    assert_eq!(composer.textarea.text(), "你　好\nhi");
}

/// Behavior: a large multi-line payload containing both non-ASCII and ASCII (e.g. "UTF-8",
/// "Unicode") should be captured as a single paste-like burst, and Enter key events should
/// become `\n` within the buffered content.

#[test]
fn non_ascii_burst_buffers_large_multiline_mixed_ascii_and_unicode() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    const LARGE_MIXED_PAYLOAD: &str = "天地玄黄 宇宙洪荒\n\
日月盈昃 辰宿列张\n\
寒来暑往 秋收冬藏\n\
\n\
你好世界 编码测试\n\
汉字处理 UTF-8\n\
终端显示 正确无误\n\
\n\
风吹竹林 月照大江\n\
白云千载 青山依旧\n\
程序员 与 Unicode 同行";

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    // Force an active burst so the test doesn't depend on timing heuristics.
    composer
        .paste_burst
        .begin_with_retro_grabbed(String::new(), Instant::now());

    for ch in LARGE_MIXED_PAYLOAD.chars() {
        let code = if ch == '\n' {
            KeyCode::Enter
        } else {
            KeyCode::Char(ch)
        };
        let _ = composer.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
    }

    assert!(composer.textarea.text().is_empty());
    let _ = flush_after_paste_burst(&mut composer);
    assert_eq!(composer.textarea.text(), LARGE_MIXED_PAYLOAD);
}

/// Behavior: while a paste-like burst is active, Enter should not submit; it should insert a
/// newline into the buffered payload and flush as a single paste later.

#[test]
fn ascii_burst_treats_enter_as_newline() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    let mut now = Instant::now();
    let step = Duration::from_millis(1);

    let _ = composer
        .handle_input_basic_with_time(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE), now);
    now += step;
    let _ = composer
        .handle_input_basic_with_time(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), now);
    now += step;

    let (result, _) = composer.handle_submission(now);
    assert!(
        matches!(result, InputResult::None),
        "Enter during a burst should insert newline, not submit"
    );

    for ch in ['t', 'h', 'e', 'r', 'e'] {
        now += step;
        let _ = composer.handle_input_basic_with_time(
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
            now,
        );
    }

    assert!(composer.textarea.text().is_empty());
    let flush_time = now + PasteBurst::recommended_active_flush_delay() + step;
    let flushed = composer.handle_paste_burst_flush(flush_time);
    assert!(flushed, "expected paste burst to flush");
    assert_eq!(composer.textarea.text(), "hi\nthere");
}

/// Behavior: even if Enter suppression would normally be active for a burst, Enter should
/// still dispatch a built-in slash command when the first line begins with `/`.

#[test]
fn non_char_key_flushes_active_burst_before_input() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    // Force an active burst so we can deterministically buffer characters without relying on
    // timing.
    composer
        .paste_burst
        .begin_with_retro_grabbed(String::new(), Instant::now());

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    assert!(composer.textarea.text().is_empty());
    assert!(composer.is_in_paste_burst());

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(composer.textarea.text(), "hi");
    assert_eq!(composer.textarea.cursor(), 1);
    assert!(!composer.is_in_paste_burst());
}

/// Behavior: enabling `disable_paste_burst` flushes any held first character (flicker
/// suppression) and then inserts subsequent chars immediately without creating burst state.

#[test]
fn handle_paste_small_inserts_text() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    let needs_redraw = composer.handle_paste("hello".to_string());
    assert!(needs_redraw);
    assert_eq!(composer.textarea.text(), "hello");
    assert!(composer.pending_pastes.is_empty());

    let (result, _) = composer.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    match result {
        InputResult::Submitted(text) => assert_eq!(text, "hello"),
        _ => panic!("expected Submitted"),
    }
}

#[test]
fn empty_enter_returns_none() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    // Ensure composer is empty and press Enter.
    assert!(composer.textarea.text().is_empty());
    let (result, _needs_redraw) =
        composer.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    match result {
        InputResult::None => {}
        other => panic!("expected None for empty enter, got: {other:?}"),
    }
}

/// Behavior: a large explicit paste inserts a placeholder into the textarea, stores the full
/// content in `pending_pastes`, and expands the placeholder to the full content on submit.

#[test]
fn handle_paste_large_uses_placeholder_and_replaces_on_submit() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    let large = "x".repeat(LARGE_PASTE_CHAR_THRESHOLD + 10);
    let needs_redraw = composer.handle_paste(large.clone());
    assert!(needs_redraw);
    let placeholder = format!("[Pasted Content {} chars]", large.chars().count());
    assert_eq!(composer.textarea.text(), placeholder);
    assert_eq!(composer.pending_pastes.len(), 1);
    assert_eq!(composer.pending_pastes[0].0, placeholder);
    assert_eq!(composer.pending_pastes[0].1, large);

    let (result, _) = composer.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    match result {
        InputResult::Submitted(text) => assert_eq!(text, large),
        _ => panic!("expected Submitted"),
    }
    assert!(composer.pending_pastes.is_empty());
}

/// Behavior: editing that removes a paste placeholder should also clear the associated
/// `pending_pastes` entry so it cannot be submitted accidentally.

#[test]
fn edit_clears_pending_paste() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let large = "y".repeat(LARGE_PASTE_CHAR_THRESHOLD + 1);
    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    composer.handle_paste(large);
    assert_eq!(composer.pending_pastes.len(), 1);

    // Any edit that removes the placeholder should clear pending_paste
    composer.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert!(composer.pending_pastes.is_empty());
}

#[test]
fn ui_snapshots() {
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;
    use ratatui::Terminal;

    let mut terminal = match Terminal::new(TestBackend::new(100, 10)) {
        Ok(t) => t,
        Err(e) => panic!("Failed to create terminal: {e}"),
    };

    let test_cases = vec![
        ("empty", None),
        ("small", Some("short".to_string())),
        ("large", Some("z".repeat(LARGE_PASTE_CHAR_THRESHOLD + 5))),
        ("multiple_pastes", None),
        ("backspace_after_pastes", None),
    ];

    for (name, input) in test_cases {
        // Create a fresh composer for each test case
        let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

        if let Some(text) = input {
            composer.handle_paste(text);
        } else if name == "multiple_pastes" {
            // First large paste
            composer.handle_paste("x".repeat(LARGE_PASTE_CHAR_THRESHOLD + 3));
            // Second large paste
            composer.handle_paste("y".repeat(LARGE_PASTE_CHAR_THRESHOLD + 7));
            // Small paste
            composer.handle_paste(" another short paste".to_string());
        } else if name == "backspace_after_pastes" {
            // Three large pastes
            composer.handle_paste("a".repeat(LARGE_PASTE_CHAR_THRESHOLD + 2));
            composer.handle_paste("b".repeat(LARGE_PASTE_CHAR_THRESHOLD + 4));
            composer.handle_paste("c".repeat(LARGE_PASTE_CHAR_THRESHOLD + 6));
            // Move cursor to end and press backspace
            composer.textarea.set_cursor(composer.textarea.text().len());
            composer.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        }

        terminal
            .draw(|f| composer.render(f.area(), f.buffer_mut()))
            .unwrap_or_else(|e| panic!("Failed to draw {name} composer: {e}"));

        insta::assert_snapshot!(name, terminal.backend());
    }
}

fn flush_after_paste_burst(composer: &mut ChatComposer) -> bool {
    std::thread::sleep(PasteBurst::recommended_active_flush_delay());
    composer.flush_paste_burst_if_due()
}

// Test helper: simulate human typing with a brief delay and flush the paste-burst buffer

fn type_chars_humanlike(composer: &mut ChatComposer, chars: &[char]) {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;
    for &ch in chars {
        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        std::thread::sleep(PasteBurst::recommended_flush_delay());
        let _ = composer.flush_paste_burst_if_due();
    }
}

#[test]
fn test_multiple_pastes_submission() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    // Define test cases: (paste content, is_large)
    let test_cases = [
        ("x".repeat(LARGE_PASTE_CHAR_THRESHOLD + 3), true),
        (" and ".to_string(), false),
        ("y".repeat(LARGE_PASTE_CHAR_THRESHOLD + 7), true),
    ];

    // Expected states after each paste
    let mut expected_text = String::new();
    let mut expected_pending_count = 0;

    // Apply all pastes and build expected state
    let states: Vec<_> = test_cases
        .iter()
        .map(|(content, is_large)| {
            composer.handle_paste(content.clone());
            if *is_large {
                let placeholder = format!("[Pasted Content {} chars]", content.chars().count());
                expected_text.push_str(&placeholder);
                expected_pending_count += 1;
            } else {
                expected_text.push_str(content);
            }
            (expected_text.clone(), expected_pending_count)
        })
        .collect();

    // Verify all intermediate states were correct
    assert_eq!(
        states,
        vec![
            (
                format!("[Pasted Content {} chars]", test_cases[0].0.chars().count()),
                1
            ),
            (
                format!(
                    "[Pasted Content {} chars] and ",
                    test_cases[0].0.chars().count()
                ),
                1
            ),
            (
                format!(
                    "[Pasted Content {} chars] and [Pasted Content {} chars]",
                    test_cases[0].0.chars().count(),
                    test_cases[2].0.chars().count()
                ),
                2
            ),
        ]
    );

    // Submit and verify final expansion
    let (result, _) = composer.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    if let InputResult::Submitted(text) = result {
        assert_eq!(text, format!("{} and {}", test_cases[0].0, test_cases[2].0));
    } else {
        panic!("expected Submitted");
    }
}

#[test]
fn test_placeholder_deletion() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    // Define test cases: (content, is_large)
    let test_cases = [
        ("a".repeat(LARGE_PASTE_CHAR_THRESHOLD + 5), true),
        (" and ".to_string(), false),
        ("b".repeat(LARGE_PASTE_CHAR_THRESHOLD + 6), true),
    ];

    // Apply all pastes
    let mut current_pos = 0;
    let states: Vec<_> = test_cases
        .iter()
        .map(|(content, is_large)| {
            composer.handle_paste(content.clone());
            if *is_large {
                let placeholder = format!("[Pasted Content {} chars]", content.chars().count());
                current_pos += placeholder.len();
            } else {
                current_pos += content.len();
            }
            (
                composer.textarea.text().to_string(),
                composer.pending_pastes.len(),
                current_pos,
            )
        })
        .collect();

    // Delete placeholders one by one and collect states
    let mut deletion_states = vec![];

    // First deletion
    composer.textarea.set_cursor(states[0].2);
    composer.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    deletion_states.push((
        composer.textarea.text().to_string(),
        composer.pending_pastes.len(),
    ));

    // Second deletion
    composer.textarea.set_cursor(composer.textarea.text().len());
    composer.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    deletion_states.push((
        composer.textarea.text().to_string(),
        composer.pending_pastes.len(),
    ));

    // Verify all states
    assert_eq!(
        deletion_states,
        vec![
            (" and [Pasted Content 1006 chars]".to_string(), 1),
            (" and ".to_string(), 0),
        ]
    );
}

/// Behavior: if multiple large pastes share the same placeholder label (same char count),
/// deleting one placeholder removes only its corresponding `pending_pastes` entry.

#[test]
fn deleting_duplicate_length_pastes_removes_only_target() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    let paste = "x".repeat(LARGE_PASTE_CHAR_THRESHOLD + 4);
    let placeholder_base = format!("[Pasted Content {} chars]", paste.chars().count());
    let placeholder_second = format!("{placeholder_base} #2");

    composer.handle_paste(paste.clone());
    composer.handle_paste(paste.clone());
    assert_eq!(
        composer.textarea.text(),
        format!("{placeholder_base}{placeholder_second}")
    );
    assert_eq!(composer.pending_pastes.len(), 2);

    composer.textarea.set_cursor(composer.textarea.text().len());
    composer.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

    assert_eq!(composer.textarea.text(), placeholder_base);
    assert_eq!(composer.pending_pastes.len(), 1);
    assert_eq!(composer.pending_pastes[0].0, placeholder_base);
    assert_eq!(composer.pending_pastes[0].1, paste);
}

/// Behavior: large-paste placeholder numbering does not get reused after deletion, so a new
/// paste of the same length gets a new unique placeholder label.

#[test]
fn large_paste_numbering_does_not_reuse_after_deletion() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    let paste = "x".repeat(LARGE_PASTE_CHAR_THRESHOLD + 4);
    let base = format!("[Pasted Content {} chars]", paste.chars().count());
    let second = format!("{base} #2");
    let third = format!("{base} #3");

    composer.handle_paste(paste.clone());
    composer.handle_paste(paste.clone());
    assert_eq!(composer.textarea.text(), format!("{base}{second}"));

    composer.textarea.set_cursor(base.len());
    composer.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(composer.textarea.text(), second);
    assert_eq!(composer.pending_pastes.len(), 1);
    assert_eq!(composer.pending_pastes[0].0, second);

    composer.textarea.set_cursor(composer.textarea.text().len());
    composer.handle_paste(paste);

    assert_eq!(composer.textarea.text(), format!("{second}{third}"));
    assert_eq!(composer.pending_pastes.len(), 2);
    assert_eq!(composer.pending_pastes[0].0, second);
    assert_eq!(composer.pending_pastes[1].0, third);
}

#[test]
fn test_partial_placeholder_deletion() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    // Define test cases: (cursor_position_from_end, expected_pending_count)
    let test_cases = [
        5, // Delete from middle - should clear tracking
        0, // Delete from end - should clear tracking
    ];

    let paste = "x".repeat(LARGE_PASTE_CHAR_THRESHOLD + 4);
    let placeholder = format!("[Pasted Content {} chars]", paste.chars().count());

    let states: Vec<_> = test_cases
        .into_iter()
        .map(|pos_from_end| {
            composer.handle_paste(paste.clone());
            composer
                .textarea
                .set_cursor(placeholder.len() - pos_from_end);
            composer.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
            let result = (
                composer.textarea.text().contains(&placeholder),
                composer.pending_pastes.len(),
            );
            composer.textarea.set_text_clearing_elements("");
            result
        })
        .collect();

    assert_eq!(
        states,
        vec![
            (false, 0), // After deleting from middle
            (false, 0), // After deleting from end
        ]
    );
}

// --- Image attachment tests ---

#[test]
fn pending_first_ascii_char_flushes_as_typed() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    assert!(composer.is_in_paste_burst());
    assert!(composer.textarea.text().is_empty());

    std::thread::sleep(PasteBurst::recommended_flush_delay());
    let flushed = composer.flush_paste_burst_if_due();
    assert!(flushed, "expected pending first char to flush");
    assert_eq!(composer.textarea.text(), "h");
    assert!(!composer.is_in_paste_burst());
}

/// Behavior: fast "paste-like" ASCII input should buffer and then flush as a single paste. If
/// the payload is small, it should insert directly (no placeholder).

#[test]
fn burst_paste_fast_small_buffers_and_flushes_on_stop() {
    use ratatui::crossterm::event::KeyCode;
    use ratatui::crossterm::event::KeyEvent;
    use ratatui::crossterm::event::KeyModifiers;

    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    let count = 32;
    let mut now = Instant::now();
    let step = Duration::from_millis(1);
    for _ in 0..count {
        let _ = composer.handle_input_basic_with_time(
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
            now,
        );
        assert!(
            composer.is_in_paste_burst(),
            "expected active paste burst during fast typing"
        );
        assert!(
            composer.textarea.text().is_empty(),
            "text should not appear during burst"
        );
        now += step;
    }

    assert!(
        composer.textarea.text().is_empty(),
        "text should remain empty until flush"
    );
    let flush_time = now + PasteBurst::recommended_active_flush_delay() + step;
    let flushed = composer.handle_paste_burst_flush(flush_time);
    assert!(flushed, "expected buffered text to flush after stop");
    assert_eq!(composer.textarea.text(), "a".repeat(count));
    assert!(
        composer.pending_pastes.is_empty(),
        "no placeholder for small burst"
    );
}

/// Behavior: fast "paste-like" ASCII input should buffer and then flush as a single paste. If
/// the payload is large, it should insert a placeholder and defer the full text until submit.

#[test]
fn burst_paste_fast_large_inserts_placeholder_on_flush() {
    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    let count = LARGE_PASTE_CHAR_THRESHOLD + 1; // > threshold to trigger placeholder
    let mut now = Instant::now();
    let step = Duration::from_millis(1);
    for _ in 0..count {
        let _ = composer.handle_input_basic_with_time(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
            now,
        );
        now += step;
    }

    // Nothing should appear until we stop and flush
    assert!(composer.textarea.text().is_empty());
    let flush_time = now + PasteBurst::recommended_active_flush_delay() + step;
    let flushed = composer.handle_paste_burst_flush(flush_time);
    assert!(flushed, "expected flush after stopping fast input");

    let expected_placeholder = format!("[Pasted Content {count} chars]");
    assert_eq!(composer.textarea.text(), expected_placeholder);
    assert_eq!(composer.pending_pastes.len(), 1);
    assert_eq!(composer.pending_pastes[0].0, expected_placeholder);
    assert_eq!(composer.pending_pastes[0].1.len(), count);
    assert!(composer.pending_pastes[0].1.chars().all(|c| c == 'x'));
}

/// Behavior: human-like typing (with delays between chars) should not be classified as a paste
/// burst. Characters should appear immediately and should not trigger a paste placeholder.

#[test]
fn humanlike_typing_1000_chars_appears_live_no_placeholder() {
    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    let count = LARGE_PASTE_CHAR_THRESHOLD; // 1000 in current config
    let chars: Vec<char> = vec!['z'; count];
    type_chars_humanlike(&mut composer, &chars);

    assert_eq!(composer.textarea.text(), "z".repeat(count));
    assert!(composer.pending_pastes.is_empty());
}

#[test]
fn current_text_with_pending_expands_placeholders() {
    let mut composer = ChatComposer::new(true, false, "Ask Codex to do anything".to_string());

    let placeholder = "[Pasted Content 5 chars]".to_string();
    composer.textarea.insert_element(&placeholder);
    composer
        .pending_pastes
        .push((placeholder.clone(), "hello".to_string()));

    assert_eq!(
        composer.current_text_with_pending(),
        "hello".to_string(),
        "placeholder should expand to actual text"
    );
}
