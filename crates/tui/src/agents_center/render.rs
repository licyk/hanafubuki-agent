// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The command center on the screen: the filters, the sessions with their
//! children under them, the confirmation or the last notice, and the keys.

use std::time::{SystemTime, UNIX_EPOCH};

use hanafubuki_agent::{AgentSessionOrigin, AgentSessionSummary};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

use super::{AgentsCenter, CenterFilter, Row, SessionStatus};
use crate::key_hint;
use crate::text_formatting::truncate_text;

/// The widest a title grows before the status column.
const TITLE_WIDTH: usize = 48;

impl AgentsCenter {
    pub(crate) fn render(&mut self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        let [header, notice, list, hint] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);
        self.view_rows = (list.height > 0).then_some(usize::from(list.height));
        let rows = self.rows();
        self.ensure_selected_visible(rows.len());
        self.header_line().render(header, buf);
        self.notice_line().render(notice, buf);
        self.render_rows(&rows, list, buf);
        hint_line().render(hint, buf);
    }

    fn header_line(&self) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec!["Agents".bold().cyan(), "   ".into()];
        for (index, filter) in CenterFilter::ALL.into_iter().enumerate() {
            if index > 0 {
                spans.push(" · ".dim());
            }
            let label = format!("{} {}", index + 1, filter.label());
            spans.push(if filter == self.filter {
                label.bold().reversed()
            } else {
                label.dim()
            });
        }
        Line::from(spans)
    }

    fn notice_line(&self) -> Line<'static> {
        if self.confirm_delete.is_some() {
            return Line::from(vec![
                "Delete the session and its children? ".magenta().bold(),
                "y".bold(),
                " deletes, any other key keeps it".dim(),
            ]);
        }
        match (&self.notice, &self.error) {
            (Some(notice), _) => Line::from(notice.clone().magenta()),
            (None, Some(error)) => Line::from(error.clone().red()),
            (None, None) => Line::from(""),
        }
    }

    fn render_rows(&self, rows: &[Row], area: Rect, buf: &mut Buffer) {
        if rows.is_empty() {
            let empty = if !self.loaded {
                "Loading…"
            } else if self.filter == CenterFilter::All {
                "No sessions yet."
            } else {
                "No sessions match this filter."
            };
            Line::from(empty.italic().dim()).render(area, buf);
            return;
        }
        let now = now_millis();
        for (offset, row) in rows
            .iter()
            .enumerate()
            .skip(self.scroll_top)
            .take(usize::from(area.height))
        {
            let y = area.y + u16::try_from(offset - self.scroll_top).unwrap_or(0);
            let line = self.row_line(row, offset == self.selected, now);
            line.render(Rect::new(area.x, y, area.width, 1), buf);
        }
    }

    fn row_line(&self, row: &Row, selected: bool, now: u128) -> Line<'static> {
        let summary = &row.summary;
        let marker = if selected { "› ".bold() } else { "  ".into() };
        let indent = if row.child { "  └ " } else { "" };
        let title_width = TITLE_WIDTH.saturating_sub(indent.chars().count());
        let title = truncate_text(&title_of(summary), title_width);
        let pad = " ".repeat(title_width.saturating_sub(title.chars().count()));
        let mut title_span: Span<'static> = title.into();
        if selected {
            title_span = title_span.bold();
        }
        let status = SessionStatus::of(summary);
        let status_style = match status {
            SessionStatus::Running => Style::new().cyan(),
            SessionStatus::Idle => Style::new().dim(),
            SessionStatus::Failed => Style::new().red(),
        };
        let mut spans = vec![
            marker,
            indent.dim(),
            title_span,
            pad.into(),
            "  ".into(),
            Span::styled(format!("{:<8}", status.label()), status_style),
        ];
        if let Some(origin) = summary.origin.map(origin_label) {
            spans.push(format!("{origin:<9}").dim());
        }
        spans.push(ago(now, summary.updated_at).dim());
        if Some(summary.session_id) == self.current {
            spans.push("  (shown)".magenta());
        }
        Line::from(spans)
    }
}

fn hint_line() -> Line<'static> {
    let mut spans = Vec::new();
    for (key, label) in [
        (key_hint::plain(KeyCode::Enter), " open"),
        (key_hint::plain(KeyCode::Char('n')), " new"),
        (key_hint::plain(KeyCode::Char('f')), " fork"),
        (key_hint::plain(KeyCode::Char('a')), " archive"),
        (key_hint::plain(KeyCode::Char('d')), " delete"),
        (key_hint::plain(KeyCode::Char('i')), " interrupt"),
        (key_hint::plain(KeyCode::Tab), " filter"),
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

fn title_of(summary: &AgentSessionSummary) -> String {
    summary
        .title
        .as_ref()
        .map_or_else(|| "(untitled)".to_string(), |title| title.value.clone())
}

fn origin_label(origin: AgentSessionOrigin) -> &'static str {
    match origin {
        AgentSessionOrigin::Side => "side",
        AgentSessionOrigin::Subagent => "subagent",
        AgentSessionOrigin::Review => "review",
    }
}

/// How long ago `then` was, in its largest unit.
fn ago(now: u128, then: u128) -> String {
    let seconds = now.saturating_sub(then) / 1000;
    match seconds {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", seconds / 60),
        3600..=86_399 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis())
}
