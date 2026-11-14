// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The command center on the screen (Codex's `agent_center/render.rs`,
//! `rows.rs` and `hints.rs`): its title and grouping, the filter tabs with
//! their counts, the search or rename being typed, the sessions under their
//! headings in the title, status and updated columns (the last two from 56
//! columns), the details pane beside them from 90 columns, and the keys or
//! the last notice; `?` shows every key in place of the list, and a
//! confirmation is drawn over it. A pinned session is under `Pinned`, an
//! unread one has a blue dot, and a hidden one (listed only by a search)
//! says so. Where the tabs and rows are drawn is kept for the mouse.

use std::collections::HashMap;

use hanafubuki_agent::AgentSessionSummary;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

use super::details::{DETAILS_FROM, DETAILS_WIDTH};
use super::grouping::{self, HeadingKey};
use super::input::SESSION_ACTIONS;
use super::status::{Filter, Group, Status};
use super::{AgentsCenter, Confirm, Mode, Row};
use crate::key_hint::{self, KeyBinding};
use crate::keymap::{self, Action};
use crate::text_formatting::{center_truncate_path, truncate_text};

/// The width from which the status and updated columns are shown.
const METADATA_FROM: u16 = 56;
const STATUS_WIDTH: u16 = 11;
const UPDATED_WIDTH: u16 = 9;
/// The marker, the status dot and their spaces.
const GUTTER: u16 = 4;

/// One line of the list.
enum Entry {
    Heading(usize),
    Row(usize),
    Gap,
}

impl AgentsCenter {
    /// Draws the center over `area`; `now` (milliseconds) dates the rows.
    pub(crate) fn render(&mut self, area: Rect, buf: &mut Buffer, now: u128) {
        Clear.render(area, buf);
        self.layout = super::ScreenLayout::default();
        let area = area.inner(Margin::new(2, 0));
        let editing = u16::from(self.takes_text());
        let [title, tabs, rule, edit, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(editing),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);
        self.title_line().render(title, buf);
        self.render_tabs(tabs, buf);
        Line::from("─".repeat(usize::from(rule.width)).dim()).render(rule, buf);
        self.edit_line().render(edit, buf);
        self.footer_line().render(footer, buf);
        if self.mode == Mode::Help {
            Paragraph::new(help_lines()).render(body, buf);
            return;
        }
        if body.width >= DETAILS_FROM {
            let [list, gap, details] = Layout::horizontal([
                Constraint::Min(46),
                Constraint::Length(3),
                Constraint::Length(DETAILS_WIDTH),
            ])
            .areas(body);
            self.render_list(list, buf, now);
            for y in gap.y..gap.bottom() {
                buf[(gap.x + 1, y)]
                    .set_symbol("│")
                    .set_style(Style::new().dim());
            }
            self.render_details(details, buf);
        } else if self.child_selected.is_some() {
            self.render_details(body, buf);
        } else {
            self.render_list(body, buf, now);
        }
        if let Mode::Confirm(confirm) = &self.mode {
            render_confirm(confirm, area, buf);
        }
    }

