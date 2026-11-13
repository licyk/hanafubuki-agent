// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The center on the screen (Codex's `config_panel/render.rs`): the title
//! with the last outcome under it, the tabs, the rows as label · value ·
//! description columns (the label column as wide as the widest label, the
//! value padded), the selected row's `›`, and its own hint at the bottom.
//! Under 45 columns of description, the selected row's description wraps
//! below the list instead.

use hanafubuki_agent::{SettingKind, SettingSpec, SettingValue, SettingsCatalog};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Widget, Wrap};
use unicode_width::UnicodeWidthStr;

use super::editor::{LineEditor, ListState};
use super::main_page::tabs;
use super::values::{group_label, value_text};
use super::{Notice, Page, SettingsCenter};
use crate::text_formatting::truncate_text;

/// The description column's narrowest; below it, it moves under the list.
const NARROW_DESCRIPTION: u16 = 45;
const LABEL_MAX: usize = 32;
const VALUE_MIN: usize = 20;
const VALUE_MAX: usize = 28;
/// The lines the selected row's description takes below a narrow list.
const NARROW_LINES: u16 = 3;
/// `Search › `.
const SEARCH_PREFIX: u16 = 9;

/// One row as a page shows it.
pub(super) struct Row<'a> {
    pub(super) label: String,
    pub(super) value: String,
    /// The layer above the user's file that sets the value, dim after it.
    pub(super) source: Option<String>,
    pub(super) description: String,
    pub(super) style: RowStyle,
    pub(super) selectable: bool,
    /// What its keys do (`enter toggle`).
    pub(super) hint: String,
    /// The editor in its value column, while it is edited.
    pub(super) editor: Option<&'a LineEditor>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RowStyle {
    Plain,
    Heading,
    Issue,
    Disabled,
}

impl<'a> Row<'a> {
    pub(super) fn heading(title: &str) -> Self {
        Self {
            style: RowStyle::Heading,
            selectable: false,
            ..Self::open(title, "", "")
        }
    }

    /// A row that opens something, or does something.
    pub(super) fn open(label: &str, value: &str, description: &str) -> Self {
        Self {
            label: label.to_string(),
            value: value.to_string(),
            source: None,
            description: description.to_string(),
            style: RowStyle::Plain,
            selectable: true,
            hint: "enter open".to_string(),
            editor: None,
        }
    }

    /// A setting or a field: its value as its kind reads; `opens` when
    /// Enter opens a page of its own.
    pub(super) fn setting(
        spec: &SettingSpec,
        value: Option<&SettingValue>,
        secret_set: bool,
        opens: bool,
    ) -> Self {
        let mut text = value_text(spec, value, secret_set);
        let hint = match &spec.kind {
            _ if opens => "enter open",
            SettingKind::Boolean => "enter toggle",
            SettingKind::Choice { .. } | SettingKind::ChoiceOrText { .. } => "enter choose",
            SettingKind::TextList { .. } => "enter edit the list",
            _ => "enter edit",
        };
        if opens
            || matches!(
                spec.kind,
                SettingKind::Choice { .. }
                    | SettingKind::ChoiceOrText { .. }
                    | SettingKind::TextList { .. }
            )
        {
            text.push_str(" ›");
        }
        Self {
            hint: hint.to_string(),
            ..Self::open(&spec.label, &text, &spec.summary)
        }
    }
}

/// Where the last frame put what the pointer reaches.
#[derive(Debug, Default)]
pub(super) struct Layout {
    /// Each tab's cells and its place among the tabs.
    pub(super) tabs: Vec<(Rect, usize)>,
    /// Each row drawn: its line and its index.
    pub(super) rows: Vec<(u16, usize)>,
    pub(super) list: Rect,
    pub(super) cursor: Option<(u16, u16)>,
}

