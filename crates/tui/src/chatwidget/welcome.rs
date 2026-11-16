use std::time::Instant;

use ratatui::{buffer::Buffer, layout::Rect};

use super::ChatWidget;

impl ChatWidget {
    pub(crate) fn finish_welcome(&mut self) {
        self.welcome.dismiss();
        self.welcome.hide();
    }

    pub(crate) fn welcome_needs_space(&self) -> bool {
        self.welcome.needs_space() && self.no_modal_or_popup_active() && !self.setup_needed
    }

    pub(crate) fn render_welcome(&mut self, available: Rect, buf: &mut Buffer, ordinary: bool) {
        if !ordinary || !self.no_modal_or_popup_active() || self.setup_needed {
            self.welcome.hide();
            return;
        }
        let empty = self.composer_is_empty();
        if let Some(delay) = self.welcome.render(available, buf, empty, Instant::now()) {
            self.frame_requester.schedule_frame_in(delay);
        }
    }
}
