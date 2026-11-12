// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The owned transcript (Codex's `app/owned_transcript.rs`): the session's
//! cells, the live ones changing in place among them, in a transcript view
//! filling the screen above the chat (the bottom pane), and the view's
//! share of the keys.
//!
//! Each frame syncs the view with the cells, paints the view, then the
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
//! it, and the rest to the composer. Before all of these, Esc on an empty
//! composer primes editing an earlier message, which takes the keys while
//! it browses (`backtrack`). With a second conversation open, the screen
//! is shared by panes (`workspace`), each painted as above. Codex's
//! composer selection, turn tips, link hover and keymap are not ported.

use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::backend::Backend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Widget};

use super::{is_transcript_key, App, LIVE_TICK, VIEW_TICK};
use crate::chatwidget::ChatWidget;
use crate::history_cell::HistoryCell;
use crate::render::renderable::Renderable;
use crate::text_selection::is_copy_key;
use crate::transcript_view::TranscriptView;
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
        self.sync_browsing_highlight();
        let size = tui.terminal.size()?;
        if self
            .copy_notice
            .as_ref()
            .is_some_and(|(_, until)| *until <= Instant::now())
        {
            self.copy_notice = None;
        }
        let notice = self.copy_notice.as_ref().map(|(notice, _)| notice.clone());
        let leader = self.chords.leader_pending();
        let layout = self
            .workspace
            .pane_layout(Rect::new(0, 0, size.width, size.height), leader);
        let width = layout.focused.width;
        // Browsing's hints take the view's footer row when it has none.
        let browsing = self
            .backtrack
            .is_browsing()
            .then(|| super::backtrack::browsing_footer(width));
        let dragging = self.workspace.is_dragging();
        let chat_widget = &self.chat_widget;
        let cells = &self.transcript_cells;
        let view = &mut self.transcript_view;
        let (scanning, measured_footer) = prepare_pane(view, cells, width, browsing.is_some());
        let mut passive = layout.passive.zip(self.workspace.parked_mut());
        let passive_animating = passive.as_mut().is_some_and(|(area, parked)| {
            prepare_pane(
                &mut parked.transcript_view,
                &parked.transcript_cells,
                area.width,
                false,
            );
            parked.transcript_view.is_following() && parked.chat_widget.has_live_entries()
        });
        let mut footer_rows = 0;
        tui.draw(u16::MAX, |frame| {
            let buf = frame.buffer_mut();
            let painted = paint_pane(
                buf,
                layout.focused,
                PanePaint {
                    chat_widget,
                    cells,
                    view,
                    notice,
                    browsing,
                    measured_footer,
                },
            );
            footer_rows = painted.footer_rows;
            if let Some((area, parked)) = passive {
                paint_pane(
                    buf,
                    area,
                    PanePaint {
                        chat_widget: &parked.chat_widget,
                        cells: &parked.transcript_cells,
                        view: &mut parked.transcript_view,
                        notice: None,
                        browsing: None,
                        measured_footer: 0,
                    },
                );
            }
            super::workspace::paint_chrome(buf, &layout, dragging);
            if let Some((x, y)) = painted.cursor.filter(|_| layout.hud.is_none()) {
                frame.set_cursor_position((x, y));
            }
        })?;
        self.workspace.remember_layout(layout);
        let view = &mut self.transcript_view;
        let dragging = view.tick_selection(&self.transcript_cells);
        let animating = view.is_following() && self.chat_widget.has_live_entries();
        if footer_rows != measured_footer {
            self.schedule_frame(Instant::now());
        }
        if scanning || dragging {
            self.schedule_frame(Instant::now() + VIEW_TICK);
        }
        if animating || passive_animating {
            self.schedule_frame(Instant::now() + LIVE_TICK);
        }
        if let Some((_, until)) = self.copy_notice {
            self.schedule_frame(until);
        }
        self.chat_widget.reap_failed_views();
        Ok(())
    }
}

/// One conversation's pane, as [`paint_pane`] draws it.
struct PanePaint<'a> {
    chat_widget: &'a ChatWidget,
    cells: &'a [Arc<dyn HistoryCell>],
    view: &'a mut TranscriptView,
    notice: Option<String>,
    browsing: Option<Line<'static>>,
    /// The view's footer rows as measured before the frame.
    measured_footer: usize,
}

/// What a painted pane leaves: the composer's cursor, and the footer rows
/// it drew.
struct Painted {
    cursor: Option<(u16, u16)>,
    footer_rows: usize,
}

/// Syncs a pane's view with its cells at `width`: whether its search still
/// scans, and the footer rows it will need.
fn prepare_pane(
    view: &mut TranscriptView,
    cells: &[Arc<dyn HistoryCell>],
    width: u16,
    browsing: bool,
) -> (bool, usize) {
    view.sync_history_tail(cells);
    view.prepare_width(width);
    let scanning = view.advance_search(cells);
    let measured = view
        .footer(width)
        .map_or(usize::from(browsing), |footer| footer.lines.len());
    (scanning, measured)
}

/// A conversation in `area`: its transcript view, the view's footer, and
/// the chat below; the follow-latest control (or a copy's notice) in the
/// blank row above the composer.
fn paint_pane(buf: &mut Buffer, area: Rect, pane: PanePaint<'_>) -> Painted {
    let PanePaint {
        chat_widget,
        cells,
        view,
        notice,
        browsing,
        measured_footer,
    } = pane;
    let width = area.width;
    let chat_height = chat_widget.desired_height(width).min(area.height);
    let footer_height = u16::try_from(measured_footer)
        .unwrap_or(u16::MAX)
        .min(area.height - chat_height);
    let transcript = Rect {
        height: area.height - chat_height - footer_height,
        ..area
    };
    view.render(transcript, buf, cells);
    let chat_area = Rect::new(area.x, area.bottom() - chat_height, width, chat_height);
    chat_widget.render(chat_area, buf);
    // The chat's first row is the blank above the composer.
    let gap = (chat_height > 1 && chat_widget.no_modal_or_popup_active())
        .then(|| Rect::new(area.x, chat_area.y, width, 1));
    // A copy's notice takes the row from the follow control while it lasts.
    view.render_follow_control(gap.filter(|_| notice.is_none()), buf);
    if let (Some(gap), Some(notice)) = (gap, notice) {
        Line::from(notice).dim().render(gap, buf);
    }
    let footer = view.footer(width);
    let footer_rows = footer
        .as_ref()
        .map_or(usize::from(browsing.is_some()), |footer| footer.lines.len());
    let footer_area = Rect::new(area.x, transcript.bottom(), width, footer_height);
    let mut cursor = chat_widget.cursor_pos(chat_area);
    if let Some(footer) = footer {
        if let Some(column) = footer.cursor_column.filter(|_| footer_height > 0) {
            cursor = Some((footer_area.x + column, footer_area.y));
        }
        Paragraph::new(footer.lines).render(footer_area, buf);
    } else if let Some(browsing) = browsing {
        browsing.render(footer_area, buf);
    }
    Painted {
        cursor,
        footer_rows,
    }
}