    fn title_line(&self) -> Line<'static> {
        let key = keymap::current().label(Action::AgentsToggleGrouping);
        Line::from(vec![
            "Agent command center".bold(),
            format!("  Group: {}  {key}", grouping::label(self.grouping)).dim(),
        ])
    }

    /// The filters with their counts; where each is drawn is kept.
    fn render_tabs(&mut self, area: Rect, buf: &mut Buffer) {
        let mut spans = Vec::new();
        let mut x = area.x;
        for filter in Filter::ALL {
            if !spans.is_empty() {
                spans.push("  ".into());
                x = x.saturating_add(2);
            }
            let count = match filter {
                Filter::All => Some(self.count(None)),
                Filter::Group(group) => Some(self.count(Some(group))),
                // The archived sessions are read only under their tab.
                Filter::Archived => None,
            };
            let label = match count.filter(|_| self.filter.lifecycle() == filter.lifecycle()) {
                Some(count) => format!(" {} {count} ", filter.label()),
                None => format!(" {} ", filter.label()),
            };
            let width = u16::try_from(label.chars().count()).unwrap_or(u16::MAX);
            let cell = Rect::new(x, area.y, width, 1).intersection(area);
            if !cell.is_empty() {
                self.layout.tabs.push((cell, filter));
            }
            x = x.saturating_add(width);
            spans.push(if filter == self.filter {
                label.bold().reversed()
            } else {
                label.dim()
            });
        }
        Line::from(spans).render(area, buf);
    }

    fn edit_line(&self) -> Line<'static> {
        let (prompt, text) = match &self.mode {
            Mode::Search(query) => ("Search › ", query.clone()),
            Mode::Rename { title, .. } => ("Rename › ", title.clone()),
            _ => return Line::default(),
        };
        Line::from(vec![prompt.cyan().bold(), text.into(), " ".reversed()])
    }

    fn footer_line(&self) -> Line<'static> {
        if let Some(notice) = &self.notice {
            return Line::from(notice.clone().magenta());
        }
        if let (Some(error), Mode::Browse) = (&self.error, &self.mode) {
            return Line::from(error.clone().red());
        }
        let enter = key_hint::plain(KeyCode::Enter);
        let esc = key_hint::plain(KeyCode::Esc);
        let hints: Vec<(KeyBinding, &str)> = match &self.mode {
            Mode::Browse if self.child_selected.is_some() => {
                let mut hints = vec![(enter, "open")];
                hints.extend(keymap::hint(Action::AgentsStop).map(|key| (key, "stop")));
                hints.push((esc, "back to the sessions"));
                hints
            }
            Mode::Browse => {
                let mut hints = vec![
                    (key_hint::plain(KeyCode::Char('?')), "help"),
                    (esc, "back"),
                    (enter, "open"),
                    (key_hint::plain(KeyCode::Tab), "filter"),
                ];
                hints.extend(keymap::hint(Action::AgentsNewTask).map(|key| (key, "new")));
                hints
            }
            Mode::Search(_) => vec![(enter, "open"), (esc, "clear the search")],
            Mode::Rename { .. } => vec![(enter, "rename"), (esc, "cancel")],
            Mode::Confirm(_) => vec![(enter, "choose"), (esc, "cancel")],
            Mode::Help => vec![(esc, "back")],
        };
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (key, label) in hints {
            if !spans.is_empty() {
                spans.push("  ".into());
            }
            spans.push(key.into());
            spans.push(format!(" {label}").dim());
        }
        Line::from(spans)
    }

    fn render_list(&mut self, area: Rect, buf: &mut Buffer, now: u128) {
        let rows = self.rows();
        if rows.is_empty() {
            let empty = if !self.reads.loaded && self.error.is_none() {
                "Loading…"
            } else if self.error.is_some() {
                "The sessions were not read."
            } else if self.query().is_some() || self.filter != Filter::All {
                "No sessions match."
            } else {
                "No sessions yet."
            };
            Line::from(empty.italic().dim()).render(area, buf);
            return;
        }
        let [columns, list] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
        let (title, status, updated) = column_areas(columns);
        Line::from("Sessions".dim()).render(title, buf);
        Line::from("Status".dim()).render(status, buf);
        Line::from("Updated".dim())
            .right_aligned()
            .render(updated, buf);

        let entries = entries(&rows);
        let height = usize::from(list.height);
        self.view_rows = (height > 0).then_some(height);
        let selected = self.selected_index(&rows);
        let position = entries
            .iter()
            .position(|entry| matches!(entry, Entry::Row(index) if *index == selected))
            .unwrap_or(0);
        self.scroll_to(&entries, position, height);
        let totals = self.heading_totals();
        for (offset, entry) in entries
            .iter()
            .skip(self.scroll_top)
            .take(height)
            .enumerate()
        {
            let y = list.y + u16::try_from(offset).unwrap_or(0);
            let line = Rect::new(list.x, y, list.width, 1);
            match entry {
                Entry::Gap => {}
                Entry::Heading(index) => {
                    let heading = &rows[*index].heading;
                    let count = rows.iter().filter(|row| row.heading == *heading).count();
                    let total = totals.get(&heading.key).copied().unwrap_or(count);
                    heading_line(
                        heading.key.clone(),
                        &heading.label,
                        count,
                        total,
                        line.width,
                    )
                    .render(line, buf);
                }
                Entry::Row(index) => {
                    let row = &rows[*index];
                    let shown = Some(row.summary.session_id) == self.current;
                    let marked = *index == selected && self.child_selected.is_none();
                    render_row(row, marked, shown, now, line, buf);
                    self.layout.rows.push((line, row.summary.session_id));
                }
            }
        }
    }

    /// Scrolls so that the entry at `position` shows, with its heading
    /// when moving up onto the first row under one.
    fn scroll_to(&mut self, entries: &[Entry], position: usize, height: usize) {
        let mut top = self.scroll_top.min(entries.len().saturating_sub(height));
        if position < top {
            top = match position.checked_sub(1).map(|above| &entries[above]) {
                Some(Entry::Heading(_)) if height > 1 => position - 1,
                _ => position,
            };
        }
        if position >= top + height {
            top = position + 1 - height.max(1);
        }
        self.scroll_top = top;
    }

    /// How many loaded sessions each heading has, whatever the filter.
    fn heading_totals(&self) -> HashMap<HeadingKey, usize> {
        let mut totals = HashMap::new();
        for row in self.all_rows() {
            *totals.entry(row.heading.key).or_insert(0) += 1;
        }
        totals
    }

    /// How many loaded sessions `group` has (all of them for `None`).
    fn count(&self, group: Option<Group>) -> usize {
        self.all_rows()
            .iter()
            .filter(|row| group.is_none_or(|group| row.status.group == group))
            .count()
    }
}

