//! A patch's diff end to end on a vt100 screen, in both presentations: the
//! model updates two places of a file; expanded, its rows are numbered from
//! where each chunk landed and tinted to the right edge (a 24-bit dark
//! terminal), live and after `/resume`; the model's next request carries no
//! positions.

use hanafubuki_agent::test_support::{assistant_events, tool_events};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::Harness;
use crate::diff_render::{
    with_diff_style_context, DiffColorLevel, DiffRenderStyleContext, DiffTheme,
};
use crate::render::highlight::DiffScopeBackgrounds;
use crate::transcript_mode::TranscriptMode;

const PRESENTATIONS: [TranscriptMode; 2] = [TranscriptMode::Terminal, TranscriptMode::Owned];
const ORIGINAL: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\n";
/// The diff's rows, numbered: `six` is line 6 before the patch and 7 after
/// the first chunk added a line.
const NUMBERED: [&str; 8] = [
    "2  two", "3 -three", "3 +THREE", "4 +3", "  ⋮", "7  six", "7 -seven", "8 +SEVEN",
];
const ADDED_TINT: vt100::Color = vt100::Color::Rgb(33, 58, 43);

impl Harness {
    /// Opens the transcript's details: Ctrl-T once in place, twice in the
    /// Scrollback presentation's overlay (compact first).
    fn open_details(&mut self, mode: TranscriptMode) {
        self.key_with(KeyCode::Char('t'), KeyModifiers::CONTROL);
        if mode == TranscriptMode::Terminal {
            self.pump_until_screen_shows("T R A N S C R I P T");
            self.key_with(KeyCode::Char('t'), KeyModifiers::CONTROL);
        }
    }

    /// Whether the screen's row holding `text` is tinted from its first
    /// column to its last.
    fn row_is_tinted(&self, text: &str, tint: vt100::Color) -> bool {
        let screen = self.tui.terminal.backend().vt100().screen();
        let (rows, cols) = screen.size();
        (0..rows).any(|row| {
            let contents = screen.contents_between(row, 0, row, cols);
            let Some(start) = contents.find(text) else {
                return false;
            };
            let column = contents[..start].chars().count() as u16;
            let bg = |column: u16| screen.cell(row, column).unwrap().bgcolor();
            bg(column) == tint && bg(cols - 1) == tint
        })
    }
}

fn patch_call(file: &std::path::Path) -> Vec<u8> {
    let patch = format!(
        "*** Begin Patch\n*** Update File: {}\n@@\n two\n-three\n+THREE\n+3\n@@\n six\n-seven\n+SEVEN\n*** End Patch",
        file.display()
    );
    tool_events(
        "apply_patch",
        &serde_json::json!({ "patch": patch }).to_string(),
    )
}

#[test]
fn an_applied_patch_shows_numbered_tinted_rows_live_and_after_resume() {
    let truecolor = DiffRenderStyleContext::new(
        DiffTheme::Dark,
        DiffColorLevel::TrueColor,
        DiffScopeBackgrounds::default(),
    );
    with_diff_style_context(truecolor, || {
        for mode in PRESENTATIONS {
            let directory = tempfile::tempdir().unwrap();
            let file = directory.path().join("notes.txt");
            std::fs::write(&file, ORIGINAL).unwrap();
            let mut harness = Harness::start_with(
                vec![patch_call(&file), assistant_events("Patched.")],
                true,
                mode,
            );
            harness.type_and_submit("Patch it.");
            harness.pump_until_screen_shows("Yes, proceed");
            harness.key(KeyCode::Char('y'));
            harness.pump_until_screen_shows("Patched.");
            harness.pump_until_turn_ends();
            assert_eq!(
                std::fs::read_to_string(&file).unwrap(),
                "one\ntwo\nTHREE\n3\nfour\nfive\nsix\nSEVEN\n"
            );
            let body: serde_json::Value =
                serde_json::from_slice(&harness.server.requests()[1].body).unwrap();
            let output = body["input"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["type"] == "function_call_output")
                .unwrap()
                .to_string();
            assert!(output.contains("modified"), "{output}");
            assert!(
                !output.contains("Line") && !output.contains("hunk"),
                "{output}"
            );

            harness.open_details(mode);
            let screen = harness.pump_until_screen_shows("8 +SEVEN");
            for row in NUMBERED {
                assert!(screen.contains(row), "{mode:?}: {row:?}\n{screen}");
            }
            assert!(
                harness.row_is_tinted("3 +THREE", ADDED_TINT),
                "{mode:?}: {screen}"
            );
            harness.key(KeyCode::Esc);

            harness.type_and_submit("/new");
            harness.pump_until("no session", |harness| {
                harness.app.bridge.session_id().is_none()
            });
            harness.type_and_submit("/resume");
            harness.pump_until_screen_shows("Resume a previous session");
            harness.key(KeyCode::Enter);
            harness.pump_until("the resumed transcript", |harness| {
                harness.transcript().contains("Patched.")
            });
            harness.open_details(mode);
            let resumed = harness.pump_until_screen_shows("8 +SEVEN");
            assert!(resumed.contains("7 -seven"), "{mode:?} resumed: {resumed}");
        }
    });
}
