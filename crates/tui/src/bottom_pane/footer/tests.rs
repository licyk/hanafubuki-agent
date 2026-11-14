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
        navigation: None,
        update_notice: None,
        task_running: false,
        agents_hint: false,
        second_conversation: false,
        shell_command: false,
        wsl: false,
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

    snapshot_footer(
        "footer_composer_shell_command",
        FooterProps {
            shell_command: true,
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
        ("composer", "history_search", &["f7"]),
        ("composer", "external_editor", &["f9"]),
        ("composer", "paste_image", &["f10"]),
    ]);
    let text = overlay_text(overlay(), 140);
    install(&[
        ("composer", "history_search", &[]),
        ("composer", "external_editor", &[]),
        ("composer", "paste_image", &[]),
    ]);
    let unbound = overlay_text(overlay(), 140);
    keymap::install(Keymap::default());
    assert!(text.contains("f6 Change mode"), "{text}");
    assert!(text.contains("Less reasoning"), "{text}");
    assert!(!text.contains("More reasoning"), "{text}");
    assert!(!text.contains("Subagents"), "{text}");
    assert!(text.contains("f7 Search history"), "{text}");
    assert!(text.contains("f9 External editor"), "{text}");
    assert!(text.contains("f10 Paste image"), "{text}");
    assert!(!unbound.contains("Search history"), "{unbound}");
    assert!(!unbound.contains("External editor"), "{unbound}");
    assert!(!unbound.contains("Paste image"), "{unbound}");
}

#[test]
fn paste_image_shows_ctrl_alt_v_under_wsl() {
    let wsl = FooterProps {
        wsl: true,
        ..overlay()
    };
    let text = overlay_text(wsl, 140);
    assert!(text.contains("ctrl + ⌥ + v Paste image"), "{text}");
    assert!(!text.contains("ctrl + v Paste image"), "{text}");
    // Under WSL with Ctrl+Alt+V unbound, the key in force.
    install(&[("composer", "paste_image", &["ctrl-v"])]);
    let text = overlay_text(wsl, 140);
    keymap::install(Keymap::default());
    assert!(text.contains("ctrl + v Paste image"), "{text}");
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
        "! Shell command",
        "ctrl + v Paste image",
        "ctrl + g External editor",
        "ctrl + r Search history",
    ] {
        assert!(text.contains(wanted), "{wanted}:\n{text}");
    }
    // No voice.
    for absent in ["Voice", "f8", "⌥ + v"] {
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

fn notice() -> TuiUpdateNotice {
    TuiUpdateNotice {
        version: "0.3.0".to_string(),
        action: "hanafubuki-agent update".to_string(),
    }
}

/// The default status line's three items, as a session shows them.
fn status() -> Line<'static> {
    Line::from("gpt-5.1 high · ~/src/hanafubuki · Footer notice")
}

const FULL: &str = "Update available: 0.3.0 · hanafubuki-agent update";
const SHORT: &str = "Update available: 0.3.0";

/// In full at 120 columns, short at 80, not at all at 50, two columns
/// from the right edge; the status line is never cut.
#[test]
fn the_update_notice_takes_the_room_the_row_leaves() {
    let notice = notice();
    let status = status();
    let props = FooterProps {
        status_line: Some(&status),
        update_notice: Some(&notice),
        ..props(FooterMode::ComposerEmpty)
    };
    for width in [120, 80, 50] {
        snapshot_footer_at(&format!("footer_update_notice_{width}"), props, width);
    }
    let shown = |width| rows(props, width, 1).remove(0);
    let wide = shown(120);
    assert!(wide.starts_with("  gpt-5.1 high · ~/src/hanafubuki · Footer notice   "));
    assert!(wide.ends_with(FULL), "{wide}");
    assert_eq!(wide.chars().count(), 118, "{wide}");
    let medium = shown(80);
    assert!(medium.ends_with(SHORT), "{medium}");
    assert!(!medium.contains("hanafubuki-agent update"), "{medium}");
    assert_eq!(
        shown(50),
        "  gpt-5.1 high · ~/src/hanafubuki · Footer notice"
    );
    // The notice adds no row.
    let without = FooterProps {
        update_notice: None,
        ..props
    };
    for width in [120, 80, 50, 10] {
        assert_eq!(footer_height(props, width), footer_height(without, width));
    }
}

