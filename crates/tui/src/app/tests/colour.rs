//! Colour in command output, end to end on a vt100 screen in both
//! presentations: a command the model ran and one the person typed print a
//! coloured word, which the screen draws in its colour, live and after
//! `/resume`, while the model's next request carries the plain text. The
//! commands force their colour themselves, as `ls --color=always` does: the
//! Agent's command environment is unchanged.

use hanafubuki_agent::test_support::assistant_events;
use ratatui::crossterm::event::KeyCode;
use ratatui::style::Color;

use super::{command_call, Harness, WIDTH};
use crate::transcript_mode::TranscriptMode;

#[cfg(unix)]
const COLOURED: &str = r"printf '\033[31mred\033[0m plain\n'";
#[cfg(windows)]
const COLOURED: &str = r#"[Console]::Out.Write("$([char]27)[31mred$([char]27)[0m plain`n")"#;

const PRESENTATIONS: [TranscriptMode; 2] = [TranscriptMode::Terminal, TranscriptMode::Owned];

impl Harness {
    /// Whether a transcript cell draws `red` in red, before ` plain`.
    fn shows_red(&self) -> bool {
        self.app.transcript_cells.iter().any(|cell| {
            cell.display_lines(WIDTH).iter().any(|line| {
                line.spans.windows(2).any(|pair| {
                    pair[0].content == "red"
                        && line.style.patch(pair[0].style).fg == Some(Color::Red)
                        && pair[1].content == " plain"
                        && line.style.patch(pair[1].style).fg.is_none()
                })
            })
        })
    }

    /// Whether the screen shows a row with `red plain` whose `red` is drawn
    /// in palette colour 1 and ` plain` in the default colour.
    fn screen_shows_red(&self) -> bool {
        let screen = self.tui.terminal.backend().vt100().screen();
        let (rows, cols) = screen.size();
        (0..rows).any(|row| {
            let text = screen.contents_between(row, 0, row, cols);
            let Some(start) = text.find("red plain") else {
                return false;
            };
            let column = text[..start].chars().count() as u16;
            let cell = |offset: u16| screen.cell(row, column + offset).unwrap();
            cell(0).fgcolor() == vt100::Color::Idx(1)
                && cell(2).fgcolor() == vt100::Color::Idx(1)
                && cell(4).fgcolor() == vt100::Color::Default
        })
    }

    fn resume_last_session(&mut self, shown: &str) {
        self.type_and_submit("/new");
        self.pump_until("no session", |harness| {
            harness.app.bridge.session_id().is_none()
        });
        self.type_and_submit("/resume");
        self.pump_until_screen_shows("Resume a previous session");
        self.key(KeyCode::Enter);
        let shown = shown.to_string();
        self.pump_until("the resumed transcript", move |harness| {
            harness.transcript().contains(&shown)
        });
    }

    fn request_text(&self, index: usize) -> String {
        String::from_utf8_lossy(&self.server.requests()[index].body).into_owned()
    }
}

fn assert_plain(body: &str) {
    assert!(body.contains("red plain"), "{body}");
    assert!(!body.contains("\\u001b"), "{body}");
    assert!(!body.contains('\u{1b}'), "{body}");
}

#[test]
fn a_models_coloured_command_shows_its_colour_live_and_after_resume() {
    for mode in PRESENTATIONS {
        let mut harness = Harness::start_with(
            vec![command_call(COLOURED), assistant_events("It printed red.")],
            true,
            mode,
        );
        harness.type_and_submit("Run it.");
        harness.pump_until_screen_shows("Yes, proceed");
        harness.key(KeyCode::Char('y'));
        harness.pump_until_screen_shows("It printed red.");
        harness.pump_until_turn_ends();
        assert!(harness.shows_red(), "{mode:?}: {}", harness.transcript());
        assert!(harness.screen_shows_red(), "{mode:?}: {}", harness.screen());
        assert_plain(&harness.request_text(1));

        harness.resume_last_session("It printed red.");
        assert!(
            harness.shows_red(),
            "{mode:?} resumed: {}",
            harness.transcript()
        );
    }
}

#[test]
fn a_typed_coloured_command_shows_its_colour_and_the_model_reads_it_plain() {
    for mode in PRESENTATIONS {
        let mut harness = Harness::start_with(vec![assistant_events("Seen.")], true, mode);
        harness.type_and_submit(&format!("!{COLOURED}"));
        harness.pump_until("the command's output", |harness| harness.shows_red());
        harness.pump_until_turn_ends();
        assert!(harness.screen_shows_red(), "{mode:?}: {}", harness.screen());
        harness.type_and_submit("What did it print?");
        harness.pump_until_screen_shows("Seen.");
        harness.pump_until_turn_ends();
        let body = harness.request_text(0);
        assert!(body.contains("<user_shell_command>"), "{body}");
        assert_plain(&body);

        harness.resume_last_session("Seen.");
        assert!(
            harness.shows_red(),
            "{mode:?} resumed: {}",
            harness.transcript()
        );
    }
}
