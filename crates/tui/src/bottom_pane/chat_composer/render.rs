// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! How the composer is drawn: the text under its `›` prompt, and under it
//! the active popup, the history search's line, the external editor's, or
//! the footer with the mode the composer's state selects.

use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::StatefulWidget;
use ratatui::widgets::Widget;

use super::ActivePopup;
use super::ChatComposer;
use crate::bottom_pane::footer::footer_height;
use crate::bottom_pane::footer::render_footer;
use crate::bottom_pane::footer::render_footer_line;
use crate::bottom_pane::footer::FooterMode;
use crate::bottom_pane::footer::FooterProps;
use crate::render::renderable::Renderable;
use crate::render::Insets;
use crate::render::RectExt;
use crate::style::user_message_style;
use crate::ui_consts::LIVE_PREFIX_COLS;

impl ChatComposer {
    fn layout_areas(&self, area: Rect) -> [Rect; 3] {
        let popup_constraint = Constraint::Max(self.popup_or_footer_height(area.width));
        let [composer_rect, popup_rect] =
            Layout::vertical([Constraint::Min(3), popup_constraint]).areas(area);
        let textarea_rect = composer_rect.inset(Insets::tlbr(1, LIVE_PREFIX_COLS, 1, 1));
        [composer_rect, textarea_rect, popup_rect]
    }

    /// The rows under the text: the history search's line, the active
    /// popup's, or the footer's.
    fn popup_or_footer_height(&self, width: u16) -> u16 {
        if self.history_search_active() || self.external_editor_open {
            return 1;
        }
        match &self.active_popup {
            ActivePopup::None => footer_height(self.footer_props(), width),
            ActivePopup::Command(popup) => popup.calculate_required_height(width),
            ActivePopup::File(popup) => popup.calculate_required_height(),
        }
    }

    pub(super) fn footer_props(&self) -> FooterProps<'_> {
        let mode = self.footer_mode();
        // A draft written during a turn keeps the row clear.
        let shows_status_line = mode == FooterMode::ComposerEmpty
            || (mode == FooterMode::ComposerHasDraft && !self.is_task_running);
        FooterProps {
            mode,
            use_shift_enter_hint: self.use_shift_enter_hint,
            quit_shortcut_key: self.quit_shortcut_key,
            status_line: self.status_line.as_ref().filter(|_| shows_status_line),
            task_running: self.is_task_running,
            agents_hint: self.agents_navigation_available(),
            second_conversation: self.command_scope.is_some(),
            shell_command: self.textarea.text().starts_with('!'),
        }
    }

    /// Resolve the effective footer mode via a small priority waterfall.
    ///
    /// The base mode is derived solely from whether the composer is empty:
    /// `ComposerEmpty` iff empty, otherwise `ComposerHasDraft`. Transient
    /// modes (overlay, quit reminder) can override that base when their
    /// conditions are active.
    pub(super) fn footer_mode(&self) -> FooterMode {
        let base_mode = if self.is_empty() {
            FooterMode::ComposerEmpty
        } else {
            FooterMode::ComposerHasDraft
        };

        match self.footer_mode {
            FooterMode::ShortcutOverlay => FooterMode::ShortcutOverlay,
            _ if self.quit_shortcut_hint_visible() => FooterMode::QuitShortcutReminder,
            FooterMode::EscHint if self.is_empty() => FooterMode::EscHint,
            FooterMode::QuitShortcutReminder
            | FooterMode::ComposerEmpty
            | FooterMode::ComposerHasDraft
            | FooterMode::EscHint => base_mode,
        }
    }
}

impl Renderable for ChatComposer {
    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        if !self.has_focus {
            return None;
        }
        let [_, textarea_rect, popup_rect] = self.layout_areas(area);
        if self.history_search_active() {
            return self.history_search_cursor_pos(popup_rect);
        }
        let state = *self.textarea_state.borrow();
        self.textarea.cursor_pos_with_state(textarea_rect, state)
    }

    fn desired_height(&self, width: u16) -> u16 {
        const COLS_WITH_MARGIN: u16 = LIVE_PREFIX_COLS + 1;
        self.textarea
            .desired_height(width.saturating_sub(COLS_WITH_MARGIN))
            + 2
            + self.popup_or_footer_height(width)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let [composer_rect, textarea_rect, popup_rect] = self.layout_areas(area);
        match &self.active_popup {
            ActivePopup::Command(popup) => popup.render(popup_rect, buf),
            ActivePopup::File(popup) => popup.render(popup_rect, buf),
            ActivePopup::None => match self
                .history_search_line()
                .or_else(|| self.external_editor_line())
            {
                Some(line) => render_footer_line(popup_rect, buf, line),
                None => {
                    // The footer hides while a paste burst buffers, so a pasted `?` does
                    // not flash "? for shortcuts" away and back.
                    let mut props = self.footer_props();
                    if props.mode == FooterMode::ComposerEmpty && self.is_in_paste_burst() {
                        props.mode = FooterMode::ComposerHasDraft;
                    }
                    render_footer(popup_rect, buf, props);
                }
            },
        }
        Block::default()
            .style(user_message_style())
            .render(composer_rect, buf);
        if !textarea_rect.is_empty() {
            buf.set_span(
                textarea_rect.x - LIVE_PREFIX_COLS,
                textarea_rect.y,
                &"›".bold(),
                textarea_rect.width,
            );
        }

        let mut state = self.textarea_state.borrow_mut();
        StatefulWidget::render(&self.textarea, textarea_rect, buf, &mut state);
        if self.textarea.text().is_empty() && !textarea_rect.is_empty() {
            let placeholder = Span::from(self.placeholder_text.clone()).dim();
            Line::from(vec![placeholder]).render(textarea_rect, buf);
        }
    }
}