/// Each one-row mode keeps its own text whole and shows the notice after
/// it; the shortcuts and a row too full for it do not.
#[test]
fn the_update_notice_sits_beside_every_one_row_mode() {
    let notice = notice();
    let status = status();
    let with_notice = |mode| FooterProps {
        update_notice: Some(&notice),
        ..props(mode)
    };
    let cases = [
        (with_notice(FooterMode::ComposerEmpty), "? for shortcuts"),
        (
            FooterProps {
                agents_hint: true,
                status_line: Some(&status),
                ..with_notice(FooterMode::ComposerEmpty)
            },
            "Footer notice",
        ),
        (
            FooterProps {
                status_line: Some(&status),
                ..with_notice(FooterMode::ComposerHasDraft)
            },
            "Footer notice",
        ),
        (
            FooterProps {
                task_running: true,
                ..with_notice(FooterMode::ComposerHasDraft)
            },
            "to queue message",
        ),
        (
            FooterProps {
                shell_command: true,
                ..with_notice(FooterMode::ComposerHasDraft)
            },
            "runs in the session's sandbox",
        ),
        (
            with_notice(FooterMode::QuitShortcutReminder),
            "again to quit",
        ),
        (
            with_notice(FooterMode::EscHint),
            "again to edit a previous message",
        ),
        (
            FooterProps {
                second_conversation: true,
                status_line: Some(&status),
                ..with_notice(FooterMode::ComposerEmpty)
            },
            "Footer notice",
        ),
    ];
    for (props, own) in cases {
        let row = rows(props, 120, 1).remove(0);
        let (left, right) = row.split_once("   Update available").expect(&row);
        assert!(left.trim_end().ends_with(own), "{row}");
        assert_eq!(format!("Update available{right}"), FULL, "{row}");
    }
    let overlay = overlay_text(
        FooterProps {
            update_notice: Some(&notice),
            ..overlay()
        },
        140,
    );
    assert!(!overlay.contains("Update available"), "{overlay}");
    // A status line that leaves less than the short form's room.
    let long = Line::from("x".repeat(60));
    let full_row = rows(
        FooterProps {
            status_line: Some(&long),
            ..with_notice(FooterMode::ComposerEmpty)
        },
        80,
        1,
    );
    assert_eq!(full_row, [format!("  {}", "x".repeat(60))]);
}

/// The room is counted in columns: a wide character takes two.
#[test]
fn the_update_notice_counts_wide_characters_as_two_columns() {
    let notice = notice();
    // 14 characters, 28 columns: the full form would fit by characters.
    let status = Line::from("会话标题是一个很长的中文标题");
    let props = FooterProps {
        status_line: Some(&status),
        update_notice: Some(&notice),
        ..props(FooterMode::ComposerEmpty)
    };
    snapshot_footer_at("footer_update_notice_wide_characters", props, 80);
    let terminal = draw(props, 80, 1);
    let buffer = terminal.backend().buffer();
    let text: String = (0..80).map(|x| buffer[(x, 0)].symbol()).collect();
    // The cell after a wide character holds a space.
    assert!(
        text.replace(' ', "")
            .starts_with("会话标题是一个很长的中文标题Update"),
        "{text}"
    );
    assert!(text.trim_end().ends_with(SHORT), "{text}");
    assert_eq!(buffer[(77, 0)].symbol(), "0");
    assert_eq!(buffer[(78, 0)].symbol(), " ");
}
