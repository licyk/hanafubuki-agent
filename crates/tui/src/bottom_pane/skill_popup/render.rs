// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use super::SkillPopup;
use crate::bottom_pane::popup_consts::MAX_POPUP_ROWS;
use crate::text_formatting::truncate_text;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::Widget,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

impl SkillPopup {
    pub(super) fn render_rows(&self, area: Rect, buf: &mut Buffer) {
        self.state.hits.borrow_mut().clear();
        if area.is_empty() {
            return;
        }
        let inset = u16::from(area.height >= 3);
        let body = Rect {
            y: area.y + inset,
            height: area.height.saturating_sub(inset * 2),
            ..area
        };
        if body.is_empty() {
            return;
        }
        let rows = self.filtered();
        if rows.is_empty() {
            Line::from("no matches".italic()).render(body, buf);
            return;
        }
        let count = rows.len().min(MAX_POPUP_ROWS).min(body.height as usize);
        let mut start = self.state.scroll_top.min(rows.len().saturating_sub(count));
        if let Some(index) = self.state.selected_idx {
            if index < start {
                start = index;
            } else if index >= start + count {
                start = index + 1 - count;
            }
        }
        let max_name = rows
            .iter()
            .map(|(i, _, _)| {
                UnicodeWidthStr::width(truncate_text(&self.mentions[*i].display_name, 28).as_str())
            })
            .max()
            .unwrap_or(0);
        for (row, (index, indices, _)) in rows.iter().enumerate().skip(start).take(count) {
            let selected = self.state.selected_idx == Some(row);
            let item = &self.mentions[*index];
            let width = usize::from(body.width);
            let tag = item.category_tag.as_deref().unwrap_or("");
            let tag_width = UnicodeWidthStr::width(tag);
            let name_width = max_name.min(width.saturating_sub(4 + tag_width));
            let name = crate::line_truncation::truncate_line_with_ellipsis_if_overflow(
                Line::from(truncate_text(&item.display_name, 28)),
                name_width,
            )
            .to_string();
            let mut spans = vec![Span::raw(if selected { "› " } else { "  " })];
            let mut offset = 0;
            for grapheme in name.graphemes(true) {
                let end = offset + grapheme.chars().count();
                let style = if indices
                    .as_ref()
                    .is_some_and(|indices| indices.iter().any(|i| (offset..end).contains(i)))
                {
                    Style::default().bold()
                } else {
                    Style::default()
                };
                spans.push(Span::styled(grapheme.to_owned(), style));
                offset = end;
            }
            spans.push(Span::raw(" ".repeat(
                name_width.saturating_sub(UnicodeWidthStr::width(name.as_str())) + 2,
            )));
            let mut details = tag.to_owned();
            if width.saturating_sub(name_width + 4) >= 24 {
                if let Some(description) = item.description.as_ref().filter(|d| !d.is_empty()) {
                    if !details.is_empty() {
                        details.push(' ');
                    }
                    details.push_str(description);
                }
            }
            spans.push(Span::styled(
                truncate_text(&details, width.saturating_sub(name_width + 4)),
                if selected {
                    Style::default().not_bold()
                } else {
                    Style::default().dim()
                },
            ));
            let target = Rect::new(body.x, body.y + (row - start) as u16, body.width, 1);
            self.state.hits.borrow_mut().push((target, row));
            if selected {
                buf.set_style(target, crate::style::selection_style().bold());
            }
            Line::from(spans).render(target, buf);
        }
        if inset > 0 && start > 0 {
            Line::from("↑").render(Rect::new(area.x, area.y, 1, 1), buf);
        }
        if inset > 0 && start + count < rows.len() {
            Line::from("↓").render(Rect::new(area.x, area.bottom() - 1, 1, 1), buf);
        }
    }
}
