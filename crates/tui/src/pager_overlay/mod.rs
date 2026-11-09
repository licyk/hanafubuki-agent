// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Overlay UIs rendered in an alternate screen.
//!
//! The terminal-mode transcript (`Ctrl+T`, [`transcript`]): Codex's
//! `TranscriptOverlay`, a transcript view over the committed cells with
//! the live tail after them. And the static overlay `/diff` shows its
//! output in, a pager over fixed lines.

mod transcript;

use ratatui::buffer::Buffer;
use ratatui::buffer::Cell;
use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use ratatui::widgets::Wrap;

use crate::key_hint;
use crate::key_hint::KeyBinding;
use crate::keymap::{self, Action, Binding};
use crate::render::renderable::Renderable;
pub(crate) use transcript::TranscriptOverlay;

pub(crate) enum Overlay {
    Transcript(TranscriptOverlay),
    Static(StaticOverlay),
}

impl Overlay {
    pub(crate) fn new_static_with_lines(lines: Vec<Line<'static>>, title: String) -> Self {
        Self::Static(StaticOverlay::with_title(lines, title))
    }

    /// Draws the overlay; where the cursor goes, when it shows one.
    pub(crate) fn render(&mut self, area: Rect, buf: &mut Buffer) -> Option<(u16, u16)> {
        match self {
            Overlay::Transcript(o) => o.render(area, buf),
            Overlay::Static(o) => {
                o.render(area, buf);
                None
            }
        }
    }

    pub(crate) fn is_done(&self) -> bool {
        match self {
            Overlay::Transcript(o) => o.is_done(),
            Overlay::Static(o) => o.is_done(),
        }
    }
}

// Common pager navigation hints rendered on the first line
const PAGER_KEY_HINTS: &[(&[Action], &str)] = &[
    (&[Action::ScrollUp, Action::ScrollDown], "to scroll"),
    (&[Action::PageUp, Action::PageDown], "to page"),
    (&[Action::JumpTop, Action::JumpBottom], "to jump"),
];

/// Render a single line of key hints from (actions, description) pairs, the
/// keys as the keymap has them: the first key of each of several actions,
/// or every key of one.
fn render_key_hints(area: Rect, buf: &mut Buffer, pairs: &[(&[Action], &str)]) {
    let keymap = keymap::current();
    let mut spans: Vec<Span<'static>> = vec![" ".into()];
    let mut first = true;
    for (actions, desc) in pairs {
        let keys: Vec<KeyBinding> = match actions {
            [action] => keymap
                .bindings(*action)
                .iter()
                .filter_map(|binding| match binding {
                    Binding::Key(key) => Some(key.hint()),
                    _ => None,
                })
                .collect(),
            actions => actions
                .iter()
                .filter_map(|action| keymap.hint(*action))
                .collect(),
        };
        if keys.is_empty() {
            continue;
        }
        if !first {
            spans.push("   ".into());
        }
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                spans.push("/".into());
            }
            spans.push(Span::from(key));
        }
        spans.push(" ".into());
        spans.push(Span::from(desc.to_string()));
        first = false;
    }
    Paragraph::new(vec![Line::from(spans).dim()]).render(area, buf);
}

/// The pager action `key` triggers, of those the pagers take.
fn pager_action(key: &KeyEvent) -> Option<Action> {
    [
        Action::Close,
        Action::CloseTranscript,
        Action::ScrollUp,
        Action::ScrollDown,
        Action::PageUp,
        Action::PageDown,
        Action::HalfPageUp,
        Action::HalfPageDown,
        Action::JumpTop,
        Action::JumpBottom,
    ]
    .into_iter()
    .find(|action| keymap::matches(*action, key))
}

/// Generic widget for rendering a pager view.
struct PagerView {
    renderables: Vec<Box<dyn Renderable>>,
    scroll_offset: usize,
    title: String,
    last_content_height: Option<usize>,
    last_rendered_height: Option<usize>,
}

impl PagerView {
    fn new(renderables: Vec<Box<dyn Renderable>>, title: String, scroll_offset: usize) -> Self {
        Self {
            renderables,
            scroll_offset,
            title,
            last_content_height: None,
            last_rendered_height: None,
        }
    }

