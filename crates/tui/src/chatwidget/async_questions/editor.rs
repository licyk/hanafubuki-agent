// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

use std::{cell::RefCell, ops::Range, rc::Rc};

use ratatui::{
    buffer::Buffer,
    crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    layout::Rect,
    style::Stylize,
    text::Line,
    widgets::{Paragraph, StatefulWidget, Widget},
};

use super::{Pending, State};
use crate::bottom_pane::{BottomPaneView, CancellationEvent, TextAreaState};
use crate::keymap::{self, Action};
use crate::render::renderable::Renderable;
use crate::text_formatting::strip_terminal_controls;
use crate::wrapping::{word_wrap_lines, RtOptions};

/// Geometry of the last frame also tells which part of a long suggestion
/// the person could read before it is submitted in their name.
#[derive(Default)]
struct Viewport {
    id: String,
    width: u16,
    top: usize,
    height: usize,
    rows: Vec<Range<usize>>,
    seen: Vec<usize>,
    list: Rect,
    input: Rect,
    focus: Option<usize>,
}

pub(super) struct Editor {
    state: Rc<RefCell<State>>,
    viewport: RefCell<Viewport>,
    done: bool,
    other_shortcut: Option<KeyCode>,
}

impl Editor {
    pub(super) fn new(state: Rc<RefCell<State>>) -> Self {
        Self {
            state,
            viewport: Default::default(),
            done: false,
            other_shortcut: None,
        }
    }

    fn select(&mut self, index: usize) {
        let mut state = self.state.borrow_mut();
        let i = state.index;
        if let Some(q) = state.pending.get_mut(i) {
            q.selected = index.min(q.options.len());
            self.viewport.get_mut().focus = Some(q.selected);
        }
    }

    fn submit(&mut self, queue: bool) {
        let mut state = self.state.borrow_mut();
        let Some(q) = state.pending.get(state.index) else {
            return;
        };
        let vp = self.viewport.get_mut();
        if q.selected < q.options.len() {
            let Some(lines) = vp.rows.get(q.selected).filter(|_| vp.id == q.id) else {
                return;
            };
            if vp.seen[q.selected] < lines.len() {
                vp.top = lines.start + vp.seen[q.selected];
                vp.focus = None;
                return;
            }
        }
        state.submit = Some(queue);
    }

    fn leave(&mut self) {
        self.state.borrow_mut().expanded = false;
        self.done = true;
    }

    fn layout(q: &Pending, width: u16) -> (Vec<Line<'static>>, Vec<Range<usize>>) {
        let mut lines = Vec::new();
        crate::markdown::append_markdown(
            &q.title,
            Some(usize::from(width.max(1))),
            None,
            &mut lines,
        );
        lines.push(Line::default());
        let mut rows = Vec::new();
        for (index, option) in q
            .options
            .iter()
            .map(String::as_str)
            .chain((!q.options.is_empty()).then_some("Your own answer"))
            .enumerate()
        {
            let start = lines.len();
            let selected = index == q.selected;
            let mut prefix = format!("{} {}. ", if selected { "›" } else { " " }, index + 1);
            if prefix.chars().count() >= usize::from(width) {
                prefix.clear();
            }
            let opts = RtOptions::new(usize::from(width.max(1)))
                .initial_indent(Line::from(prefix.clone()))
                .subsequent_indent(Line::from(" ".repeat(prefix.chars().count())));
            let wrapped = word_wrap_lines(
                option.split('\n').map(|part| Line::from(part.to_owned())),
                opts,
            );
            lines.extend(
                wrapped
                    .into_iter()
                    .map(|line| if selected { line.cyan() } else { line }),
            );
            rows.push(start..lines.len());
        }
        (lines, rows)
    }

    fn scroll(&mut self, down: bool) {
        let vp = self.viewport.get_mut();
        let step = vp.height.saturating_sub(1).max(1);
        vp.top = if down {
            vp.top.saturating_add(step)
        } else {
            vp.top.saturating_sub(step)
        };
        vp.focus = None;
    }
}

