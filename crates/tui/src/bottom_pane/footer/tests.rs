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
        goal: None,
        navigation: None,
        task_running: false,
        agents_hint: false,
        second_conversation: false,
        shell_command: false,
        wsl: false,
        counts: &[],
        second_row: None,
        hits: Box::leak(Box::new(std::cell::RefCell::new(Vec::new()))),
        overflow: Overflow::default(),
    }
}

#[test]
fn goal_is_right_aligned_on_the_hint_row_without_shortening_the_status_line() {
    let mut goal = GoalStatus::default();
    let value =
        serde_json::from_value(serde_json::json!({"objective": "Run Python", "status": "active"}))
            .unwrap();
    goal.update(Some(&value), std::time::Instant::now());
    let status = Line::from("custom status · model · workspace · context · permission mode");
    for status_line in [None, Some(&status)] {
        let props = FooterProps {
            goal: Some(&goal),
            status_line,
            ..props(FooterMode::ComposerEmpty)
        };
        let lines = rows(props, 100, footer_height(props, 100));
        let hint_row = usize::from(status_line.is_some());
        assert!(lines[hint_row].ends_with("Pursuing goal (0s)"), "{lines:?}");
        assert_eq!(lines[hint_row].chars().count(), 100);
        assert!(lines[hint_row].contains("for shortcuts"));
        if status_line.is_some() {
            assert_eq!(lines[0], format!("  {status}"));
        }
        let narrow = rows(props, 24, footer_height(props, 24));
        assert!(
            narrow.last().unwrap().ends_with("Pursuing goal (0s)"),
            "{narrow:?}"
        );
        let without_goal = FooterProps {
            goal: None,
            ..props
        };
        assert!(footer_height(props, 24) > footer_height(without_goal, 24));
        if status_line.is_some() {
            assert_eq!(narrow[0], rows(without_goal, 24, 1)[0]);
            assert_eq!(rows(props, 24, 1), rows(without_goal, 24, 1));
        }
    }
}

#[test]
fn goal_stays_with_the_shortcut_overlay_footer_when_the_list_is_clipped() {
    let mut goal = GoalStatus::default();
    let value =
        serde_json::from_value(serde_json::json!({"objective": "Run Python", "status": "active"}))
            .unwrap();
    goal.update(Some(&value), std::time::Instant::now());
    let status = Line::from("model · workspace · context");
    for status_line in [None, Some(&status)] {
        let props = FooterProps {
            goal: Some(&goal),
            status_line,
            ..overlay()
        };
        let height = footer_height(props, 80);
        assert_eq!(
            height,
            footer_height(
                FooterProps {
                    goal: None,
                    ..props
                },
                80
            )
        );
        for height in [height, 8] {
            let lines = rows(props, 80, height);
            let last = lines.last().unwrap();
            assert!(last.starts_with("  /keymap customize"), "{lines:?}");
            assert!(last.ends_with("Pursuing goal (0s)"), "{lines:?}");
            assert_eq!(
                lines
                    .iter()
                    .filter(|line| line.contains("Pursuing goal"))
                    .count(),
                1
            );
            if status_line.is_some() {
                assert_eq!(lines[0], format!("  {status}"));
            }
        }
    }
}

fn overlay() -> FooterProps<'static> {
    FooterProps {
        agents_hint: true,
        ..props(FooterMode::ShortcutOverlay)
    }
}

/// The counts a session with one terminal, two agents and three warnings
/// would show.
fn counts() -> Vec<crate::appearance::SideCount> {
    use crate::appearance::{SideCount, SideItem};
    vec![
        SideCount::new(SideItem::Terminals, 1, "terminal", "terminals"),
        SideCount::new(SideItem::Agents, 2, "agent", "agents"),
        SideCount::new(SideItem::Warnings, 3, "warning", "warnings"),
    ]
}