    fn content_height(&self, width: u16) -> usize {
        self.renderables
            .iter()
            .map(|c| c.desired_height(width) as usize)
            .sum()
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        self.render_header(area, buf);
        let content_area = self.content_area(area);
        self.update_last_content_height(content_area.height);
        let content_height = self.content_height(content_area.width);
        self.last_rendered_height = Some(content_height);
        self.scroll_offset = self
            .scroll_offset
            .min(content_height.saturating_sub(content_area.height as usize));

        self.render_content(content_area, buf);

        self.render_bottom_bar(area, content_area, buf, content_height);
    }

    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        Span::from("/ ".repeat(area.width as usize / 2))
            .dim()
            .render(area, buf);
        let header = format!("/ {}", self.title);
        header.dim().render(area, buf);
    }

    fn render_content(&self, area: Rect, buf: &mut Buffer) {
        let mut y = -(self.scroll_offset as isize);
        let mut drawn_bottom = area.y;
        for renderable in &self.renderables {
            let top = y;
            let height = renderable.desired_height(area.width) as isize;
            y += height;
            let bottom = y;
            if bottom < area.y as isize {
                continue;
            }
            if top > area.y as isize + area.height as isize {
                break;
            }
            if top < 0 {
                let drawn = render_offset_content(area, buf, &**renderable, (-top) as u16);
                drawn_bottom = drawn_bottom.max(area.y + drawn);
            } else {
                let draw_height = (height as u16).min(area.height.saturating_sub(top as u16));
                let draw_area = Rect::new(area.x, area.y + top as u16, area.width, draw_height);
                renderable.render(draw_area, buf);
                drawn_bottom = drawn_bottom.max(draw_area.y.saturating_add(draw_area.height));
            }
        }

        for y in drawn_bottom..area.bottom() {
            if area.width == 0 {
                break;
            }
            buf[(area.x, y)] = Cell::from('~');
            for x in area.x + 1..area.right() {
                buf[(x, y)] = Cell::from(' ');
            }
        }
    }

    fn render_bottom_bar(
        &self,
        full_area: Rect,
        content_area: Rect,
        buf: &mut Buffer,
        total_len: usize,
    ) {
        let sep_y = content_area.bottom();
        let sep_rect = Rect::new(full_area.x, sep_y, full_area.width, 1);

        Span::from("─".repeat(sep_rect.width as usize))
            .dim()
            .render(sep_rect, buf);
        let percent = if total_len == 0 {
            100
        } else {
            let max_scroll = total_len.saturating_sub(content_area.height as usize);
            if max_scroll == 0 {
                100
            } else {
                (((self.scroll_offset.min(max_scroll)) as f32 / max_scroll as f32) * 100.0).round()
                    as u8
            }
        };
        let pct_text = format!(" {percent}% ");
        let pct_w = pct_text.chars().count() as u16;
        let pct_x = sep_rect.x + sep_rect.width - pct_w - 1;
        Span::from(pct_text)
            .dim()
            .render(Rect::new(pct_x, sep_rect.y, pct_w, 1), buf);
    }

    fn handle_key_event(&mut self, key_event: KeyEvent, viewport_area: Rect) {
        let half_page = || {
            let area = self.content_area(viewport_area);
            (area.height as usize).saturating_add(1) / 2
        };
        match pager_action(&key_event) {
            Some(Action::ScrollUp) => self.scroll_offset = self.scroll_offset.saturating_sub(1),
            Some(Action::ScrollDown) => self.scroll_offset = self.scroll_offset.saturating_add(1),
            Some(Action::PageUp) => {
                let page_height = self.page_height(viewport_area);
                self.scroll_offset = self.scroll_offset.saturating_sub(page_height);
            }
            Some(Action::PageDown) => {
                let page_height = self.page_height(viewport_area);
                self.scroll_offset = self.scroll_offset.saturating_add(page_height);
            }
            Some(Action::HalfPageDown) => {
                self.scroll_offset = self.scroll_offset.saturating_add(half_page());
            }
            Some(Action::HalfPageUp) => {
                self.scroll_offset = self.scroll_offset.saturating_sub(half_page());
            }
            Some(Action::JumpTop) => self.scroll_offset = 0,
            Some(Action::JumpBottom) => self.scroll_offset = usize::MAX,
            _ => {}
        }
    }

    /// Returns the height of one page in content rows.
    ///
    /// Prefers the last rendered content height (excluding header/footer chrome);
    /// if no render has occurred yet, falls back to the content area height
    /// computed from the given viewport.
    fn page_height(&self, viewport_area: Rect) -> usize {
        self.last_content_height
            .unwrap_or_else(|| self.content_area(viewport_area).height as usize)
    }

    fn update_last_content_height(&mut self, height: u16) {
        self.last_content_height = Some(height as usize);
    }

    fn content_area(&self, area: Rect) -> Rect {
        let mut area = area;
        area.y = area.y.saturating_add(1);
        area.height = area.height.saturating_sub(2);
        area
    }
}

