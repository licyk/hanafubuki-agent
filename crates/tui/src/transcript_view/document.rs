// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

//! The file reader shares the transcript's wrapped rows, selection and search.
use super::*;
use crate::sidebar_files::FileTarget;
use ratatui::style::{Modifier, Stylize};

pub(crate) struct DocumentLink {
    pub(crate) index: usize,
    pub(crate) target: FileTarget,
    range: std::ops::Range<usize>,
}
impl TranscriptView {
    pub(crate) fn file_link_at(
        &self,
        column: u16,
        row: u16,
    ) -> Option<crate::sidebar_files::FileTarget> {
        if !self.area.contains((column, row).into()) {
            return None;
        }
        let hit = self.visible.get(usize::from(row - self.area.y))?;
        let line = hit.layout.row_line(hit.row);
        line.hyperlinks
            .iter()
            .find(|link| link.columns.contains(&usize::from(column - self.area.x)))
            .and_then(|link| crate::sidebar_files::FileTarget::link(&link.destination))
    }

    pub(crate) fn step_document_link(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        current: Option<usize>,
        forward: bool,
    ) -> Option<DocumentLink> {
        let layout = self.layout(cells, 0)?;
        let mut links: Vec<(std::ops::Range<usize>, crate::sidebar_files::FileTarget)> = Vec::new();
        for row in 0..layout.row_count() {
            for link in layout.row_line(row).hyperlinks {
                if let Some(target) = crate::sidebar_files::FileTarget::link(&link.destination) {
                    let start = layout.position_at(row, link.columns.start as u16);
                    let end = layout.position_at(row, link.columns.end as u16);
                    if let Some((last, _)) = links
                        .last_mut()
                        .filter(|(last, prior)| last.end == start && *prior == target)
                    {
                        last.end = end;
                    } else {
                        links.push((start..end, target));
                    }
                }
            }
        }
        if links.is_empty() {
            return None;
        }
        let index = match (current, forward) {
            (None, true) => 0,
            (None, false) => links.len() - 1,
            (Some(i), true) => (i + 1) % links.len(),
            (Some(i), false) => (i + links.len() - 1) % links.len(),
        };
        let (range, target) = links.swap_remove(index);
        if !self
            .visible
            .iter()
            .any(|v| v.row == layout.row_for_offset(range.start))
        {
            self.jump_to_document_offset(cells, range.start);
        }
        Some(DocumentLink {
            index,
            range,
            target,
        })
    }

    pub(crate) fn document_offset(&self) -> Option<usize> {
        match self.position {
            Position::Latest => None,
            Position::Reading(anchor) => Some(anchor.offset),
        }
    }

    pub(crate) fn jump_to_document_offset(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        offset: usize,
    ) {
        self.jump_to_entry(cells, 0);
        if let Position::Reading(anchor) = &mut self.position {
            anchor.offset = offset;
        }
    }

    pub(crate) fn render_document(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        cells: &[Arc<dyn HistoryCell>],
        line_starts: &[usize],
        target: Option<(usize, usize)>,
        link: Option<&DocumentLink>,
    ) {
        let digits = line_starts
            .len()
            .checked_ilog10()
            .map_or(0, |n| n as u16 + 1);
        let gutter = if digits > 0 && area.width > 40 + digits {
            digits + 1
        } else {
            0
        };
        let text = Rect::new(area.x + gutter, area.y, area.width - gutter, area.height);
        self.render(text, buf, cells);
        if let Some(link) = link {
            for (y, visible) in (text.y..text.bottom()).zip(&self.visible) {
                visible.layout.highlight(
                    link.range.clone(),
                    Rect::new(text.x, y, text.width, 1),
                    buf,
                    visible.row,
                );
            }
        }
        if line_starts.is_empty() {
            return;
        }
        for (y, visible) in (area.top()..area.bottom()).zip(&self.visible) {
            let offset = visible.layout.position_at(visible.row, /*column*/ 0);
            let line = line_starts.partition_point(|start| *start <= offset);
            if gutter > 0 && line > 0 && line_starts[line - 1] == offset {
                Line::from(line.to_string())
                    .dim()
                    .right_aligned()
                    .render(Rect::new(area.x, y, digits, /*height*/ 1), buf);
            }
            if let Some((_, target_offset)) = target.filter(|(target_line, _)| line == *target_line)
            {
                let mut style = crate::style::user_message_style();
                if style.bg.is_none() {
                    style = crate::style::selection_style();
                }
                buf.set_style(Rect::new(text.x, y, text.width, /*height*/ 1), style);
                if visible.row == visible.layout.row_for_offset(target_offset) {
                    let x = text
                        .x
                        .saturating_add(visible.layout.column_for_offset(target_offset));
                    if x < text.right() {
                        let cell = &mut buf[(x, y)];
                        let style = cell.style();
                        cell.set_style(if style.add_modifier.contains(Modifier::REVERSED) {
                            style.not_reversed()
                        } else {
                            style.reversed()
                        });
                    }
                }
            }
        }
    }

    pub(crate) fn paint_document_scroll_indicators(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        cells: &[Arc<dyn HistoryCell>],
    ) {
        if area.height < 2 || area.width < 3 {
            return;
        }
        let start = self.start(cells);
        for (visible, row, arrow) in [
            (start > (0, 0), area.top(), "↑"),
            (start < self.bottom_start(cells), area.bottom() - 1, "↓"),
        ] {
            if visible {
                buf[(area.right() - 1, row)]
                    .set_symbol(arrow)
                    .set_style(ratatui::style::Style::default().dim());
            }
        }
    }
}
