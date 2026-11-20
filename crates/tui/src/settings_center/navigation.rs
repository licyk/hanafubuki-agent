// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The settings page stack as clickable breadcrumbs, and what a write or
//! an action answered: the notice under the breadcrumbs, and the page that
//! comes back to the row that opened it.

use std::time::Instant;

use hanafubuki_agent::{SettingApply, SettingsCatalog, SettingsWriteOutcome};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use super::form::FormTarget;
use super::{
    collection, values, Attach, CenterRequest, Notice, Page, PageOverride, Pending, SettingsCenter,
};
use crate::agent_bridge::CatalogOutcome;
use crate::text_formatting::truncate_to_width;

impl SettingsCenter {
    /// What a call answered: a field saved, a record added or an action
    /// run, and what it said.
    pub(super) fn done(&mut self, pending: Pending, outcome: CatalogOutcome) {
        match (pending, outcome) {
            (Pending::Read, _) => {}
            (Pending::Edit(at), outcome) => {
                self.notice = Notice::done(match &outcome {
                    CatalogOutcome::Written(written) => saved_notice(written),
                    _ => "Saved".to_string(),
                });
                match self.pages.get_mut(at) {
                    Some(Page::Main(page)) => page.saved(),
                    // A commit group saved together is done.
                    Some(Page::Form(page)) if matches!(page.target, FormTarget::Group { .. }) => {
                        self.pages.truncate(at)
                    }
                    Some(Page::Form(page)) => page.saved(),
                    _ => {}
                }
            }
            (
                Pending::Action {
                    at,
                    collection,
                    action,
                },
                CatalogOutcome::Acted(acted),
            ) => {
                let message = acted.message.clone();
                // A record added in its form returns to the collection; one
                // added by a page of its own returns there when that page's
                // views end.
                if let Some(Page::Form(form)) = self.pages.get(at) {
                    if matches!(form.target, FormTarget::Add { .. }) {
                        self.pages.truncate(at);
                    }
                }
                if let Some(record) = acted.record.as_deref() {
                    let catalog = self.catalog.as_ref();
                    let found = self.pages.iter().position(
                        |page| matches!(page, Page::Collection(page) if page.id() == collection),
                    );
                    if let Some(Page::Collection(page)) =
                        found.and_then(|at| self.pages.get_mut(at))
                    {
                        page.select_record(catalog, record);
                    }
                }
                self.notice = Notice::done(match message {
                    Some(message) => message,
                    None => match action.as_str() {
                        "save" => "Saved".to_string(),
                        "add" => "Added".to_string(),
                        "delete" => "Deleted".to_string(),
                        _ => format!("Done: {action}"),
                    },
                });
            }
            (Pending::Action { .. }, _) => {}
        }
    }

    /// Why a call failed: in the field that was edited, and under the
    /// breadcrumbs.
    pub(super) fn failed(&mut self, pending: Pending, error: String) {
        match pending {
            Pending::Read => {}
            Pending::Edit(at) => match self.pages.get_mut(at) {
                Some(Page::Main(page)) => page.refused(&error),
                Some(Page::Form(page)) => page.refused(&error),
                _ => {}
            },
            Pending::Action { .. } => {}
        }
        self.notice = Notice::Failed(format!("Could not save: {error}"));
    }

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
            if let Page::Hosted { command, .. } = page {
                self.requests.push(CenterRequest::ClosePage(command));
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
                Page::Hosted { title, .. } => title.clone(),
            };
            let title = truncate_to_width(&title, usize::from(area.width));
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

pub(super) fn hosted_title(page: &PageOverride, catalog: &SettingsCatalog) -> String {
    match &page.attach {
        Attach::Row { label, .. } | Attach::Record { label, .. } => (*label).to_string(),
        Attach::Setting(key) => catalog
            .sections
            .iter()
            .flat_map(|section| &section.settings)
            .find(|setting| setting.key == *key)
            .map(|setting| setting.label.clone())
            .unwrap_or_else(|| page.id.to_string()),
        Attach::Add { collection } => catalog
            .collections
            .iter()
            .find(|candidate| candidate.id == *collection)
            .and_then(|collection| {
                collection
                    .actions
                    .iter()
                    .find(|action| action.id == collection::ADD)
            })
            .map(|action| action.label.clone())
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

/// What a write said: saved, and whether a layer above the user's file
/// keeps another value, or when it applies.
fn saved_notice(written: &SettingsWriteOutcome) -> String {
    let mut notice = "Saved".to_string();
    if let Some(source) = written
        .shadowed
        .iter()
        .find_map(|(_, source)| values::source_text(source))
    {
        notice.push_str(&format!(" · the {source} layer overrides it"));
    }
    let apply = written.apply.iter().map(|(_, apply)| *apply);
    if apply.clone().any(|apply| apply == SettingApply::NextLaunch) {
        notice.push_str(" · applies at next launch");
    } else if apply
        .into_iter()
        .any(|apply| apply == SettingApply::NextSession)
    {
        notice.push_str(" · applies from the next session");
    }
    notice
}
