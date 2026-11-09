// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! How a form is drawn: a label column, each value beside its label, a
//! row list's rows under its label, the errors under their fields, the
//! command's status and the keys; scrolled so the focus shows.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use super::{stop_item, FormView, Kind, Row, Stop};
use crate::key_hint;
use crate::render::renderable::Renderable;

/// The width of a row's name column.
const ROW_NAME_WIDTH: usize = 22;
const SECRET_MARK: &str = " [secret]";
const PUBLIC_MARK: &str = " [public]";

impl FormView {
    /// The form's lines at `width`, the cursor's place in them, and the
    /// line the focus is on.
    fn layout(&self, width: u16) -> (Vec<Line<'static>>, Option<(u16, u16)>, usize) {
        let mut lines = vec![Line::from(self.title.clone().bold())];
        if let Some(subtitle) = &self.subtitle {
            lines.push(Line::from(subtitle.clone().dim()));
        }
        lines.push(Line::default());
        let label_width = self
            .items
            .iter()
            .filter(|item| !matches!(item.kind, Kind::Action | Kind::Rows { .. }))
            .map(|item| item.label.width())
            .max()
            .unwrap_or(0);
        let mut cursor = None;
        let mut focus_line = 0;
        let mut advanced_seen = false;
        for (index, item) in self.items.iter().enumerate() {
            if item.advanced && !advanced_seen {
                advanced_seen = true;
                if self.focus == Stop::Advanced {
                    focus_line = lines.len();
                }
                let arrow = if self.advanced_open { "▾" } else { "▸" };
                lines.push(Line::from(vec![
                    marker(self.focus == Stop::Advanced),
                    format!("{arrow} Advanced").bold(),
                ]));
            }
            if !self.visible(item) {
                continue;
            }
            if stop_item(self.focus) == Some(index) {
                focus_line = lines.len();
            }
            let focused = self.focus == Stop::Item(index);
            let value_x = (2 + label_width + 2) as u16;
            let value_width = width.saturating_sub(value_x);
            let mut line = Line::from(vec![
                marker(focused),
                format!("{:label_width$}  ", item.label).bold(),
            ]);
            match &item.kind {
                Kind::Text { field, placeholder } => {
                    let (value, column) =
                        field.line(placeholder, field.display(focused), value_width);
                    line.spans.extend(value.spans);
                    if let Some(column) = column {
                        cursor = Some((value_x + column, lines.len() as u16));
                    }
                }
                Kind::Choice { options, selected } => {
                    line.spans
                        .extend(choice_spans(options, *selected, focused, value_width));
                }
                Kind::Toggle(on) => {
                    let mark = if *on { "[x]" } else { "[ ]" };
                    line.push_span(if focused { mark.cyan() } else { mark.into() });
                }
                Kind::Action => {
                    let label = item.label.clone();
                    line = Line::from(vec![
                        marker(focused),
                        if focused {
                            label.bold().cyan()
                        } else {
                            label.bold()
                        },
                    ]);
                }
                Kind::Rows {
                    rows,
                    secret_column,
                    add_label,
                } => {
                    lines.push(Line::from(vec!["  ".into(), item.label.clone().bold()]));
                    for (row_index, row) in rows.iter().enumerate() {
                        if matches!(self.focus, Stop::RowName(i, r) | Stop::RowValue(i, r) | Stop::RowSecret(i, r) if i == index && r == row_index)
                        {
                            focus_line = lines.len();
                        }
                        let (row_line, row_cursor) =
                            self.row_line(index, row_index, row, *secret_column, width);
                        if let Some(x) = row_cursor {
                            cursor = Some((x, lines.len() as u16));
                        }
                        lines.push(row_line);
                    }
                    let add_focused = self.focus == Stop::AddRow(index);
                    if add_focused {
                        focus_line = lines.len();
                    }
                    let add = format!("+ {add_label}");
                    line = Line::from(vec![
                        "  ".into(),
                        marker(add_focused),
                        if add_focused { add.cyan() } else { add.dim() },
                    ]);
                }
            }
            lines.push(line);
            if let Some(error) = &item.error {
                lines.push(Line::from(format!("    {error}").red()));
            }
        }
        if !self.status.is_empty() {
            lines.push(Line::default());
            lines.extend(self.status.iter().cloned());
        }
        lines.push(Line::default());
        lines.push(Line::from(vec![
            key_hint::plain(KeyCode::Tab).into(),
            " move · ".dim(),
            key_hint::plain(KeyCode::Enter).into(),
            " next or run · ".dim(),
            key_hint::plain(KeyCode::Char(' ')).into(),
            " switch · ".dim(),
            key_hint::plain(KeyCode::Esc).into(),
            " back".dim(),
        ]));
        (lines, cursor, focus_line)
    }

