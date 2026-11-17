// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The unified @ and $ pickers sit above the composer, with its normal footer
//! below. Slash completion, history search and editor hints keep their own
//! below-composer presentation.

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
    fn layout_areas(&self, area: Rect) -> [Rect; 4] {
        let total = self.popup_or_footer_height(area.width);
        let above = matches!(
            self.active_popup,
            ActivePopup::Mention(_) | ActivePopup::Skill(_)
        );
        let (composer, popup, footer) = if above {
            let footer_height = footer_height(self.footer_props(), area.width);
            let [mut composer, mut popup, footer] = Layout::vertical([
                Constraint::Min(3),
                Constraint::Max(total.saturating_sub(footer_height)),
                Constraint::Length(footer_height),
            ])
            .areas(area);
            popup.y = area.y;
            composer.y = popup.bottom();
            (composer, popup, footer)
        } else {
            let [composer, popup] =
                Layout::vertical([Constraint::Min(3), Constraint::Max(total)]).areas(area);
            (composer, popup, popup)
        };
        let textarea = composer.inset(Insets::tlbr(1, LIVE_PREFIX_COLS, 1, 1));
        [composer, textarea, popup, footer]
    }

    /// The rows under the text: the history search's line, the active
    /// popup's, or the footer's.
    fn popup_or_footer_height(&self, width: u16) -> u16 {
        if self.history_search_active() || self.external_editor_open {
            return 1 + u16::from(self.status_line.is_some());
        }
        match &self.active_popup {
            ActivePopup::None => footer_height(self.footer_props(), width),
            ActivePopup::Command(popup) => popup.calculate_required_height(width),
            ActivePopup::Mention(popup) => {
                popup.calculate_required_height(width) + footer_height(self.footer_props(), width)
            }
            ActivePopup::Skill(popup) => {
                popup.calculate_required_height(width) + footer_height(self.footer_props(), width)
            }
        }
    }

    pub(super) fn footer_props(&self) -> FooterProps<'_> {
        let mode = self.footer_mode();
        FooterProps {
            mode,
            use_shift_enter_hint: self.use_shift_enter_hint,
            quit_shortcut_key: self.quit_shortcut_key,
            status_line: self.status_line.as_ref(),
            navigation: self.navigation_footer.as_deref(),
            task_running: self.is_task_running,
            agents_hint: self.agents_navigation_available(),
            second_conversation: self.command_scope.is_some(),
            shell_command: self.textarea.text().starts_with('!'),
            wsl: self.wsl,
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
        let [_, textarea_rect, mut popup_rect, _] = self.layout_areas(area);
        if self.history_search_active() {
            popup_rect.y += u16::from(self.status_line.is_some());
            popup_rect.height = popup_rect
                .height
                .saturating_sub(u16::from(self.status_line.is_some()));
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
        let [composer_rect, textarea_rect, popup_rect, footer_rect] = self.layout_areas(area);
        match &self.active_popup {
            ActivePopup::Command(popup) => popup.render(popup_rect, buf),
            ActivePopup::Mention(popup) => popup.render(popup_rect, buf),
            ActivePopup::Skill(popup) => popup.render(popup_rect, buf),
            ActivePopup::None => match self
                .history_search_line()
                .or_else(|| self.external_editor_line())
            {
                Some(line) => {
                    let mut hints = popup_rect;
                    if let Some(status) = &self.status_line {
                        render_footer_line(hints, buf, status.clone());
                        hints.y += 1;
                        hints.height = hints.height.saturating_sub(1);
                    }
                    render_footer_line(hints, buf, line);
                }
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
        if matches!(
            self.active_popup,
            ActivePopup::Mention(_) | ActivePopup::Skill(_)
        ) {
            render_footer(footer_rect, buf, self.footer_props());
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