/// The rows under their headings, a blank line between two headings.
fn entries(rows: &[Row]) -> Vec<Entry> {
    let mut entries = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let first = index == 0 || rows[index - 1].heading != row.heading;
        if first {
            if index > 0 {
                entries.push(Entry::Gap);
            }
            entries.push(Entry::Heading(index));
        }
        entries.push(Entry::Row(index));
    }
    entries
}

fn heading_line(
    key: HeadingKey,
    label: &str,
    count: usize,
    total: usize,
    width: u16,
) -> Line<'static> {
    let count = if count == total {
        count.to_string()
    } else {
        format!("{count} of {total}")
    };
    let label = match key {
        HeadingKey::Path(_) => {
            let room = usize::from(width).saturating_sub(count.len() + 2).min(64);
            center_truncate_path(label, room)
        }
        _ => label.to_string(),
    };
    Line::from(format!("{label}  {count}").dim())
}

/// The title, status and updated columns of a line.
fn column_areas(line: Rect) -> (Rect, Rect, Rect) {
    let gutter = line.width.min(GUTTER);
    let metadata = if line.width >= METADATA_FROM {
        STATUS_WIDTH + 2 + UPDATED_WIDTH
    } else {
        0
    };
    let title = Rect::new(line.x + gutter, line.y, line.width - gutter - metadata, 1);
    if metadata == 0 {
        return (title, Rect::default(), Rect::default());
    }
    let status = Rect::new(title.right() + 2, line.y, STATUS_WIDTH, 1);
    let updated = Rect::new(line.right() - UPDATED_WIDTH, line.y, UPDATED_WIDTH, 1);
    (title, status, updated)
}

