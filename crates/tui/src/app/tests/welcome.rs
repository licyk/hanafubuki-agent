use hanafubuki_agent::test_support::assistant_events;
use ratatui::crossterm::event::KeyCode;

use super::Harness;
use crate::transcript_mode::TranscriptMode;

fn has_mascot(harness: &Harness) -> bool {
    harness
        .screen()
        .chars()
        .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c))
}

#[test]
fn welcome_fades_with_the_draft_and_stays_gone_after_a_turn_in_both_modes() {
    for mode in [TranscriptMode::Owned, TranscriptMode::Terminal] {
        let mut harness = Harness::start_with(vec![assistant_events("All done.")], true, mode);
        harness.pump_until_screen_shows("directory:");
        harness.pump_until("welcome mascot", has_mascot);
        if mode == TranscriptMode::Owned {
            if let Some(path) = std::env::var_os("HANAFUBUKI_AGENT_WELCOME_TUI_PREVIEW") {
                std::thread::sleep(std::time::Duration::from_millis(500));
                harness.app.draw(&mut harness.tui).unwrap();
                let screen = harness.tui.terminal.backend().vt100().screen();
                let (height, width) = screen.size();
                let cells: Vec<_> = (0..height)
                    .flat_map(|row| {
                        (0..width).map(move |col| {
                            let cell = screen.cell(row, col).unwrap();
                            let color = match cell.fgcolor() {
                                vt100::Color::Rgb(r, g, b) => Some([r, g, b]),
                                _ => None,
                            };
                            serde_json::json!({"symbol": cell.contents(), "color": color})
                        })
                    })
                    .collect();
                std::fs::write(
                    path,
                    serde_json::to_vec(
                        &serde_json::json!({"width": width, "height": height, "cells": cells}),
                    )
                    .unwrap(),
                )
                .unwrap();
            }
        }
        assert!(!harness
            .transcript()
            .chars()
            .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)));
        harness.key(KeyCode::Char('h'));
        harness.pump_until("draft fades the mascot out", |h| !has_mascot(h));
        harness.key(KeyCode::Backspace);
        harness.pump_until("clearing the draft restores the mascot", has_mascot);
        harness.exchange("Hello", "All done.");
        harness.pump_until("the completed conversation has no mascot", |h| {
            !has_mascot(h)
        });
        harness.type_and_submit("/clear");
        harness.pump_until_turn_ends();
        assert!(!has_mascot(&harness));
        harness.type_and_submit("/new");
        harness.pump_until("a new conversation restores the mascot", has_mascot);
    }
}

#[test]
fn exiting_an_empty_scrollback_session_erases_the_decoration() {
    let mut harness = Harness::start(Vec::new());
    harness.pump_until("welcome mascot", has_mascot);
    harness.app.write_tail_at_exit(&mut harness.tui).unwrap();
    assert!(!has_mascot(&harness));
    assert!(!harness
        .transcript()
        .chars()
        .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)));
}

#[test]
fn welcome_yields_to_small_windows_and_overlays_then_reappears() {
    let mut harness = Harness::owned(Vec::new());
    harness.pump_until("welcome mascot", has_mascot);
    harness.tui.terminal.backend_mut().resize(50, 20);
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(!has_mascot(&harness));
    harness.tui.terminal.backend_mut().resize(100, 40);
    harness.pump_until("welcome after enlarging the window", has_mascot);
    harness.type_and_submit("/config");
    harness.pump_until_screen_shows("Settings");
    assert!(!has_mascot(&harness));
    harness.leave_settings();
    harness.pump_until("welcome after closing the overlay", has_mascot);
}

#[test]
fn global_settings_save_and_apply_the_welcome_toggle_and_pose() {
    let mut harness = Harness::owned(Vec::new());
    harness.pump_until("welcome mascot", has_mascot);
    harness.type_and_submit("/config interface");
    harness.pump_until_screen_shows("Terminal interface");
    harness.select_setting("Welcome mascot");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved");
    assert!(harness.config_file().contains("welcomeEnabled = false"));
    assert!(!harness.app.chat_widget.preferences().welcome_enabled);
    harness.select_setting("Welcome mascot theme");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Random");
    harness.key(KeyCode::Down);
    harness.key(KeyCode::Enter);
    harness.pump_until("fixed center theme applied", |h| {
        h.app.chat_widget.preferences().welcome_theme == hanafubuki_agent::AgentWelcomeTheme::Center
    });
    assert!(harness.config_file().contains("welcomeTheme = \"center\""));
    harness.leave_settings();
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(!has_mascot(&harness));
    harness.type_and_submit("/config interface");
    harness.pump_until_screen_shows("Terminal interface");
    harness.select_setting("Welcome mascot");
    harness.key(KeyCode::Enter);
    harness.pump_until("welcome enabled", |h| {
        h.app.chat_widget.preferences().welcome_enabled
    });
    harness.leave_settings();
    harness.pump_until("welcome restored by setting", has_mascot);
}
