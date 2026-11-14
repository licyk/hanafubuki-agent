//! Web links as terminal hyperlinks, end to end over the standalone Agent
//! on a vt100 screen, in both presentations: the bytes the terminal gets
//! hold each link as OSC 8, opened and closed on each row, in prose, a
//! list and a table, while the answer streams and after it ends, after a
//! resize's rewrite and in the transcript overlay; the screen shows the
//! label alone on a terminal that opens links, the address after it on
//! any other. Scrollback's screen is short, so it writes the answer into
//! the terminal's scrollback rather than keep it in its tail.

use hanafubuki_agent::test_support::assistant_events;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::*;
use crate::markdown_render::{preferences, WebLinkDisplay};
use crate::test_support::osc8_links;

const ANSWER: &str = "See [the docs](https://example.com/docs) for the rest.\n\n- [x] Read https://example.com/a/rather/long/path/that/wraps/at/forty/columns\n- [ ] Write\n\n| Name | Where |\n|---|---|\n| guide | [here](https://example.com/guide) |\n\nDone.\n";

const LINKS: [(&str, &str); 3] = [
    ("https://example.com/docs", "the docs"),
    (
        "https://example.com/a/rather/long/path/that/wraps/at/forty/columns",
        "https://example.com/a/rather/long/path/that/wraps/at/forty/columns",
    ),
    ("https://example.com/guide", "here"),
];

const PRESENTATIONS: [TranscriptMode; 2] = [TranscriptMode::Terminal, TranscriptMode::Owned];

impl Harness {
    /// Every byte written to the terminal since the last call.
    fn written_bytes(&mut self) -> String {
        self.tui.terminal.backend_mut().take_written()
    }
}

/// The text the bytes print, escape sequences left out.
fn visible_text(written: &str) -> String {
    let mut text = String::new();
    let mut rest = written;
    while let Some(ch) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix("\x1b]") {
            rest = after.split_once('\x07').map_or("", |(_, after)| after);
        } else if let Some(after) = rest.strip_prefix("\x1b[") {
            let end = after
                .find(|c: char| ('\x40'..='\x7e').contains(&c))
                .map_or(after.len(), |end| end + 1);
            // A cursor move starts another stretch of text.
            if after[..end].ends_with('H') {
                text.push('\n');
            }
            rest = &after[end..];
        } else {
            text.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    text
}

/// Each link's text, its rows joined, as the bytes carry it.
fn linked_text(written: &str) -> Vec<(String, String)> {
    let mut joined: Vec<(String, String)> = Vec::new();
    for (destination, text) in osc8_links(written) {
        match joined.iter_mut().find(|(seen, _)| *seen == destination) {
            Some((_, all)) => all.push_str(&text),
            None => joined.push((destination, text)),
        }
    }
    joined
}

fn assert_links(written: &str, what: &str) {
    let linked = linked_text(written);
    for (destination, label) in LINKS {
        let Some((_, text)) = linked.iter().find(|(seen, _)| seen == destination) else {
            panic!("{what}: no link to {destination} in {linked:?}");
        };
        // A link drawn again by a later frame repeats its text.
        assert!(
            text.replace(' ', "").contains(&label.replace(' ', "")),
            "{what}: {destination} over {text:?}"
        );
    }
}

/// Scrollback's screen: its tail holds less than the answer.
const SHORT: u16 = 14;

fn height(mode: TranscriptMode) -> u16 {
    match mode {
        TranscriptMode::Terminal => SHORT,
        TranscriptMode::Owned => HEIGHT,
    }
}

fn answered(mode: TranscriptMode, display: WebLinkDisplay) -> Harness {
    preferences::set_web_link_display(display);
    let mut harness = Harness::start_with(vec![assistant_events(ANSWER)], true, mode);
    harness.tui.terminal.backend_mut().resize(40, height(mode));
    harness.type_and_submit("Links?");
    harness.pump_until_screen_shows("Done.");
    harness.pump_until_turn_ends();
    harness
}

#[test]
fn links_reach_the_terminal_as_hyperlinks_in_both_presentations() {
    for mode in PRESENTATIONS {
        for display in [WebLinkDisplay::LabelOnly, WebLinkDisplay::WithDestination] {
            let mut harness = answered(mode, display);
            let written = harness.written_bytes();
            assert_links(&written, &format!("{mode:?} {display:?}"));
            if mode == TranscriptMode::Terminal {
                // What left the tail went through the scrollback writer.
                let history = harness.tui.written_history_lines.clone();
                assert!(
                    history
                        .iter()
                        .flat_map(|line| &line.hyperlinks)
                        .any(|link| link.destination == LINKS[0].0),
                    "{display:?}: {history:?}"
                );
            }
            let shown = visible_text(&written);
            let label_only = display == WebLinkDisplay::LabelOnly;
            assert_eq!(
                shown.contains("See the docs for the rest."),
                label_only,
                "{mode:?} {display:?}\n{shown}"
            );
            assert_eq!(
                shown.contains("(https://example.com/docs)"),
                !label_only,
                "{mode:?} {display:?}\n{shown}"
            );
            assert!(!harness.screen().contains("8;;"));
        }
    }
    preferences::set_web_link_display(WebLinkDisplay::WithDestination);
}

#[test]
fn links_survive_the_rewrite_after_a_resize() {
    for mode in PRESENTATIONS {
        let mut harness = answered(mode, WebLinkDisplay::LabelOnly);
        harness.written_bytes();
        harness.tui.terminal.backend_mut().resize(70, height(mode));
        harness.app.draw(&mut harness.tui).unwrap();
        harness.app.reflow.set_due_for_test();
        harness.app.draw(&mut harness.tui).unwrap();
        let written = harness.written_bytes();
        assert_links(&written, &format!("{mode:?} after the resize"));
        // Scrollback writes the answer again; Fullscreen draws what changed.
        let shown = match mode {
            TranscriptMode::Terminal => visible_text(&written),
            TranscriptMode::Owned => harness.screen(),
        };
        assert!(
            shown.contains("See the docs for the rest."),
            "{mode:?}\n{shown}"
        );
    }
    preferences::set_web_link_display(WebLinkDisplay::WithDestination);
}

#[test]
fn the_transcript_overlay_shows_the_links() {
    let mut harness = answered(TranscriptMode::Terminal, WebLinkDisplay::LabelOnly);
    harness.tui.terminal.backend_mut().resize(40, HEIGHT);
    harness.written_bytes();
    harness.key_with(KeyCode::Char('t'), KeyModifiers::CONTROL);
    harness.pump_until_screen_shows("See the docs");
    assert_links(&harness.written_bytes(), "the transcript overlay");
    harness.key(KeyCode::Esc);
    preferences::set_web_link_display(WebLinkDisplay::WithDestination);
}
