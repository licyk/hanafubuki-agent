// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::*;
use insta::assert_snapshot;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn snapshot_footer(name: &str, props: FooterProps<'_>) {
    let height = footer_height(props).max(1);
    let mut terminal = Terminal::new(TestBackend::new(80, height)).unwrap();
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, f.area().width, height);
            render_footer(area, f.buffer_mut(), props);
        })
        .unwrap();
    assert_snapshot!(name, terminal.backend());
}

fn props(mode: FooterMode) -> FooterProps<'static> {
    FooterProps {
        mode,
        use_shift_enter_hint: false,
        quit_shortcut_key: key_hint::ctrl(KeyCode::Char('c')),
        status_line: None,
        task_running: false,
        agents_hint: false,
    }
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
        "footer_shortcuts_shift_and_esc",
        FooterProps {
            use_shift_enter_hint: true,
            ..props(FooterMode::ShortcutOverlay)
        },
    );

    snapshot_footer(
        "footer_shortcuts_while_a_turn_runs",
        FooterProps {
            task_running: true,
            ..props(FooterMode::ShortcutOverlay)
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