impl SettingsCenter {
    /// Draws the center; the area a command's views take, when they are
    /// the page on top.
    pub(crate) fn render(&mut self, area: Rect, buf: &mut Buffer) -> Option<Rect> {
        Clear.render(area, buf);
        let area = Rect {
            x: area.x + 2,
            width: area.width.saturating_sub(4),
            ..area
        };
        let line = |y: u16| Rect {
            y,
            height: 1,
            ..area
        };
        let mut layout = Layout::default();
        let Some(catalog) = self.catalog.as_ref() else {
            Line::from("Settings".bold()).render(line(area.y + 1), buf);
            let text = match &self.read_error {
                Some(error) => format!("The settings could not be read: {error}").red(),
                None => "Reading the settings…".dim(),
            };
            Line::from(text).render(line(area.y + 2), buf);
            self.layout = layout;
            return None;
        };
        let top = self.pages.len() - 1;
        let mut y = area.y + 1;
        self.title(catalog, top).render(line(y), buf);
        notice_line(&self.notice, self.editor_error()).render(line(y + 1), buf);
        y += 3;
        let bottom = area.bottom().saturating_sub(1);
        let items = self.list_items(top);
        let (view, mut list, hint, empty) = match &self.pages[top] {
            Page::Hosted(_) => {
                Line::from("esc back".dim()).render(line(bottom), buf);
                self.layout = layout;
                return Some(Rect {
                    y,
                    height: bottom.saturating_sub(y),
                    ..area
                });
            }
            Page::Main(page) => {
                if page.search.is_none() {
                    layout.tabs = render_tabs(catalog, page.tab, line(y), buf);
                }
                y += 2;
                if let Some(search) = &page.search {
                    let (query, cursor) = search.query.line(
                        "type to search every tab",
                        search.query.display(true),
                        area.width.saturating_sub(SEARCH_PREFIX),
                    );
                    let mut spans = vec!["Search › ".dim()];
                    spans.extend(query.spans);
                    Line::from(spans).render(line(y), buf);
                    if search.editing {
                        layout.cursor = cursor.map(|cursor| (area.x + SEARCH_PREFIX + cursor, y));
                    }
                    y += 2;
                }
                let rows = page.rows(catalog);
                let view = page.view(catalog, &rows);
                let hint = match &page.search {
                    _ if page.editor.is_some() => Some("enter save · esc cancel".to_string()),
                    Some(search) if search.editing => {
                        Some("type to search · enter results · esc clear".to_string())
                    }
                    Some(_) => Some(row_hint(
                        &view,
                        page.list.selected,
                        "/ search · ",
                        "esc clear search",
                    )),
                    None => Some(row_hint(
                        &view,
                        page.list.selected,
                        "←/→ tab · / search · ",
                        "esc close",
                    )),
                };
                let empty = if page.search.is_some() {
                    "Nothing matches."
                } else {
                    ""
                };
                (view, page.list, hint, empty)
            }
            page => {
                let subtitle = match page {
                    Page::Form(form) => form.subtitle(catalog),
                    Page::Choice(choice) => choice.subtitle.clone(),
                    Page::List(list) => list.spec.summary.clone(),
                    _ => String::new(),
                };
                if !subtitle.is_empty() {
                    Paragraph::new(subtitle.dim())
                        .wrap(Wrap { trim: true })
                        .render(
                            Rect {
                                y,
                                height: 2,
                                ..area
                            },
                            buf,
                        );
                    y += 3;
                }
                let (view, list) = match page {
                    Page::Collection(page) => (page.view(catalog), page.list),
                    Page::Form(page) => (page.view(catalog), page.list),
                    Page::Choice(page) => (page.view(), page.list),
                    Page::List(page) => (page.view(&items), page.list),
                    Page::Main(_) | Page::Hosted(_) => (Vec::new(), ListState::default()),
                };
                (view, list, None, "")
            }
        };
        let body = Rect {
            y,
            height: bottom.saturating_sub(y + 1),
            ..area
        };
        render_list(&mut layout, &view, &mut list, body, buf, empty);
        let hint = hint.unwrap_or_else(|| editing_hint(&view, list.selected, "esc back"));
        Line::from(hint.dim()).render(line(bottom), buf);
        drop(view);
        if let Some(state) = self.pages[top].list_mut() {
            *state = list;
        }
        self.layout = layout;
        None
    }

    /// Where the frame's cursor goes: in the field being edited.
    pub(crate) fn cursor(&self) -> Option<(u16, u16)> {
        self.layout.cursor
    }

