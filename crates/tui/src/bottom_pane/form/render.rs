// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! How a form is drawn: a label column, each value beside its label, the
//! errors under their fields, the command's status and the keys; scrolled
//! so the focus shows.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use super::{FormView, Kind};
use crate::key_hint;
use crate::render::renderable::Renderable;

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
            .filter(|item| !matches!(item.kind, Kind::Action))
            .map(|item| item.label.width())
            .max()
            .unwrap_or(0);
        let mut cursor = None;
        let mut focus_line = 0;
        for (index, item) in self.items.iter().enumerate() {
            if self.focus == index {
                focus_line = lines.len();
            }
            let focused = self.focus == index;
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
