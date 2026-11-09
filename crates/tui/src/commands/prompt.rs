// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A one-line prompt over the composer (Codex's `CustomPromptView`, one
//! line instead of several): a title, the field, a note under it, and a
//! suggestion Tab takes. Enter submits what is typed, Esc cancels. The
//! command that shows it may hand it its replies (`/rename`'s title and
//! suggestion).

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};

use super::CommandReply;
use crate::app_event::AppEventSender;
use crate::bottom_pane::{BottomPaneView, CancellationEvent, TextField};
use crate::key_hint;
use crate::render::renderable::Renderable;

/// What Enter does with the trimmed text, which is not empty.
pub(crate) type PromptSubmitted = Box<dyn Fn(&AppEventSender, String)>;

/// How the command's view takes a reply: the view, and the reply it hands
/// back when it is not its.
pub(crate) type PromptReplies = fn(&mut PromptView, CommandReply) -> Option<CommandReply>;

/// The field's indent under the title.
const INDENT: u16 = 2;

pub(crate) struct PromptView {
    title: String,
    placeholder: String,
    field: TextField,
    note: Option<String>,
    suggestion: Option<String>,
    app_event_tx: AppEventSender,
    on_submit: PromptSubmitted,
    on_reply: Option<PromptReplies>,
    done: bool,
}

impl PromptView {
    pub(crate) fn new(
        title: impl Into<String>,
        placeholder: impl Into<String>,
        app_event_tx: AppEventSender,
        on_submit: PromptSubmitted,
    ) -> Self {
        Self {
            title: title.into(),
            placeholder: placeholder.into(),
            field: TextField::new("", false),
            note: None,
            suggestion: None,
            app_event_tx,
            on_submit,
            on_reply: None,
            done: false,
        }
    }

    pub(crate) fn with_replies(mut self, on_reply: PromptReplies) -> Self {
        self.on_reply = Some(on_reply);
        self
    }

    /// Replaces what is typed.
    pub(crate) fn set_text(&mut self, text: &str) {
        self.field = TextField::new(text, false);
    }

    pub(crate) fn text(&self) -> &str {
        self.field.text()
    }

    /// The line under the field.
    pub(crate) fn set_note(&mut self, note: Option<String>) {
        self.note = note;
    }

    /// What Tab puts into the field; shown under it until then.
    pub(crate) fn set_suggestion(&mut self, suggestion: Option<String>) {
        self.suggestion = suggestion;
    }

    fn lines(&self, width: u16) -> (Vec<Line<'static>>, Option<u16>) {
        let mut lines = vec![Line::from(self.title.clone().bold())];
        let (field, cursor) = self.field.line(
            &self.placeholder,
            self.field.display(true),
            width.saturating_sub(INDENT),
        );
        let mut field_line = Line::from("  ");
        field_line.spans.extend(field.spans);
        lines.push(field_line);
        match (&self.suggestion, &self.note) {
            (Some(suggestion), _) => lines.push(Line::from(vec![
                "  Suggested: ".dim(),
                suggestion.clone().cyan(),
                "  ".into(),
                key_hint::plain(KeyCode::Tab).into(),
                " to use it".dim(),
            ])),
            (None, Some(note)) => lines.push(Line::from(format!("  {note}").dim())),
            (None, None) => {}
        }
        lines.push(Line::from(vec![
            "  ".into(),
            key_hint::plain(KeyCode::Enter).into(),
            " to confirm   ".dim(),
            key_hint::plain(KeyCode::Esc).into(),
            " to cancel".dim(),
        ]));
        (lines, cursor)
    }
}

impl BottomPaneView for PromptView {
    fn handle_key_event(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        match key.code {
            KeyCode::Esc => self.done = true,
            KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
                let text = self.field.text().trim().to_string();
                if !text.is_empty() {
                    (self.on_submit)(&self.app_event_tx, text);
                    self.done = true;
                }
            }
            KeyCode::Tab => {
                if let Some(suggestion) = self.suggestion.take() {
                    self.set_text(&suggestion);
                }
            }
            _ => self.field.input(key),
        }
    }

    fn is_complete(&self) -> bool {
        self.done
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        self.field.paste(&pasted);
        true
    }

    fn take_command_reply(&mut self, reply: CommandReply) -> Option<CommandReply> {
        match self.on_reply {
            Some(on_reply) => on_reply(self, reply),
            None => Some(reply),
        }
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.done = true;
        CancellationEvent::Handled
    }
}

impl Renderable for PromptView {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(self.lines(area.width).0).render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.lines(width).0.len() as u16
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        let column = self.lines(area.width).1?;
        (area.height > 1).then(|| (area.x + INDENT + column, area.y + 1))
    }
}