    fn title(&self, catalog: &SettingsCatalog, top: usize) -> Line<'static> {
        let mut spans = vec!["Settings".bold()];
        let page = match &self.pages[top] {
            Page::Main(_) => None,
            Page::Collection(page) => Some(page.title(catalog)),
            Page::Form(page) => Some(page.title(catalog)),
            Page::Choice(page) => Some(page.title.clone()),
            Page::List(page) => Some(page.spec.label.clone()),
            Page::Hosted(page) => Some(page.id.to_string()),
        };
        if let Some(page) = page.filter(|page| !page.is_empty()) {
            spans.push(" › ".dim());
            spans.push(page.bold());
        }
        Line::from(spans)
    }

    /// Why the field being edited was refused.
    fn editor_error(&self) -> Option<&str> {
        let editor = match self.pages.last()? {
            Page::Main(page) => page.editor.as_ref(),
            Page::Form(page) => page.editor.as_ref(),
            Page::List(page) => page.editor.as_ref().map(|(_, editor)| editor),
            _ => None,
        };
        editor?.error.as_deref()
    }
}

fn notice_line(notice: &Notice, editor_error: Option<&str>) -> Line<'static> {
    if let Some(error) = editor_error {
        return Line::from(format!("✗ {error}").red());
    }
    match notice {
        Notice::Ready => Line::from("Changes save automatically".dim()),
        Notice::Busy => Line::from("Saving…".dim()),
        Notice::Done(text) => Line::from(text.clone().green()),
        Notice::Failed(text) => Line::from(text.clone().red()),
    }
}

/// The tabs, each with how many of its values have an issue; when they
/// do not fit, from the first that lets the selected one show.
fn render_tabs(
    catalog: &SettingsCatalog,
    selected: usize,
    area: Rect,
    buf: &mut Buffer,
) -> Vec<(Rect, usize)> {
    let labels = tabs(catalog)
        .into_iter()
        .map(|group| match issues_in(catalog, group) {
            0 => (format!(" {} ", group_label(group)), false),
            count => (format!(" {} ({count}) ", group_label(group)), true),
        })
        .collect::<Vec<_>>();
    let selected = selected.min(labels.len().saturating_sub(1));
    let width = |range: &[(String, bool)]| {
        range
            .iter()
            .map(|(label, _)| label.width() + 1)
            .sum::<usize>()
    };
    let first = (0..=selected)
        .find(|first| width(&labels[*first..=selected]) <= usize::from(area.width))
        .unwrap_or(selected);
    let mut x = area.x;
    let mut hits = Vec::new();
    for (index, (label, issues)) in labels.into_iter().enumerate().skip(first) {
        let width = label.width() as u16;
        if x + width > area.right() {
            break;
        }
        let span = if index == selected {
            Span::styled(label, Style::new().bold().reversed())
        } else if issues {
            label.yellow()
        } else {
            label.dim()
        };
        let cell = Rect { x, width, ..area };
        Line::from(span).render(cell, buf);
        hits.push((cell, index));
        x += width + 1;
    }
    hits
}

/// The values, collections and records of `group` that have an issue.
fn issues_in(catalog: &SettingsCatalog, group: hanafubuki_agent::SettingsGroup) -> usize {
    let values = catalog
        .sections
        .iter()
        .filter(|section| section.group == group)
        .flat_map(|section| &section.values)
        .filter(|state| state.issue.is_some())
        .count();
    let collections = catalog
        .collections
        .iter()
        .filter(|collection| collection.group == group)
        .map(|collection| {
            collection.issues.len()
                + collection
                    .records
                    .iter()
                    .filter(|record| record.issue.is_some())
                    .count()
        })
        .sum::<usize>();
    values + collections
}

fn row_hint(rows: &[Row], selected: usize, prefix: &str, suffix: &str) -> String {
    let row = rows.get(selected).filter(|row| row.selectable);
    match row {
        Some(row) => format!("{prefix}{} · {suffix}", row.hint),
        None => format!("{prefix}{suffix}"),
    }
}

fn editing_hint(rows: &[Row], selected: usize, suffix: &str) -> String {
    match rows.get(selected) {
        Some(row) if row.editor.is_some() => "enter save · esc cancel".to_string(),
        _ => row_hint(rows, selected, "", suffix),
    }
}