fn render_row(row: &Row, selected: bool, shown: bool, now: u128, line: Rect, buf: &mut Buffer) {
    let (title_area, status_area, updated_area) = column_areas(line);
    let marker = if selected { "›".bold() } else { " ".into() };
    let (label, mut dot, style) = status_look(row.status);
    // Codex's unread marker, on a session that asks nothing of you.
    if row.unread && matches!(row.status.group, Group::Ready | Group::Inactive) {
        dot = Span::styled("●", Style::new().fg(Color::Blue));
    }
    Line::from(vec![marker, " ".into(), dot, " ".into()]).render(line, buf);
    let suffix = match (shown, row.hidden) {
        (true, _) => " (shown)",
        (false, true) => " (hidden)",
        (false, false) => "",
    };
    let suffix = if row.pending_wake_events > 0 {
        format!("{suffix} · {} updates", row.pending_wake_events)
    } else {
        suffix.into()
    };
    let room = usize::from(title_area.width).saturating_sub(suffix.len());
    let mut title: Span<'static> = truncate_text(&title_of(&row.summary), room).into();
    if selected || row.unread {
        title = title.bold();
    }
    Line::from(vec![title, suffix.magenta()]).render(title_area, buf);
    Line::from(Span::styled(label, style)).render(status_area, buf);
    Line::from(ago(now, row.summary.updated_at).dim())
        .right_aligned()
        .render(updated_area, buf);
}

/// A status's label, its dot and its style.
pub(super) fn status_look(status: Status) -> (&'static str, Span<'static>, Style) {
    let style = match status.group {
        Group::NeedsYou => Style::new().red(),
        Group::Working => Style::new().green(),
        Group::Ready => Style::new().cyan(),
        Group::Inactive => Style::new().dim(),
    };
    let dot = match (status.error, status.group) {
        (true, _) => "!",
        (false, Group::NeedsYou | Group::Working) => "●",
        (false, Group::Ready | Group::Inactive) => "○",
    };
    (status.label(), Span::styled(dot, style), style)
}

fn render_confirm(confirm: &Confirm, area: Rect, buf: &mut Buffer) {
    let (question, about, label) = if confirm.delete {
        (
            format!("Permanently delete “{}”?", confirm.title),
            "Its history and its children's are deleted. This cannot be undone.",
            "Delete the session and its children",
        )
    } else {
        (
            format!("Archive “{}”?", confirm.title),
            "It and its children leave the list; the Archived tab and the resume picker \
             restore them.",
            "Archive the session and its children",
        )
    };
    let width = area.width.min(64);
    let height = area.height.min(9);
    let modal = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    Clear.render(modal, buf);
    let choice = |chosen: bool, text: &'static str| {
        if chosen {
            Line::from(vec!["› ".bold(), text.bold()])
        } else {
            Line::from(format!("  {text}"))
        }
    };
    let lines = vec![
        Line::from(question.bold()),
        Line::from(about.dim()),
        Line::default(),
        choice(!confirm.accept, "Cancel"),
        choice(confirm.accept, label),
    ];
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).dim())
        .render(modal, buf);
}

/// Every key of the center: the fixed ones, then the keymap's `agents`
/// actions as configured.
fn help_lines() -> Vec<Line<'static>> {
    let keymap = keymap::current();
    let mut entries: Vec<(String, &str)> = vec![
        (
            "up, down, k, j".to_string(),
            "Move; past an end, to the other end",
        ),
        (
            "page-up, page-down, home, end".to_string(),
            "Move a page, or to the first or last",
        ),
        (
            "enter, right".to_string(),
            "Open the session (restore it under Archived)",
        ),
        (
            "tab, shift-tab, 1–6".to_string(),
            "Filter: All, Needs you, Working, Ready, Inactive, Archived",
        ),
        (
            "click, wheel".to_string(),
            "Select a session, child or tab; on the one selected, open it",
        ),
        ("esc, q".to_string(), "Back"),
    ];
    entries.extend(
        SESSION_ACTIONS
            .into_iter()
            .map(|action| (keymap.label(action), action.spec().description)),
    );
    let width = entries
        .iter()
        .map(|(keys, _)| keys.chars().count())
        .max()
        .unwrap_or(0);
    let mut lines = vec![Line::from("Keys".bold()), Line::default()];
    lines.extend(
        entries
            .into_iter()
            .map(|(keys, does)| Line::from(vec![format!("{keys:<width$}  ").cyan(), does.into()])),
    );
    lines
}

pub(super) fn title_of(summary: &AgentSessionSummary) -> String {
    summary
        .title
        .as_ref()
        .map_or_else(|| "(untitled)".to_string(), |title| title.value.clone())
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
