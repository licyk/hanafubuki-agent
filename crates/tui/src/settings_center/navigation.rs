// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The settings page stack as clickable breadcrumbs, and transient save notices.

use std::time::Instant;

use hanafubuki_agent::SettingsCatalog;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use super::{Attach, CenterRequest, Notice, Page, PageOverride, SettingsCenter};
use crate::text_formatting::truncate_text;

impl SettingsCenter {
    pub(crate) fn notice_deadline(&self) -> Option<Instant> {
        match self.notice {
            Notice::Done(_, deadline) => Some(deadline),
            _ => None,
        }
    }

    pub(super) fn expire_notice(&mut self, now: Instant) {
        if self
            .notice_deadline()
            .is_some_and(|deadline| now >= deadline)
        {
            self.notice = Notice::Ready;
        }
    }

    pub(crate) fn handle_breadcrumb(&mut self, mouse: MouseEvent) -> bool {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return false;
        }
        let at = Position::new(mouse.column, mouse.row);
        let Some(&(_, index)) = self
            .layout
            .breadcrumbs
            .iter()
            .find(|(rect, _)| rect.contains(at))
        else {
            return false;
        };
        for page in self.pages.iter().skip(index + 1).rev() {
            if let Page::Hosted(page) = page {
                self.requests
                    .push(CenterRequest::ClosePage(page.command.name));
            }
        }
        self.pages.truncate(index + 1);
        self.layout = Default::default();
        true
    }

    pub(super) fn render_breadcrumbs(
        &self,
        catalog: &SettingsCatalog,
        area: Rect,
        buf: &mut Buffer,
    ) -> (Vec<(Rect, usize)>, u16) {
        let mut hits = Vec::new();
        let mut x = area.x;
        let mut y = area.y;
        for (index, page) in self.pages.iter().enumerate() {
            let title = match page {
                Page::Main(_) => "Settings".to_string(),
                Page::Collection(page) => page.title(catalog),
                Page::Form(page) => page.title(catalog),
                Page::Choice(page) => page.title.clone(),
                Page::List(page) => page.spec.label.clone(),
                Page::Hosted(page) => hosted_title(page, catalog),
            };
            let title = truncate_text(&title, usize::from(area.width));
            let width = title.width() as u16;
            if index > 0 {
                if x.saturating_add(3).saturating_add(width) > area.right() {
                    x = area.x;
                    y = y.saturating_add(1);
                } else {
                    Line::from(" › ".dim()).render(
                        Rect {
                            x,
                            y,
                            width: 3,
                            height: 1,
                        },
                        buf,
                    );
                    x += 3;
                }
            }
            if y >= buf.area.bottom() {
                break;
            }
            let rect = Rect {
                x,
                y,
                width,
                height: 1,
            };
            let title = if index + 1 < self.pages.len() {
                title.cyan().underlined()
            } else {
                title.bold()
            };
            Line::from(title).render(rect, buf);
            hits.push((rect, index));
            x += width;
        }
        (hits, y)
    }
}

fn hosted_title(page: &PageOverride, catalog: &SettingsCatalog) -> String {
    match &page.attach {
        Attach::Row { label, .. } | Attach::Record { label, .. } => (*label).to_string(),
        Attach::Setting(key) => catalog
            .sections
            .iter()
            .flat_map(|section| &section.settings)
            .find(|setting| setting.key == *key)
            .map(|setting| setting.label.clone())
            .unwrap_or_else(|| page.id.to_string()),
        Attach::RecordField {
            collection, field, ..
        } => catalog
            .collections
            .iter()
            .find(|candidate| candidate.id == *collection)
            .and_then(|collection| {
                collection
                    .record
                    .iter()
                    .find(|setting| setting.key == *field)
            })
            .map(|setting| setting.label.clone())
            .unwrap_or_else(|| page.id.to_string()),
    }
}