/// The rows in `area`, scrolled to the selected one; under a narrow
/// description column, the selected row's description below them.
fn render_list(
    layout: &mut Layout,
    rows: &[Row],
    list: &mut ListState,
    area: Rect,
    buf: &mut Buffer,
    empty: &str,
) {
    layout.list = area;
    if rows.is_empty() {
        Line::from(empty.dim().italic()).render(Rect { height: 1, ..area }, buf);
        return;
    }
    list.selected = list.selected.min(rows.len() - 1);
    let width = usize::from(area.width);
    let label_width = rows
        .iter()
        .filter(|row| row.style != RowStyle::Heading)
        .map(|row| row.label.width())
        .max()
        .unwrap_or(0)
        .min(LABEL_MAX)
        .min(width / 2);
    let value_width = rows
        .iter()
        .map(|row| row.value.width() + row.source.as_ref().map_or(0, |source| source.width() + 3))
        .max()
        .unwrap_or(0)
        .clamp(VALUE_MIN, VALUE_MAX);
    let used = 2 + label_width + 2 + value_width + 2;
    let narrow = width.saturating_sub(used) < usize::from(NARROW_DESCRIPTION);
    let (value_width, description_width) = if narrow {
        (width.saturating_sub(2 + label_width + 2), 0)
    } else {
        (value_width, width - used)
    };
    let list_height = if narrow {
        area.height.saturating_sub(NARROW_LINES + 1)
    } else {
        area.height
    };
    list.keep_visible(usize::from(list_height), rows.len());
    let columns = Columns {
        label: label_width,
        value: value_width,
        description: description_width,
    };
    for (offset, (index, row)) in rows
        .iter()
        .enumerate()
        .skip(list.scroll)
        .take(usize::from(list_height))
        .enumerate()
    {
        let y = area.y + offset as u16;
        let selected = index == list.selected && row.selectable;
        let cell = Rect {
            y,
            height: 1,
            ..area
        };
        let (line, cursor) = row_line(row, selected, &columns);
        line.render(cell, buf);
        if let Some(cursor) = cursor {
            layout.cursor = Some((area.x + cursor, y));
        }
        if row.selectable {
            layout.rows.push((y, index));
        }
    }
    if narrow {
        let description = rows
            .get(list.selected)
            .filter(|row| row.selectable)
            .map_or("", |row| row.description.as_str());
        let below = Rect {
            y: area.y + list_height + 1,
            height: NARROW_LINES,
            ..area
        };
        Paragraph::new(description.to_string().dim())
            .wrap(Wrap { trim: true })
            .render(below, buf);
    }
}

struct Columns {
    label: usize,
    value: usize,
    /// None under a narrow width.
    description: usize,
}

/// A row's line, and the column of its editor's cursor.
fn row_line(row: &Row, selected: bool, columns: &Columns) -> (Line<'static>, Option<u16>) {
    if row.style == RowStyle::Heading {
        return (Line::from(row.label.clone().dim().bold()), None);
    }
    let marker = if selected { "› " } else { "  " };
    let label = pad(&truncate_text(&row.label, columns.label), columns.label);
    let mut spans = vec![Span::raw(marker), Span::raw(label), Span::raw("  ")];
    let mut cursor = None;
    if let Some(editor) = row.editor {
        let width = columns.value
            + if columns.description > 0 {
                2 + columns.description
            } else {
                0
            };
        let field = editor.field();
        let (line, column) = field.line("", field.display(true), width as u16);
        cursor = column.map(|column| (2 + columns.label + 2) as u16 + column);
        spans.extend(line.spans);
        return (Line::from(spans), cursor);
    }
    let source = row.source.as_ref().map(|source| format!(" · {source}"));
    let source_width = source.as_ref().map_or(0, |source| source.width());
    let value = truncate_text(
        &row.value,
        columns.value.saturating_sub(source_width).max(1),
    );
    let value_width = value.width() + source_width;
    spans.push(Span::raw(value));
    if let Some(source) = source {
        spans.push(source.dim());
    }
    if columns.description > 0 {
        spans.push(Span::raw(
            " ".repeat(columns.value.saturating_sub(value_width) + 2),
        ));
        let description = truncate_text(&row.description, columns.description);
        spans.push(match row.style {
            RowStyle::Issue => description.yellow(),
            _ if selected => Span::raw(description),
            _ => description.dim(),
        });
    }
    let mut line = Line::from(spans);
    if selected {
        line = line.style(Style::new().cyan().bold());
    } else if row.style == RowStyle::Disabled {
        line = line.style(Style::new().dim());
    }
    (line, cursor)
}

fn pad(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}

/// The settings center's text of a value, for tests.
#[cfg(test)]
pub(super) fn tab_labels(catalog: &SettingsCatalog) -> Vec<&'static str> {
    tabs(catalog).into_iter().map(group_label).collect()
}
