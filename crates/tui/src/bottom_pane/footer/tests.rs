// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use std::collections::BTreeMap;

use super::*;
use crate::keymap::Keymap;
use insta::assert_snapshot;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn snapshot_footer(name: &str, props: FooterProps<'_>) {
    snapshot_footer_at(name, props, 80);
}

fn snapshot_footer_at(name: &str, props: FooterProps<'_>, width: u16) {
    let height = footer_height(props, width).max(1);
    assert_snapshot!(name, draw(props, width, height).backend());
}

fn draw(props: FooterProps<'_>, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, f.area().width, height);
            render_footer(area, f.buffer_mut(), props);
        })
        .unwrap();
    terminal
}

/// The rows drawn, trimmed.
fn rows(props: FooterProps<'_>, width: u16, height: u16) -> Vec<String> {
    let terminal = draw(props, width, height);
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// The shortcuts drawn, a row a line, each run of spaces made one.
fn overlay_text(props: FooterProps<'_>, width: u16) -> String {
    rows(props, width, footer_height(props, width))
        .iter()
        .map(|row| row.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn props(mode: FooterMode) -> FooterProps<'static> {
    FooterProps {
        mode,
        use_shift_enter_hint: false,
        quit_shortcut_key: key_hint::ctrl(KeyCode::Char('c')),
        status_line: None,
        task_running: false,
        agents_hint: false,
        second_conversation: false,
    }
}

fn overlay() -> FooterProps<'static> {
    FooterProps {
        agents_hint: true,
        ..props(FooterMode::ShortcutOverlay)
    }
}

fn install(entries: &[(&str, &str, &[&str])]) {
    let mut config: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for (context, action, keys) in entries {
        config.entry(context.to_string()).or_default().insert(
            action.to_string(),
            keys.iter().map(|key| key.to_string()).collect(),
        );
    }
    let (keymap, warnings) = Keymap::resolve(&config);
    assert!(warnings.is_empty(), "{warnings:#?}");
    keymap::install(keymap);
}

#[test]
fn footer_snapshots() {
    snapshot_footer("footer_shortcuts_default", props(FooterMode::ComposerEmpty));

    snapshot_footer(
        "footer_agents_and_shortcuts",
        FooterProps {
            agents_hint: true,
            ..props(FooterMode::ComposerEmpty)
        },
    );
    let status = Line::from("model · ~/project");
    snapshot_footer(
        "footer_status_line_and_agents",
        FooterProps {
            agents_hint: true,
            status_line: Some(&status),
            ..props(FooterMode::ComposerEmpty)
        },
    );

    snapshot_footer(
        "footer_ctrl_c_quit_idle",
        props(FooterMode::QuitShortcutReminder),
    );

    snapshot_footer(
        "footer_ctrl_c_quit_running",
        props(FooterMode::QuitShortcutReminder),
    );

    snapshot_footer(
        "footer_composer_has_draft_queue_hint_disabled",
        props(FooterMode::ComposerHasDraft),
    );

    snapshot_footer(
        "footer_composer_has_draft_queue_hint_enabled",
        FooterProps {
            task_running: true,
            ..props(FooterMode::ComposerHasDraft)
        },
    );
}

/// Three columns, two (Compose beside the other two), then one.
#[test]
fn the_shortcuts_flow_into_three_two_or_one_column() {
    snapshot_footer_at("shortcut_overlay_140_columns", overlay(), 140);
    snapshot_footer_at("shortcut_overlay_90_columns", overlay(), 90);
    snapshot_footer_at("shortcut_overlay_50_columns", overlay(), 50);
    snapshot_footer_at(
        "shortcut_overlay_while_a_turn_runs",
        FooterProps {
            task_running: true,
            use_shift_enter_hint: true,
            ..overlay()
        },
        140,
    );
}

#[test]
fn the_shortcuts_cut_to_the_height_keep_the_keymap_footer() {
    let height = 8;
    let shown = rows(overlay(), 50, height);
    assert_eq!(shown.len(), usize::from(height));
    assert_eq!(shown[6].trim(), "… more: /keymap");
    assert_eq!(shown[7].trim(), "/keymap customize");
    assert!(footer_height(overlay(), 50) > height);
}

#[test]
fn the_shortcuts_show_the_keys_in_force() {
    install(&[
        ("chat", "cycle_mode", &["f6"]),
        ("chat", "reasoning_up", &[]),
        ("composer", "previous_agent", &[]),
    ]);
    let text = overlay_text(overlay(), 140);
    keymap::install(Keymap::default());
    assert!(text.contains("f6 Change mode"), "{text}");
    assert!(text.contains("Less reasoning"), "{text}");
    assert!(!text.contains("More reasoning"), "{text}");
    assert!(!text.contains("Subagents"), "{text}");
}

#[test]
fn the_shortcuts_list_what_works_here() {
    let text = overlay_text(overlay(), 140);
    for wanted in [
        "Keyboard shortcuts",
        "/ Commands",
        "esc esc Edit last message",
        "shift + tab",
        "⌥ + ,",
        "⌥ + ← / ⌥ + →",
        "Subagents (empty prompt)",
        "Agents (empty prompt)",
        "ctrl + home / ctrl + end",
        "shift + ⌥ + , / shift + ⌥ + .",
        "/keymap customize",
    ] {
        assert!(text.contains(wanted), "{wanted}:\n{text}");
    }
    // No voice; and no `!`, Ctrl+V, Ctrl+G or Ctrl+R until their actions
    // exist.
    for absent in [
        "Voice",
        "f8",
        "Shell command",
        "Paste image",
        "External editor",
        "Search history",
    ] {
        assert!(!text.contains(absent), "{absent}:\n{text}");
    }
    // A second conversation's mode, effort and earlier messages are the
    // main one's; Left there opens nothing.
    let second = overlay_text(
        FooterProps {
            second_conversation: true,
            agents_hint: false,
            ..overlay()
        },
        140,
    );
    for absent in [
        "Change mode",
        "reasoning",
        "Edit last message",
        "Agents (empty",
    ] {
        assert!(!second.contains(absent), "{absent}:\n{second}");
    }
    assert!(second.contains("Subagents (empty prompt)"), "{second}");
}

#[test]
fn shortcut_overlay_toggles_back_to_the_base_mode() {
    let overlay = toggle_shortcut_mode(FooterMode::ComposerEmpty, false, true);
    assert_eq!(overlay, FooterMode::ShortcutOverlay);
    assert_eq!(
        toggle_shortcut_mode(overlay, false, false),
        FooterMode::ComposerHasDraft
    );
    assert_eq!(
        reset_mode_after_activity(FooterMode::QuitShortcutReminder),
        FooterMode::ComposerEmpty
    );
}
