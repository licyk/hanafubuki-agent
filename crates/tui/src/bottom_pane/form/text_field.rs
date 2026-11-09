// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A form's single-line field, plain or secret. Each field owns its editing
//! history, its killed text included, so yanking in another field cannot
//! reveal a secret.

use ratatui::crossterm::event::KeyEvent;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;

use crate::bottom_pane::TextArea;

/// How a field's value is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FieldDisplay {
    Focused,
    Unfocused,
    /// Dots, the last character shown while it is typed.
    FocusedSecret,
    UnfocusedSecret,
}

pub(crate) struct TextField {
    editor: TextArea,
    secret: bool,
}

impl TextField {
    pub(crate) fn new(text: &str, secret: bool) -> Self {
        let mut editor = TextArea::new_single_line();
        editor.insert_str(text);
        Self { editor, secret }
    }

    pub(crate) fn input(&mut self, event: KeyEvent) {
        self.editor.input(event);
    }

    /// Pasted text, without its control characters (line breaks included).
    pub(crate) fn paste(&mut self, pasted: &str) {
        let text: String = pasted.chars().filter(|c| !c.is_control()).collect();
        self.editor.insert_str(&text);
    }

    pub(crate) fn text(&self) -> &str {
        self.editor.text()
    }

    pub(crate) fn set_secret(&mut self, secret: bool) {
        self.secret = secret;
    }

    pub(crate) fn is_secret(&self) -> bool {
        self.secret
    }

    pub(crate) fn display(&self, focused: bool) -> FieldDisplay {
        match (focused, self.secret) {
            (true, false) => FieldDisplay::Focused,
            (false, false) => FieldDisplay::Unfocused,
            (true, true) => FieldDisplay::FocusedSecret,
            (false, true) => FieldDisplay::UnfocusedSecret,
        }
    }

    /// The value in `width` columns, scrolled so a focused field's cursor
    /// shows; an empty field shows `placeholder`. The second value is the
    /// cursor's column, for a focused field.
    pub(crate) fn line(
        &self,
        placeholder: &str,
        display: FieldDisplay,
        width: u16,
    ) -> (Line<'static>, Option<u16>) {
        let editor = &self.editor;
        let focused = matches!(display, FieldDisplay::Focused | FieldDisplay::FocusedSecret);
        let mut line = Line::default();
        if editor.text().is_empty() {
            if !placeholder.is_empty() {
                line.push_span(placeholder.to_owned().dim());
            }
            return (line, focused.then_some(0));
        }
        let mut remaining = usize::from(width.max(1));
        let start = if focused {
            editor.single_line_viewport(remaining as u16).0
        } else {
            0
        };
        let mut cursor = None;
        let mut column = 0u16;
        let mut content = editor.text()[start..].grapheme_indices(true).peekable();
        while let Some((offset, grapheme)) = content.next() {
            let cell = match display {
                FieldDisplay::Focused | FieldDisplay::Unfocused => grapheme,
                FieldDisplay::FocusedSecret if content.peek().is_none() => grapheme,
                FieldDisplay::FocusedSecret | FieldDisplay::UnfocusedSecret => "•",
            };
            let span = Span::raw(cell.to_owned());
            let span = if focused { span.cyan() } else { span };
            let Some(rest) = remaining.checked_sub(span.width()) else {
                break;
            };
            remaining = rest;
            if focused && start + offset == editor.cursor() {
                cursor = Some(column);
            }
            column += span.width() as u16;
            line.push_span(span);
        }
        if focused && cursor.is_none() {
            cursor = Some(column);
        }
        (line, cursor)
    }
}