/// A renderable that caches its desired height.
struct CachedRenderable {
    renderable: Box<dyn Renderable>,
    height: std::cell::Cell<Option<u16>>,
    last_width: std::cell::Cell<Option<u16>>,
}

impl CachedRenderable {
    fn new(renderable: impl Into<Box<dyn Renderable>>) -> Self {
        Self {
            renderable: renderable.into(),
            height: std::cell::Cell::new(None),
            last_width: std::cell::Cell::new(None),
        }
    }
}

impl Renderable for CachedRenderable {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.renderable.render(area, buf);
    }
    fn desired_height(&self, width: u16) -> u16 {
        if self.last_width.get() != Some(width) {
            let height = self.renderable.desired_height(width);
            self.height.set(Some(height));
            self.last_width.set(Some(width));
        }
        self.height.get().unwrap_or(0)
    }
}

pub(crate) struct StaticOverlay {
    view: PagerView,
    is_done: bool,
}

impl StaticOverlay {
    pub(crate) fn with_title(lines: Vec<Line<'static>>, title: String) -> Self {
        let paragraph = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
        Self {
            view: PagerView::new(vec![Box::new(CachedRenderable::new(paragraph))], title, 0),
            is_done: false,
        }
    }

    fn render_hints(&self, area: Rect, buf: &mut Buffer) {
        let line1 = Rect::new(area.x, area.y, area.width, 1);
        let line2 = Rect::new(area.x, area.y.saturating_add(1), area.width, 1);
        render_key_hints(line1, buf, PAGER_KEY_HINTS);
        render_key_hints(line2, buf, &[(&[Action::Close], "to quit")]);
    }

    pub(crate) fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let top_h = area.height.saturating_sub(3);
        let top = Rect::new(area.x, area.y, area.width, top_h);
        let bottom = Rect::new(area.x, area.y + top_h, area.width, 3);
        self.view.render(top, buf);
        self.render_hints(bottom, buf);
    }

    /// Handles a key; `viewport` is the screen the overlay is drawn on.
    pub(crate) fn handle_key_event(&mut self, key_event: KeyEvent, viewport: Rect) {
        if is_ctrl_c(&key_event) || keymap::matches(Action::Close, &key_event) {
            self.is_done = true;
        } else {
            self.view.handle_key_event(key_event, viewport);
        }
    }

    pub(crate) fn is_done(&self) -> bool {
        self.is_done
    }
}

/// Ctrl-C closes a pager, whatever the keymap says.
fn is_ctrl_c(key: &KeyEvent) -> bool {
    key_hint::ctrl(KeyCode::Char('c')).is_press(*key)
}

fn render_offset_content(
    area: Rect,
    buf: &mut Buffer,
    renderable: &dyn Renderable,
    scroll_offset: u16,
) -> u16 {
    let height = renderable.desired_height(area.width);
    let mut tall_buf = Buffer::empty(Rect::new(
        0,
        0,
        area.width,
        height.min(area.height + scroll_offset),
    ));
    renderable.render(*tall_buf.area(), &mut tall_buf);
    let copy_height = area
        .height
        .min(tall_buf.area().height.saturating_sub(scroll_offset));
    for y in 0..copy_height {
        let src_y = y + scroll_offset;
        for x in 0..area.width {
            buf[(area.x + x, area.y + y)] = tall_buf[(x, src_y)].clone();
        }
    }

    copy_height
}

#[cfg(test)]
mod tests;
