// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The owned transcript (Codex's `app/owned_transcript.rs`): the session's
//! cells and the live tail in a transcript view filling the screen above
//! the chat (the bottom pane), and the view's share of the keys.
//!
//! Each frame syncs the live cells into the view, paints the view, then the
//! view's footer and the chat below it, and puts the follow-latest control
//! (or, for a few seconds, the last copy's notice) in the blank row above
//! the composer.
//!
//! Keys: while a popup, an approval or a question is open, the chat has
//! them but for a copy of the view's selection. Otherwise the view's
//! selection, search and activity focus take theirs first; Ctrl+T toggles
//! the detailed presentation (Esc leaves it); plain Enter on an empty
//! composer returns a scrolled view to the latest output; the view's own
//! keys (paging, the jumps, F3, F4, Ctrl+Space, Esc while scrolled) go to
//! it, and the rest to the composer. Codex's composer selection, turn tips,
//! sidebar, side pane, link hover and keymap are not ported.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use ratatui::backend::Backend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};

use super::{is_transcript_key, App, LIVE_TICK, VIEW_TICK};
use crate::render::renderable::Renderable;
use crate::text_selection::is_copy_key;
use crate::tui::Tui;

/// How long a copy's notice stays.
pub(super) const COPY_NOTICE: Duration = Duration::from_secs(3);

impl App {
    /// Takes the alternate screen for the run, the pointer with it.
    pub(crate) fn enter_owned_screen<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) -> io::Result<()> {
        tui.enter_alt_screen(true)
    }

    /// `true` when the view (or the presentation toggle) took `key`.
    pub(super) fn handle_owned_key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        let view = &mut self.transcript_view;
        let cells = &self.transcript_cells;
        if !self.chat_widget.no_modal_or_popup_active() {
            if !(is_copy_key(key) && view.owns_interaction_key(key)) {
                return false;
            }
        } else if !view.has_active_interaction() {
            if is_transcript_key(key) {
                view.set_presentation(!view.is_detailed(), self.render_mode);
                return true;
            }
            if view.is_detailed() && key.code == KeyCode::Esc && key.modifiers.is_empty() {
                view.set_presentation(false, self.render_mode);
                return true;
            }
            if key.code == KeyCode::Enter
                && key.modifiers == KeyModifiers::NONE
                && self.chat_widget.composer_is_empty()
                && view.can_return_to_latest()
            {
                view.jump_to_latest();
                return true;
            }
        }
        let Some(action) = view.handle_key(key, cells) else {
            return false;
        };
        self.apply_view_action(action);
        true
    }

    pub(super) fn draw_owned<B: Backend<Error = io::Error> + Write>(
        &mut self,
        tui: &mut Tui<B>,
    ) -> io::Result<()> {
        self.load_older_pages();
        let width = tui.terminal.size()?.width;
        if self
            .copy_notice
            .as_ref()
            .is_some_and(|(_, until)| *until <= Instant::now())
        {
            self.copy_notice = None;
        }
        let notice = self.copy_notice.as_ref().map(|(notice, _)| notice.clone());
        let chat_widget = &self.chat_widget;
        let cells = &self.transcript_cells;
        let view = &mut self.transcript_view;
        view.sync_history_tail(cells);
        view.prepare_width(width);
        let scanning = view.advance_search(cells);
        let live_key = chat_widget.active_cell_transcript_key();
        view.sync_live_cells(cells, width, live_key, &chat_widget.active_cells());
        let measured_footer = view.footer(width).map_or(0, |footer| footer.lines.len());
        let chat_height = chat_widget.desired_height(width);
        let mut footer_rows = 0;
        tui.draw(u16::MAX, |frame| {
            let area = frame.area();
            let chat_height = chat_height.min(area.height);
            let footer_height = u16::try_from(measured_footer)
                .unwrap_or(u16::MAX)
                .min(area.height - chat_height);
            let transcript = Rect {
                height: area.height - chat_height - footer_height,
                ..area
            };
            view.render(transcript, frame.buffer_mut(), cells);
            let chat_area = Rect::new(area.x, area.bottom() - chat_height, area.width, chat_height);
            chat_widget.render(chat_area, frame.buffer_mut());
            // The chat's first row is the blank above the composer.
            let gap = (chat_height > 1 && chat_widget.no_modal_or_popup_active())
                .then(|| Rect::new(area.x, chat_area.y, area.width, 1));
            // A copy's notice takes the row from the follow control while
            // it lasts.
            view.render_follow_control(gap.filter(|_| notice.is_none()), frame.buffer_mut());
            if let (Some(gap), Some(notice)) = (gap, notice) {
                Line::from(notice).dim().render(gap, frame.buffer_mut());
            }
            let footer = view.footer(width);
            footer_rows = footer.as_ref().map_or(0, |footer| footer.lines.len());
            let footer_area = Rect::new(area.x, transcript.bottom(), area.width, footer_height);
            let mut cursor = chat_widget.cursor_pos(chat_area);
            if let Some(footer) = footer {
                if let Some(column) = footer.cursor_column.filter(|_| footer_height > 0) {
                    cursor = Some((footer_area.x + column, footer_area.y));
                }
                Paragraph::new(footer.lines).render(footer_area, frame.buffer_mut());
            }
            if let Some((x, y)) = cursor {
                frame.set_cursor_position((x, y));
            }
        })?;
        let view = &mut self.transcript_view;
        let dragging = view.tick_selection(&self.transcript_cells);
        let animating =
            view.is_following() && live_key.is_some_and(|key| key.animation_tick.is_some());
        if footer_rows != measured_footer {
            self.schedule_frame(Instant::now());
        }
        if scanning || dragging {
            self.schedule_frame(Instant::now() + VIEW_TICK);
        }
        if animating {
            self.schedule_frame(Instant::now() + LIVE_TICK);
        }
        if let Some((_, until)) = self.copy_notice {
            self.schedule_frame(until);
        }
        self.chat_widget.reap_failed_views();
        Ok(())
    }
}
