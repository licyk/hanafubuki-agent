// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! The resume picker: the history store's sessions, the latest first, to
//! reopen one. It is an overlay on the alternate screen, opened by `/resume`
//! and by `hanafubuki-agent resume`.
//!
//! Codex's picker reads rollout files on background tasks, page by page,
//! and searches what it loaded under a scan cap. Here the pages come from
//! the facade's session list, which is cursor-paged and searches titles
//! itself: the app loads a page whenever the picker asks for one
//! ([`ResumePicker::page_request`]), synchronously. A session's title is
//! its first message until the Agent names it, so the conversation column
//! shows the title. Sessions no turn ran in are left out. The summaries
//! carry no working directory or Git branch, so those columns are gone.

use chrono::{DateTime, Utc};
use hanafubuki_agent::{AgentSessionId, AgentSessionListRequest, AgentSessionPage};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize as _;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};
use unicode_width::UnicodeWidthStr;

use crate::key_hint;
use crate::text_formatting::truncate_text;

const PAGE_SIZE: usize = 25;
const LOAD_NEAR_THRESHOLD: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionSelection {
    /// Esc: the session the interface shows stays (a new one at startup).
    StartFresh,
    Resume(AgentSessionId),
    Exit,
}

#[derive(Clone, Debug)]
struct Row {
    session_id: AgentSessionId,
    preview: String,
    updated_at: Option<DateTime<Utc>>,
}

pub(crate) struct ResumePicker {
    rows: Vec<Row>,
    /// Whether the first page of the current query was read.
    loaded: bool,
    next_cursor: Option<String>,
    /// Why the last page could not be read.
    error: Option<String>,
    selected: usize,
    scroll_top: usize,
    query: String,
    view_rows: Option<usize>,
    /// Opened by `hanafubuki-agent resume`, before any session was shown.
    at_startup: bool,
}

impl ResumePicker {
    pub(crate) fn new(at_startup: bool) -> Self {
        Self {
            rows: Vec::new(),
            loaded: false,
            next_cursor: None,
            error: None,
            selected: 0,
            scroll_top: 0,
            query: String::new(),
            view_rows: None,
            at_startup,
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Option<SessionSelection> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        match key.code {
            KeyCode::Esc => return Some(SessionSelection::StartFresh),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(SessionSelection::Exit);
            }
            KeyCode::Enter => {
                if let Some(row) = self.rows.get(self.selected) {
                    return Some(SessionSelection::Resume(row.session_id));
                }
            }
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                    self.ensure_selected_visible();
                }
            }
            KeyCode::Down => {
                if self.selected + 1 < self.rows.len() {
                    self.selected += 1;
                    self.ensure_selected_visible();
                }
            }
            KeyCode::PageUp => {
                let step = self.view_rows.unwrap_or(10).max(1);
                if self.selected > 0 {
                    self.selected = self.selected.saturating_sub(step);
                    self.ensure_selected_visible();
                }
            }
            KeyCode::PageDown => {
                if !self.rows.is_empty() {
                    let step = self.view_rows.unwrap_or(10).max(1);
                    let max_index = self.rows.len().saturating_sub(1);
                    self.selected = (self.selected + step).min(max_index);
                    self.ensure_selected_visible();
                }
            }
            KeyCode::Backspace => {
                let mut new_query = self.query.clone();
                new_query.pop();
                self.set_query(new_query);
            }
            // basic text input for search
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                let mut new_query = self.query.clone();
                new_query.push(c);
                self.set_query(new_query);
            }
            _ => {}
        }
        None
    }

    /// The page the picker needs next: the first one of its query, or the
    /// next one while the selection nears the end of what is loaded or the
    /// rows do not fill the list. `None` when nothing more is needed.
    pub(crate) fn page_request(&self) -> Option<AgentSessionListRequest> {
        if self.error.is_some() {
            return None;
        }
        let cursor = if self.loaded {
            let cursor = self.next_cursor.clone()?;
            let remaining = self.rows.len().saturating_sub(self.selected + 1);
            let unfilled = self.rows.len() < self.view_rows.unwrap_or(PAGE_SIZE);
            if remaining > LOAD_NEAR_THRESHOLD && !unfilled {
                return None;
            }
            Some(cursor)
        } else {
            None
        };
        Some(AgentSessionListRequest {
            page_size: PAGE_SIZE,
            cursor,
            search: (!self.query.is_empty()).then(|| self.query.clone()),
            ..AgentSessionListRequest::default()
        })
    }

    pub(crate) fn ingest_page(&mut self, page: Result<AgentSessionPage, String>) {
        let page = match page {
            Ok(page) => page,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        self.loaded = true;
        self.next_cursor = page.next_cursor;
        self.rows.extend(
            page.sessions
                .into_iter()
                .filter(|session| session.latest_turn_phase.is_some())
                .map(|session| Row {
                    session_id: session.session_id,
                    preview: session
                        .title
                        .map(|title| title.value)
                        .unwrap_or_else(|| "(untitled)".to_string()),
                    updated_at: i64::try_from(session.updated_at)
                        .ok()
                        .and_then(DateTime::from_timestamp_millis),
                }),
        );
        self.ensure_selected_visible();
    }

    fn set_query(&mut self, new_query: String) {
        if self.query == new_query {
            return;
        }
        self.query = new_query;
        self.rows.clear();
        self.loaded = false;
        self.next_cursor = None;
        self.error = None;
        self.selected = 0;
        self.scroll_top = 0;
    }

    fn ensure_selected_visible(&mut self) {
        if self.rows.is_empty() {
            self.scroll_top = 0;
            return;
        }
        let capacity = self.view_rows.unwrap_or(self.rows.len()).max(1);

        if self.selected < self.scroll_top {
            self.scroll_top = self.selected;
        } else {
            let last_visible = self.scroll_top.saturating_add(capacity - 1);
            if self.selected > last_visible {
                self.scroll_top = self.selected.saturating_sub(capacity - 1);
            }
        }

        let max_start = self.rows.len().saturating_sub(capacity);
        if self.scroll_top > max_start {
            self.scroll_top = max_start;
        }
    }

    fn update_view_rows(&mut self, rows: usize) {
        self.view_rows = if rows == 0 { None } else { Some(rows) };
        self.ensure_selected_visible();
    }

    pub(crate) fn render(&mut self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        let [header, search, columns, list, hint] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(area.height.saturating_sub(4)),
            Constraint::Length(1),
        ])
        .areas(area);
        self.update_view_rows(list.height as usize);

        // Header
        Line::from(vec!["Resume a previous session".bold().cyan()]).render(header, buf);

        // Search line
        let q = if self.query.is_empty() {
            "Type to search".dim().to_string()
        } else {
            format!("Search: {}", self.query)
        };
        Line::from(q).render(search, buf);

        let metrics = calculate_column_metrics(&self.rows);

        // Column headers and list
        render_column_headers(buf, columns, &metrics);
        self.render_list(buf, list, &metrics);

        // Hint line
        let esc_label = if self.at_startup {
            " to start new "
        } else {
            " to go back "
        };
        let hint_line: Line = vec![
            key_hint::plain(KeyCode::Enter).into(),
            " to resume ".dim(),
            "    ".dim(),
            key_hint::plain(KeyCode::Esc).into(),
            esc_label.dim(),
            "    ".dim(),
            key_hint::ctrl(KeyCode::Char('c')).into(),
            " to quit ".dim(),
            "    ".dim(),
            key_hint::plain(KeyCode::Up).into(),
            "/".dim(),
            key_hint::plain(KeyCode::Down).into(),
            " to browse".dim(),
        ]
        .into();
        hint_line.render(hint, buf);
    }

    fn render_list(&self, buf: &mut Buffer, area: Rect, metrics: &ColumnMetrics) {
        if area.height == 0 {
            return;
        }

        let rows = &self.rows;
        if rows.is_empty() {
            self.render_empty_state_line().render(area, buf);
            return;
        }

        let capacity = area.height as usize;
        let start = self.scroll_top.min(rows.len().saturating_sub(1));
        let end = rows.len().min(start + capacity);
        let max_updated_width = metrics.max_updated_width;
        let mut y = area.y;

        for (idx, (row, updated_label)) in rows[start..end]
            .iter()
            .zip(metrics.labels[start..end].iter())
            .enumerate()
        {
            let is_sel = start + idx == self.selected;
            let marker = if is_sel { "> ".bold() } else { "  ".into() };
            let marker_width = 2usize;
            let updated_span = Span::from(format!("{updated_label:<max_updated_width$}")).dim();

            let preview_width = (area.width as usize)
                .saturating_sub(marker_width)
                .saturating_sub(max_updated_width + 2);
            let preview = truncate_text(&row.preview, preview_width);
            let line: Line = vec![marker, updated_span, "  ".into(), preview.into()].into();
            line.render(Rect::new(area.x, y, area.width, 1), buf);
            y = y.saturating_add(1);
        }
    }

    fn render_empty_state_line(&self) -> Line<'static> {
        if let Some(error) = &self.error {
            return vec![Span::from(error.clone()).red()].into();
        }
        if !self.query.is_empty() {
            return vec!["No results for your search".italic().dim()].into();
        }
        vec!["No sessions yet".italic().dim()].into()
    }
}

