//! Ctrl+G end to end, in Scrollback and Fullscreen: the draft goes to the
//! editor (an editor that is a script, `FakeEditor`) and comes back as the
//! draft, which is then sent; the terminal is on the screen it was on with
//! its modes. An unset, missing or failing editor leaves the draft and
//! says why.

use std::time::Duration;

use hanafubuki_agent::test_support::assistant_events;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use vt100::MouseProtocolMode;

use super::Harness;
use crate::external_editor::EditorVariables;
use crate::test_support::FakeEditor;
use crate::transcript_mode::TranscriptMode;

impl Harness {
    fn use_editor(&mut self, visual: Option<String>) {
        self.app.editor = EditorVariables {
            visual,
            editor: None,
        };
    }

    /// Ctrl+G on `draft`, typed as fast as a paste: the key takes what the
    /// burst holds too.
    fn edit_draft(&mut self, draft: &str) {
        for character in draft.chars() {
            self.key(KeyCode::Char(character));
        }
        self.key_with(KeyCode::Char('g'), KeyModifiers::CONTROL);
    }

    fn composer_text(&self) -> String {
        self.app.chat_widget.composer_text()
    }
}

fn the_draft_is_edited_and_sent(mode: TranscriptMode) {
    let mut harness = Harness::start_with(vec![assistant_events("Read it.")], true, mode);
    let editor = FakeEditor::new("edited in the editor\nwith a second line\n".as_bytes(), 0);
    harness.use_editor(Some(editor.command_line()));
    harness.edit_draft("a first draft");
    harness.pump_until("the edited draft", |harness| {
        harness.composer_text() == "edited in the editor\nwith a second line"
    });
    assert_eq!(editor.seen().as_deref(), Some("a first draft"));
    let screen = harness.pump_until_screen_shows("› edited in the editor");
    assert!(screen.contains("with a second line"), "{screen}");
    assert!(
        !screen.contains("Save and close external editor"),
        "{screen}"
    );
    let terminal = harness.tui.terminal.backend().vt100().screen();
    assert_eq!(terminal.alternate_screen(), mode.is_owned());
    assert!(terminal.bracketed_paste());
    // On Windows the pointer is the console's mode, which vt100 does not
    // see.
    if mode.is_owned() && cfg!(not(windows)) {
        assert_eq!(terminal.mouse_protocol_mode(), MouseProtocolMode::AnyMotion);
    }

    std::thread::sleep(Duration::from_millis(100));
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Read it.");
    let requests = harness.server.requests();
    assert_eq!(requests.len(), 1);
    let body = String::from_utf8_lossy(&requests[0].body);
    assert!(
        body.contains("edited in the editor\\nwith a second line"),
        "{body}"
    );
}

#[test]
fn ctrl_g_edits_the_draft_in_the_editor_in_scrollback() {
    the_draft_is_edited_and_sent(TranscriptMode::Terminal);
}

#[test]
fn ctrl_g_edits_the_draft_in_the_editor_in_fullscreen() {
    the_draft_is_edited_and_sent(TranscriptMode::Owned);
}

fn a_failed_edit_keeps_the_draft(mode: TranscriptMode) {
    let mut harness = Harness::start_with(Vec::new(), true, mode);
    harness.use_editor(None);
    harness.edit_draft("kept");
    harness.pump_until_screen_shows("Set VISUAL or EDITOR to edit the message in an editor.");

    harness.use_editor(Some("hanafubuki-agent-no-such-editor --wait".to_string()));
    harness.key_with(KeyCode::Char('g'), KeyModifiers::CONTROL);
    harness.pump_until_screen_shows("The editor `hanafubuki-agent-no-such-editor` was not found");

    let failing = FakeEditor::new(b"never taken", 3);
    harness.use_editor(Some(failing.command_line()));
    harness.key_with(KeyCode::Char('g'), KeyModifiers::CONTROL);
    harness.pump_until_screen_shows("; the draft is unchanged.");
    assert_eq!(failing.seen().as_deref(), Some("kept"));

    assert_eq!(harness.composer_text(), "kept");
    let terminal = harness.tui.terminal.backend().vt100().screen();
    assert_eq!(terminal.alternate_screen(), mode.is_owned());
}

#[test]
fn a_failed_edit_keeps_the_draft_in_scrollback() {
    a_failed_edit_keeps_the_draft(TranscriptMode::Terminal);
}

#[test]
fn a_failed_edit_keeps_the_draft_in_fullscreen() {
    a_failed_edit_keeps_the_draft(TranscriptMode::Owned);
}
