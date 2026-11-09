// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).
// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

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
//! shows the title. Sessions no turn ran in are left out, and so are child
//! sessions (the list is the top level's). Tab switches to the archived
//! sessions; Ctrl+A archives the selected session, or restores it there
//! (Codex's `resume_picker/archive.rs`), which the app carries out
//! ([`ResumePicker::take_archive_request`]). Enter on an archived session
//! restores it too, where Codex also resumes it.

mod lookup;

use chrono::{DateTime, Utc};
use hanafubuki_agent::{
    AgentSessionId, AgentSessionLifecycle, AgentSessionListRequest, AgentSessionPage,
};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Stylize as _;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};
use unicode_width::UnicodeWidthStr;

use crate::key_hint;
use crate::text_formatting::truncate_text;
pub(crate) use lookup::find_session;

const PAGE_SIZE: usize = 25;
const LOAD_NEAR_THRESHOLD: usize = 5;
/// The widest the branch and directory columns grow.
const BRANCH_WIDTH: usize = 20;
const DIRECTORY_WIDTH: usize = 28;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionSelection {
    /// Esc: the session the interface shows stays (a new one at startup).
    StartFresh,
    Resume(AgentSessionId),
    Exit,
}

/// A session to archive (or, `archive: false`, to restore).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArchiveRequest {
    pub(crate) session_id: AgentSessionId,
    pub(crate) archive: bool,
}

#[derive(Clone, Debug)]
struct Row {
    session_id: AgentSessionId,
    preview: String,
    updated_at: Option<DateTime<Utc>>,
    /// Where the session was created, and the branch checked out there.
    cwd: Option<String>,
    branch: Option<String>,
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
    /// The archived sessions are listed instead of the others.
    archived: bool,
    /// The session the interface shows, which `/archive` archives instead.
    current: Option<AgentSessionId>,
    archive_request: Option<ArchiveRequest>,
    /// What the last archiving said, until the next key.
    notice: Option<String>,
}