impl BottomPaneView for Editor {
    fn captures_keys(&self) -> bool {
        true
    }
    fn is_complete(&self) -> bool {
        self.done || self.state.borrow().pending.is_empty()
    }
    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.leave();
        CancellationEvent::Handled
    }
    fn handle_paste(&mut self, pasted: String) -> bool {
        let mut state = self.state.borrow_mut();
        let i = state.index;
        let Some(q) = state.pending.get_mut(i) else {
            return false;
        };
        q.selected = q.options.len();
        q.draft.insert_str(&strip_terminal_controls(&pasted));
        self.viewport.get_mut().focus = Some(q.selected);
        true
    }
    fn handle_key_event(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release
            || (key.kind == KeyEventKind::Repeat && self.other_shortcut == Some(key.code))
        {
            return;
        }
        let mut state = self.state.borrow_mut();
        let i = state.index;
        self.other_shortcut = None;
        let len = state.pending.len();
        let Some(q) = state.pending.get_mut(i) else {
            return;
        };
        let free = q.selected == q.options.len();
        let next = q.selected.saturating_add(1) % (q.options.len() + 1);
        let prev = q.selected.checked_sub(1).unwrap_or(q.options.len());
        if keymap::matches(Action::EditQueuedMessage, &key) {
            if i + 1 < len {
                state.index += 1;
            } else {
                drop(state);
                self.leave()
            }
            return;
        }
        let queue = keymap::matches(Action::Queue, &key);
        if queue || keymap::matches(Action::Submit, &key) {
            drop(state);
            if key.kind == KeyEventKind::Press {
                self.submit(queue);
            }
            return;
        }
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {
                drop(state);
                self.leave()
            }
            (KeyCode::BackTab, _) => {
                state.index = i.saturating_sub(1);
            }
            (KeyCode::Char('s'), KeyModifiers::CONTROL) if key.kind == KeyEventKind::Press => {
                state.remove_current();
            }
            (KeyCode::PageDown, _) => {
                drop(state);
                self.scroll(true)
            }
            (KeyCode::PageUp, _) => {
                drop(state);
                self.scroll(false)
            }
            (KeyCode::Down, KeyModifiers::NONE) if !q.options.is_empty() => {
                drop(state);
                self.select(next)
            }
            (KeyCode::Up, KeyModifiers::NONE) if !q.options.is_empty() => {
                drop(state);
                self.select(prev)
            }
            (KeyCode::Char(c @ '1'..='9'), KeyModifiers::NONE)
                if !free && usize::from(c as u8 - b'1') <= q.options.len() =>
            {
                if key.kind != KeyEventKind::Press {
                    return;
                }
                let index = usize::from(c as u8 - b'1');
                let other = index == q.options.len();
                drop(state);
                self.select(index);
                if other {
                    self.other_shortcut = Some(key.code);
                } else {
                    self.submit(false);
                }
            }
            (KeyCode::Char(_), mods)
                if !free
                    && (matches!(mods, KeyModifiers::NONE | KeyModifiers::SHIFT)
                        || crate::key_hint::is_altgr(mods)) =>
            {
                q.selected = q.options.len();
                self.viewport.get_mut().focus = Some(q.selected);
                q.draft.input(key);
            }
            _ if free => q.draft.input(key),
            _ => {}
        }
    }
    fn pointer_key(&mut self, mouse: MouseEvent) -> Option<KeyCode> {
        let vp = self.viewport.get_mut();
        if !vp.list.contains((mouse.column, mouse.row).into()) {
            return None;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => Some(KeyCode::PageUp),
            MouseEventKind::ScrollDown => Some(KeyCode::PageDown),
            MouseEventKind::Down(MouseButton::Left) => {
                let row = vp.top + usize::from(mouse.row - vp.list.y);
                let i = vp.rows.iter().position(|range| range.contains(&row))?;
                self.select(i);
                Some(KeyCode::Null)
            }
            _ => None,
        }
    }
}

impl Renderable for Editor {
    fn desired_height(&self, width: u16) -> u16 {
        let state = self.state.borrow();
        let Some(q) = state.pending.get(state.index) else {
            return 0;
        };
        let (lines, _) = Self::layout(q, width);
        let draft = if q.selected == q.options.len() {
            q.draft.desired_height(width.max(1))
        } else {
            0
        };
        u16::try_from(lines.len())
            .unwrap_or(u16::MAX)
            .saturating_add(draft)
            .saturating_add(2)
    }
    fn cursor_pos(&self, _: Rect) -> Option<(u16, u16)> {
        let state = self.state.borrow();
        let q = state.pending.get(state.index)?;
        let area = self.viewport.borrow().input;
        (!area.is_empty() && q.selected == q.options.len())
            .then(|| q.draft.cursor_pos(area))
            .flatten()
    }
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let state = self.state.borrow();
        let Some(q) = state.pending.get(state.index) else {
            return;
        };
        if area.is_empty() {
            return;
        }
        Line::from(format!(
            "Question {} of {}",
            state.index + 1,
            state.pending.len()
        ))
        .bold()
        .render(Rect { height: 1, ..area }, buf);
        let body = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(2),
        );
        let input_height = if q.selected == q.options.len() {
            q.draft
                .desired_height(body.width.max(1))
                .max(1)
                .min(body.height / 2 + body.height % 2)
        } else {
            0
        };
        let list = Rect {
            height: body.height - input_height,
            ..body
        };
        let input = Rect::new(body.x, list.bottom(), body.width, input_height);
        let (lines, rows) = Self::layout(q, list.width);
        let mut vp = self.viewport.borrow_mut();
        if vp.id != q.id || vp.width != area.width {
            *vp = Viewport {
                id: q.id.clone(),
                width: area.width,
                seen: vec![0; rows.len()],
                ..Default::default()
            };
        }
        vp.list = list;
        vp.input = input;
        vp.height = usize::from(list.height);
        if let Some(range) = vp.focus.take().and_then(|i| rows.get(i)) {
            if range.start < vp.top || range.end > vp.top.saturating_add(vp.height) {
                vp.top = range.start;
            }
        }
        vp.top = vp.top.min(lines.len().saturating_sub(vp.height));
        let end = vp.top + vp.height;
        let top = vp.top;
        for (i, range) in rows.iter().enumerate() {
            let next = range.start + vp.seen[i];
            if top <= next && next < end {
                vp.seen[i] += lines[next..range.end.min(end)]
                    .iter()
                    .take_while(|line| line.width() <= usize::from(list.width))
                    .count();
            }
        }
        vp.rows = rows;
        Paragraph::new(
            lines
                .into_iter()
                .skip(top)
                .take(vp.height)
                .collect::<Vec<_>>(),
        )
        .render(list, buf);
        if !input.is_empty() {
            StatefulWidget::render(&q.draft, input, buf, &mut TextAreaState::default());
        }
        if q.selected == q.options.len() && q.draft.is_empty() {
            Line::from("Type your answer").dim().render(input, buf);
        }
        if area.height > 1 {
            Line::from(format!(
                "{} send · {} queue · PgUp/PgDn scroll · Ctrl-S skip · Esc back",
                keymap::current().label(Action::Submit),
                keymap::current().label(Action::Queue)
            ))
            .dim()
            .render(Rect::new(area.x, area.bottom() - 1, area.width, 1), buf);
        }
    }
}
