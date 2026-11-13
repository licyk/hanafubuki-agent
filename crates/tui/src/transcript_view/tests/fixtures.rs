// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Cells and drawing helpers the transcript view's tests share.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use hanafubuki_agent::AgentToolDisplay;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::history_cell::{self, HistoryCell, LiveCell};
use crate::transcript_view::TranscriptView;

/// A cell of plain lines that counts how often it is laid out.
#[derive(Debug)]
pub(crate) struct TestCell {
    pub(crate) text: String,
    pub(crate) renders: Arc<AtomicUsize>,
}

impl HistoryCell for TestCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.renders.fetch_add(1, Ordering::Relaxed);
        self.text
            .lines()
            .map(|line| line.to_owned().into())
            .collect()
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.display_lines(80)
    }
}

pub(crate) fn cell(text: impl Into<String>) -> Arc<dyn HistoryCell> {
    Arc::new(TestCell {
        text: text.into(),
        renders: Arc::default(),
    })
}

/// An activity's lines, running (its spinner turns) until it is done.
#[derive(Debug)]
pub(crate) struct Activity {
    pub(crate) text: String,
    pub(crate) done: bool,
}

impl HistoryCell for Activity {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.text
            .lines()
            .map(|line| line.to_owned().into())
            .collect()
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.display_lines(80)
    }

    fn transcript_animation_tick(&self) -> Option<u64> {
        (!self.done).then_some(0)
    }
}

/// A running activity that changes in place.
pub(crate) fn live(text: &str) -> Arc<LiveCell<Activity>> {
    LiveCell::new(Activity {
        text: text.to_owned(),
        done: false,
    })
}

pub(crate) fn prompt(text: &str) -> Arc<dyn HistoryCell> {
    Arc::new(history_cell::new_user_prompt(&text.into()))
}

/// A finished tool call whose output is `lines` numbered lines.
pub(crate) fn tool_call(call_id: &str, lines: usize) -> Arc<dyn HistoryCell> {
    let mut cell = history_cell::new_tool_call(call_id.to_string(), "shell".to_string(), None);
    cell.finish(
        call_id,
        AgentToolDisplay::Text {
            text: (1..=lines)
                .map(|line| format!("out {line}\n"))
                .collect::<String>(),
        },
    );
    Arc::new(cell)
}

pub(crate) fn render(
    view: &mut TranscriptView,
    cells: &[Arc<dyn HistoryCell>],
    width: u16,
    height: u16,
) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    view.render(area, &mut buffer, cells);
    buffer
}

/// The buffer's rows, each without its trailing blanks.
pub(crate) fn text(buffer: &Buffer) -> String {
    (buffer.area.top()..buffer.area.bottom())
        .map(|y| {
            (buffer.area.left()..buffer.area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub(crate) fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

pub(crate) fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// Types `query` into an open search and scans until it settles.
pub(crate) fn find(view: &mut TranscriptView, cells: &[Arc<dyn HistoryCell>], query: &str) {
    view.begin_search();
    view.paste_search(query);
    while view.advance_search(cells) {}
}