    /// One row: its name, its value and, with a secret column, whether it
    /// is secret; the cursor's column when a field of it has the focus.
    fn row_line(
        &self,
        item: usize,
        index: usize,
        row: &Row,
        secret_column: bool,
        width: u16,
    ) -> (Line<'static>, Option<u16>) {
        let name_focused = self.focus == Stop::RowName(item, index);
        let value_focused = self.focus == Stop::RowValue(item, index);
        let secret_focused = self.focus == Stop::RowSecret(item, index);
        let row_focused = name_focused || value_focused || secret_focused;
        let mut line = Line::from(vec!["  ".into(), marker(row_focused)]);
        let name_x = 4u16;
        let (name, name_cursor) = row.name.line(
            "NAME",
            row.name.display(name_focused),
            ROW_NAME_WIDTH as u16,
        );
        let name_used = name.width();
        line.spans.extend(name.spans);
        line.push_span(" ".repeat(ROW_NAME_WIDTH.saturating_sub(name_used)));
        line.push_span(" = ".dim());
        let value_x = name_x + ROW_NAME_WIDTH as u16 + 3;
        let mark_width = if secret_column {
            SECRET_MARK.width()
        } else {
            0
        } as u16;
        let value_width = width.saturating_sub(value_x + mark_width).max(1);
        let (value, value_cursor) =
            row.value
                .line("value", row.value.display(value_focused), value_width);
        let value_used = value.width() as u16;
        line.spans.extend(value.spans);
        if secret_column {
            let mark = if row.value.is_secret() {
                SECRET_MARK
            } else {
                PUBLIC_MARK
            };
            line.push_span(" ".repeat(usize::from(value_width.saturating_sub(value_used))));
            line.push_span(if secret_focused {
                mark.cyan()
            } else {
                mark.dim()
            });
        }
        let cursor = name_cursor
            .filter(|_| name_focused)
            .map(|column| name_x + column)
            .or_else(|| {
                value_cursor
                    .filter(|_| value_focused)
                    .map(|column| value_x + column)
            });
        (line, cursor)
    }

    /// How far the lines are scrolled so the focus shows in `height` rows.
    fn scroll(focus_line: usize, total: usize, height: u16) -> usize {
        let height = usize::from(height);
        if total <= height {
            return 0;
        }
        (focus_line + 2).saturating_sub(height).min(total - height)
    }
}

impl Renderable for FormView {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let (lines, _, focus_line) = self.layout(area.width);
        let scroll = Self::scroll(focus_line, lines.len(), area.height);
        Paragraph::new(lines)
            .scroll((scroll as u16, 0))
            .render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.layout(width).0.len() as u16
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        let (lines, cursor, focus_line) = self.layout(area.width);
        let scroll = Self::scroll(focus_line, lines.len(), area.height) as u16;
        let (x, y) = cursor?;
        let y = y.checked_sub(scroll)?;
        (y < area.height && x < area.width).then(|| (area.x + x, area.y + y))
    }
}

fn marker(focused: bool) -> Span<'static> {
    if focused {
        "› ".cyan()
    } else {
        "  ".into()
    }
}

/// Every option with the selected one marked, or only the selected one
/// between arrows when they do not all fit.
fn choice_spans(
    options: &[String],
    selected: usize,
    focused: bool,
    width: u16,
) -> Vec<Span<'static>> {
    let all: usize = options.iter().map(|option| option.width() + 4).sum();
    let style = |span: Span<'static>| if focused { span.cyan() } else { span };
    if all <= usize::from(width) {
        options
            .iter()
            .enumerate()
            .map(|(index, option)| {
                if index == selected {
                    style(format!("● {option}  ").into())
                } else {
                    format!("○ {option}  ").dim()
                }
            })
            .collect()
    } else {
        let option = options.get(selected).cloned().unwrap_or_default();
        vec![style(format!("‹ {option} ›").into())]
    }
}
