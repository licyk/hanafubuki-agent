// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use hanafubuki_agent::{AgentAlternateScreen, AgentTuiPreferences};

use super::TranscriptMode;

fn preferences(fullscreen: bool, alternate_screen: AgentAlternateScreen) -> AgentTuiPreferences {
    AgentTuiPreferences {
        fullscreen_transcript: fullscreen,
        alternate_screen,
        raw_output: false,
        ..AgentTuiPreferences::default()
    }
}

#[test]
fn the_defaults_own_the_transcript_except_for_terminal_app_over_ssh() {
    let defaults = AgentTuiPreferences::default();
    assert_eq!(
        TranscriptMode::resolve(&defaults, false, || false),
        TranscriptMode::Owned
    );
    assert_eq!(
        TranscriptMode::resolve(&defaults, false, || true),
        TranscriptMode::Terminal
    );
}

#[test]
fn every_restriction_falls_back_to_the_terminal() {
    use AgentAlternateScreen::{Always, Auto, Never};
    // (fullscreen, alternateScreen, --no-alt-screen, Terminal.app over SSH)
    // -> owned
    let cases = [
        (true, Auto, false, false, true),
        (true, Auto, false, true, false),
        (true, Always, false, true, true),
        (true, Never, false, false, false),
        (true, Always, true, false, false),
        (true, Auto, true, false, false),
        (false, Always, false, false, false),
        (false, Auto, false, false, false),
    ];
    for (fullscreen, screen, no_alt_screen, terminal_app_over_ssh, owned) in cases {
        let mode = TranscriptMode::resolve(&preferences(fullscreen, screen), no_alt_screen, || {
            terminal_app_over_ssh
        });
        assert_eq!(
            mode.is_owned(),
            owned,
            "{fullscreen} {screen:?} {no_alt_screen} {terminal_app_over_ssh}"
        );
    }
}

#[test]
fn the_terminal_is_asked_only_under_auto() {
    use AgentAlternateScreen::{Always, Auto, Never};
    for (fullscreen, screen, no_alt_screen, asked) in [
        (true, Auto, false, true),
        (true, Always, false, false),
        (true, Never, false, false),
        (true, Auto, true, false),
        (false, Auto, false, false),
    ] {
        let mut asked_count = 0;
        TranscriptMode::resolve(&preferences(fullscreen, screen), no_alt_screen, || {
            asked_count += 1;
            false
        });
        assert_eq!(
            asked_count,
            usize::from(asked),
            "{fullscreen} {screen:?} {no_alt_screen}"
        );
    }
}