#[test]
fn the_counts_sit_at_the_right_edge_of_the_hint_row() {
    let counts = counts();
    let props = FooterProps {
        agents_hint: true,
        counts: &counts,
        ..props(FooterMode::ComposerEmpty)
    };
    let lines = rows(props, 80, footer_height(props, 80));
    let row = lines.last().unwrap();
    assert!(
        row.starts_with("  ← for agents · ? for shortcuts"),
        "{lines:?}"
    );
    assert!(
        row.ends_with("1 terminal · 2 agents · 3 warnings"),
        "{lines:?}"
    );
    assert_eq!(row.chars().count(), 80);
    assert_eq!(lines.len(), 1, "the counts fit beside the hints");
}

#[test]
fn a_row_too_narrow_for_both_sides_follows_the_overflow_rule() {
    // Wide enough for either side alone, not for both.
    let width = 40;
    let counts = counts();
    let at = |overflow: Overflow| {
        let props = FooterProps {
            agents_hint: true,
            counts: &counts,
            overflow,
            ..props(FooterMode::ComposerEmpty)
        };
        rows(props, width, footer_height(props, width))
    };

    // Both sides move: the counts keep their row, the hints follow them.
    let wrapped_all = at(Overflow::WrapAll);
    assert_eq!(wrapped_all.len(), 2, "{wrapped_all:?}");
    assert!(
        wrapped_all[0].starts_with("  ← for agents"),
        "{wrapped_all:?}"
    );
    assert!(
        wrapped_all[1].trim_end().ends_with("3 warnings"),
        "{wrapped_all:?}"
    );

    // The right side keeps the row; the left side goes under it.
    let wrapped_left = at(Overflow::WrapLeft);
    assert_eq!(wrapped_left.len(), 2, "{wrapped_left:?}");
    assert!(
        wrapped_left[0].trim_end().ends_with("3 warnings"),
        "{wrapped_left:?}"
    );
    assert!(
        wrapped_left[1].starts_with("  ← for agents"),
        "{wrapped_left:?}"
    );

    // One row: the hints are cut where the counts begin.
    let truncated = at(Overflow::Truncate);
    assert_eq!(truncated.len(), 1, "{truncated:?}");
    assert!(truncated[0].ends_with("3 warnings"), "{truncated:?}");
    assert_eq!(truncated[0].chars().count(), width as usize);
}

#[test]
fn no_counts_leaves_the_hint_row_as_it_was() {
    let with_none = FooterProps {
        agents_hint: true,
        counts: &[],
        ..props(FooterMode::ComposerEmpty)
    };
    let plain = FooterProps {
        agents_hint: true,
        ..props(FooterMode::ComposerEmpty)
    };
    assert_eq!(
        rows(with_none, 80, footer_height(with_none, 80)),
        rows(plain, 80, footer_height(plain, 80))
    );
}

#[test]
fn each_count_is_a_place_the_pointer_can_land_on() {
    let counts = counts();
    let hits = std::cell::RefCell::new(Vec::new());
    let props = FooterProps {
        agents_hint: true,
        counts: &counts,
        hits: &hits,
        ..props(FooterMode::ComposerEmpty)
    };
    let width = 80;
    let height = footer_height(props, width);
    draw(props, width, height);

    // Three pieces, left to right, each on the counts' row: the row the
    // counts were drawn on, one up from the bottom.
    let hits = hits.into_inner();
    assert_eq!(hits.len(), 3, "{hits:?}");
    assert_eq!(
        hits.iter().map(|(_, item)| *item).collect::<Vec<_>>(),
        [
            crate::appearance::SideItem::Terminals,
            crate::appearance::SideItem::Agents,
            crate::appearance::SideItem::Warnings,
        ]
    );
    for (rect, _) in &hits {
        assert_eq!(rect.y, height - 1, "{hits:?}");
        assert!(
            rect.width > 0 && rect.right() <= width,
            "{rect:?} is not on the row"
        );
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
