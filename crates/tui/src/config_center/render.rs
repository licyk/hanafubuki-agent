//! The configuration center on the screen: the search line, the domains
//! with their effective values, the layers and warnings, and the keys.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

use super::{ConfigCenter, ConfigSection};
use crate::key_hint;
use crate::text_formatting::truncate_text;
use ratatui::crossterm::event::KeyCode;

const LABEL_WIDTH: usize = 30;

impl ConfigCenter {
    pub(crate) fn render(&mut self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        let [title, search, notice, list, hint] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);
        self.view_rows = (list.height > 0).then_some(usize::from(list.height));
        let rows = self.rows();
        self.ensure_selected_visible(rows.len());

        title_line(self).render(title, buf);
        search_line(self).render(search, buf);
        notice_line(self).render(notice, buf);
        render_rows(self, &rows, list, buf);
        hint_line().render(hint, buf);
    }
}

fn title_line(center: &ConfigCenter) -> Line<'static> {
    let mut spans = vec!["Agent settings".bold().cyan(), "   ".into()];
    match center.state_summary() {
        Ok(summary) => spans.push(summary.dim()),
        Err(error) => spans.push(error.red()),
    }
    Line::from(spans)
}

fn search_line(center: &ConfigCenter) -> Line<'static> {
    let query = center.query.clone();
    Line::from(vec![
        "Search: ".bold(),
        query.into(),
        if center.search_focused {
            "▏".cyan()
        } else {
            " ".into()
        },
    ])
}

fn notice_line(center: &ConfigCenter) -> Line<'static> {
    if center.state.is_err() {
        return Line::from("The configuration could not be read.".red());
    }
    match center.selected_command() {
        Some(command) => Line::from(format!("Enter opens {command}.").dim()),
        None => Line::from("This domain's form is coming in the next phase.".dim()),
    }
}

fn render_rows(center: &ConfigCenter, rows: &[ConfigSection], area: Rect, buf: &mut Buffer) {
    if rows.is_empty() {
        Line::from("No settings match the search.".italic().dim()).render(area, buf);
        return;
    }
    for (offset, section) in rows
        .iter()
        .enumerate()
        .skip(center.scroll_top)
        .take(usize::from(area.height))
    {
        let y = area.y + u16::try_from(offset - center.scroll_top).unwrap_or(0);
        row_line(center, *section, offset == center.selected)
            .render(Rect::new(area.x, y, area.width, 1), buf);
    }
}

fn row_line(center: &ConfigCenter, section: ConfigSection, selected: bool) -> Line<'static> {
    let marker = if selected { "› ".bold() } else { "  ".into() };
    let label = truncate_text(section.label(), LABEL_WIDTH);
    let padding = " ".repeat(LABEL_WIDTH.saturating_sub(label.chars().count()));
    let summary = center.section_summary(section);
    let mut label_span: Span<'static> = label.into();
    if selected {
        label_span = label_span.bold();
    }
    Line::from(vec![
        marker,
        label_span,
        padding.into(),
        "  ".into(),
        summary.dim(),
    ])
}

fn hint_line() -> Line<'static> {
    let mut spans = Vec::new();
    for (key, label) in [
        (key_hint::plain(KeyCode::Char('/')), " search"),
        (key_hint::plain(KeyCode::Enter), " open"),
        (key_hint::plain(KeyCode::Up), " up"),
        (key_hint::plain(KeyCode::Down), " down"),
        (key_hint::plain(KeyCode::Esc), " close"),
    ] {
        if !spans.is_empty() {
            spans.push("  ".into());
        }
        spans.push(key.into());
        spans.push(label.dim());
    }
    Line::from(spans)
}
