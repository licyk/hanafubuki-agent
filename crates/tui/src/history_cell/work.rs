//! An independently disclosed activity, updated from backend snapshots.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ratatui::style::Stylize;
use ratatui::text::Line;

use super::{raw_lines_from_source, ActivityDisclosure, HistoryCell};
use crate::agent_bridge::WorkSnapshot;
use crate::render::line_utils::prefix_lines;
use crate::status_indicator_widget::fmt_elapsed_tenths;

#[derive(Debug)]
pub(crate) struct WorkCell {
    snapshot: WorkSnapshot,
    timed: bool,
}

impl WorkCell {
    pub(crate) fn new(snapshot: WorkSnapshot) -> Self {
        Self {
            snapshot,
            timed: true,
        }
    }

    pub(crate) fn replayed(item_id: u64, summary: String) -> Self {
        Self {
            snapshot: WorkSnapshot {
                id: format!("compaction:h{item_id}"),
                title: "Context compacted".to_string(),
                text: Some(summary),
                started_at: 0,
                completed_at: Some(0),
                warning: None,
                committed: false,
            },
            timed: false,
        }
    }

    pub(crate) fn finished(&self) -> bool {
        self.snapshot.completed_at.is_some()
    }

    pub(crate) fn update(&mut self, mut snapshot: WorkSnapshot) {
        if self.finished() {
            return;
        }
        if snapshot.text.as_ref().is_none_or(String::is_empty) {
            snapshot.text = self.snapshot.text.take();
        }
        self.snapshot = snapshot;
    }

    fn elapsed(&self) -> Duration {
        let end = self.snapshot.completed_at.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64
        });
        Duration::from_millis(end.saturating_sub(self.snapshot.started_at))
    }

    fn header(&self) -> Line<'static> {
        let duration = if self.timed {
            format!(" · {}", fmt_elapsed_tenths(self.elapsed()))
        } else {
            String::new()
        };
        Line::from(format!("• {}{duration}", self.snapshot.title).dim())
    }

    fn text(&self) -> &str {
        self.snapshot.text.as_deref().unwrap_or_default()
    }
}

impl HistoryCell for WorkCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        vec![self.header()]
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = vec![self.header()];
        let mut body = Vec::new();
        crate::markdown::append_markdown(
            self.text(),
            Some(usize::from(width.saturating_sub(2)).max(1)),
            None,
            &mut body,
        );
        lines.extend(prefix_lines(body, "  ".into(), "  ".into()));
        lines
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![self.header()];
        lines.extend(raw_lines_from_source(self.text()));
        lines
    }

    fn copy_source(&self) -> Option<String> {
        (!self.text().is_empty()).then(|| self.text().to_string())
    }

    fn activity_ids(&self) -> Vec<String> {
        if self.text().is_empty() {
            Vec::new()
        } else {
            vec![self.snapshot.id.clone()]
        }
    }

    fn activity_disclosure(&self, _width: u16) -> Option<ActivityDisclosure> {
        (!self.text().is_empty()).then_some(ActivityDisclosure::Generic)
    }

    fn transcript_animation_tick(&self) -> Option<u64> {
        (!self.finished()).then(|| (self.elapsed().as_millis() / 100) as u64)
    }
}

#[cfg(test)]
mod tests;