fn human_time_ago(ts: DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now - ts;
    let secs = delta.num_seconds();
    if secs < 60 {
        let n = secs.max(0);
        if n == 1 {
            format!("{n} second ago")
        } else {
            format!("{n} seconds ago")
        }
    } else if secs < 60 * 60 {
        let m = secs / 60;
        if m == 1 {
            format!("{m} minute ago")
        } else {
            format!("{m} minutes ago")
        }
    } else if secs < 60 * 60 * 24 {
        let h = secs / 3600;
        if h == 1 {
            format!("{h} hour ago")
        } else {
            format!("{h} hours ago")
        }
    } else {
        let d = secs / (60 * 60 * 24);
        if d == 1 {
            format!("{d} day ago")
        } else {
            format!("{d} days ago")
        }
    }
}

fn format_updated_label(row: &Row) -> String {
    match row.updated_at {
        Some(updated) => human_time_ago(updated),
        None => "-".to_string(),
    }
}

fn render_column_headers(buf: &mut Buffer, area: Rect, metrics: &ColumnMetrics) {
    if area.height == 0 {
        return;
    }
    let label = format!(
        "{text:<width$}",
        text = "Updated",
        width = metrics.max_updated_width
    );
    let spans: Vec<Span> = vec![
        "  ".into(),
        Span::from(label).bold(),
        "  ".into(),
        "Conversation".bold(),
    ];
    Line::from(spans).render(area, buf);
}

struct ColumnMetrics {
    max_updated_width: usize,
    labels: Vec<String>,
}

fn calculate_column_metrics(rows: &[Row]) -> ColumnMetrics {
    let labels: Vec<String> = rows.iter().map(format_updated_label).collect();
    let max_updated_width = labels
        .iter()
        .map(|label| UnicodeWidthStr::width(label.as_str()))
        .fold(UnicodeWidthStr::width("Updated"), usize::max);
    ColumnMetrics {
        max_updated_width,
        labels,
    }
}

#[cfg(test)]
mod tests;