impl ResumePicker {
    pub(crate) fn new(at_startup: bool, current: Option<AgentSessionId>) -> Self {
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
            archived: false,
            current,
            archive_request: None,
            notice: None,
        }
    }

    /// The picker searching for `query` from the start.
    pub(crate) fn with_query(mut self, query: String) -> Self {
        self.set_query(query);
        self
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Option<SessionSelection> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        self.notice = None;
        match key.code {
            KeyCode::Esc => return Some(SessionSelection::StartFresh),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(SessionSelection::Exit);
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.request_archive();
            }
            KeyCode::Tab => {
                self.archived = !self.archived;
                self.start_over();
            }
            KeyCode::Enter if self.archived => self.request_archive(),
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
        let lifecycle = if self.archived {
            AgentSessionLifecycle::Archived
        } else {
            AgentSessionLifecycle::Available
        };
        Some(AgentSessionListRequest {
            page_size: PAGE_SIZE,
            cursor,
            lifecycle,
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
                    cwd: session.cwd,
                    branch: session.branch,
                }),
        );
        self.ensure_selected_visible();
    }

    /// The archiving the person asked for, once: the app carries it out
    /// and hands back how it went ([`Self::archive_done`]).
    pub(crate) fn take_archive_request(&mut self) -> Option<ArchiveRequest> {
        self.archive_request.take()
    }

    /// `session_id` was archived (or restored): it leaves the list.
    pub(crate) fn archive_done(&mut self, session_id: AgentSessionId, result: Result<(), String>) {
        let verb = if self.archived {
            "restored"
        } else {
            "archived"
        };
        match result {
            Ok(()) => {
                self.rows.retain(|row| row.session_id != session_id);
                self.selected = self.selected.min(self.rows.len().saturating_sub(1));
                self.ensure_selected_visible();
                self.notice = Some(format!("Session {verb}"));
            }
            Err(error) => self.notice = Some(format!("The session was not {verb}: {error}")),
        }
    }

    fn request_archive(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        if !self.archived && Some(row.session_id) == self.current {
            self.notice = Some("Use /archive to archive the current session".to_string());
            return;
        }
        self.archive_request = Some(ArchiveRequest {
            session_id: row.session_id,
            archive: !self.archived,
        });
    }

    fn set_query(&mut self, new_query: String) {
        if self.query == new_query {
            return;
        }
        self.query = new_query;
        self.start_over();
    }

    /// The list read again from its first page.
    fn start_over(&mut self) {
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
        let title = if self.archived {
            "Archived sessions"
        } else {
            "Resume a previous session"
        };
        Line::from(vec![title.bold().cyan()]).render(header, buf);

        // Search line, or what the last archiving said
        let line = match &self.notice {
            Some(notice) => Line::from(notice.clone().magenta()),
            None if self.query.is_empty() => Line::from("Type to search".dim()),
            None => Line::from(format!("Search: {}", self.query)),
        };
        line.render(search, buf);

        let metrics = calculate_column_metrics(&self.rows);

        // Column headers and list
        render_column_headers(buf, columns, &metrics);
        self.render_list(buf, list, &metrics);

        // Hint line
        let esc_label = if self.at_startup {
            " start new"
        } else {
            " back"
        };
        let (enter_label, tab_label, archive_label) = if self.archived {
            (" restore", " others", " restore")
        } else {
            (" resume", " archived", " archive")
        };
        let hint_line: Line = vec![
            key_hint::plain(KeyCode::Enter).into(),
            enter_label.dim(),
            "  ".dim(),
            key_hint::plain(KeyCode::Esc).into(),
            esc_label.dim(),
            "  ".dim(),
            key_hint::plain(KeyCode::Tab).into(),
            tab_label.dim(),
            "  ".dim(),
            key_hint::ctrl(KeyCode::Char('a')).into(),
            archive_label.dim(),
            "  ".dim(),
            key_hint::ctrl(KeyCode::Char('c')).into(),
            " quit".dim(),
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
            let mut spans = vec![marker, updated_span, "  ".into()];
            let mut used = marker_width + max_updated_width + 2;
            if metrics.branch_width > 0 {
                let branch = truncate_text(row.branch.as_deref().unwrap_or("-"), BRANCH_WIDTH);
                spans.push(Span::from(pad(&branch, metrics.branch_width)).cyan());
                spans.push("  ".into());
                used += metrics.branch_width + 2;
            }
            if metrics.directory_width > 0 {
                let directory = shorten_directory(row.cwd.as_deref().unwrap_or("-"));
                spans.push(Span::from(pad(&directory, metrics.directory_width)).dim());
                spans.push("  ".into());
                used += metrics.directory_width + 2;
            }
            let preview_width = (area.width as usize).saturating_sub(used);
            spans.push(truncate_text(&row.preview, preview_width).into());
            let line: Line = spans.into();
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
        if self.archived {
            return vec!["No archived sessions".italic().dim()].into();
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
    let mut spans: Vec<Span> = vec!["  ".into(), Span::from(label).bold(), "  ".into()];
    if metrics.branch_width > 0 {
        spans.push(Span::from(pad("Branch", metrics.branch_width)).bold());
        spans.push("  ".into());
    }
    if metrics.directory_width > 0 {
        spans.push(Span::from(pad("Directory", metrics.directory_width)).bold());
        spans.push("  ".into());
    }
    spans.push("Conversation".bold());
    Line::from(spans).render(area, buf);
}

struct ColumnMetrics {
    max_updated_width: usize,
    labels: Vec<String>,
    /// Zero when no row has a branch (or a directory): the column is left
    /// out.
    branch_width: usize,
    directory_width: usize,
}

fn calculate_column_metrics(rows: &[Row]) -> ColumnMetrics {
    let labels: Vec<String> = rows.iter().map(format_updated_label).collect();
    let max_updated_width = labels
        .iter()
        .map(|label| UnicodeWidthStr::width(label.as_str()))
        .fold(UnicodeWidthStr::width("Updated"), usize::max);
    let branch_width = rows
        .iter()
        .filter_map(|row| row.branch.as_deref())
        .map(|branch| UnicodeWidthStr::width(truncate_text(branch, BRANCH_WIDTH).as_str()))
        .max()
        .map_or(0, |width| width.max(UnicodeWidthStr::width("Branch")));
    let directory_width = rows
        .iter()
        .filter_map(|row| row.cwd.as_deref())
        .map(|cwd| UnicodeWidthStr::width(shorten_directory(cwd).as_str()))
        .max()
        .map_or(0, |width| width.max(UnicodeWidthStr::width("Directory")));
    ColumnMetrics {
        max_updated_width,
        labels,
        branch_width,
        directory_width,
    }
}

/// `text` padded with spaces to `width` columns.
fn pad(text: &str, width: usize) -> String {
    let fill = width.saturating_sub(UnicodeWidthStr::width(text));
    format!("{text}{}", " ".repeat(fill))
}

/// The directory's end, which tells sessions apart, within
/// [`DIRECTORY_WIDTH`] columns.
fn shorten_directory(directory: &str) -> String {
    if UnicodeWidthStr::width(directory) <= DIRECTORY_WIDTH {
        return directory.to_string();
    }
    let mut tail = Vec::new();
    let mut width = 1;
    for character in directory.chars().rev() {
        let character_width = unicode_width::UnicodeWidthChar::width(character).unwrap_or(0);
        if width + character_width > DIRECTORY_WIDTH {
            break;
        }
        width += character_width;
        tail.push(character);
    }
    let tail: String = tail.into_iter().rev().collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests;
