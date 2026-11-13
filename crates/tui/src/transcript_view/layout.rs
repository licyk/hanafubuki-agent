// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Width-specific layouts, kept only for recently displayed entries.
//!
//! A layout is kept for an entry's revision, width and presentation: an
//! entry that changes in place is laid out again at its next revision, and
//! one that animates (a spinner, a timer) when its tick changes, at most
//! once a frame, so measuring and painting share it.

use std::sync::Weak;

use super::*;

const MAX_CACHED_ENTRIES: usize = 64;
const MAX_CACHED_LAYOUT_BYTES: usize = 8 * 1024 * 1024;

/// An activity's lines in the compact presentation, and what its details
/// would add.
pub(super) struct ActivityTranscriptLines {
    pub(super) activity: Vec<Line<'static>>,
    pub(super) disclosure: Option<ActivityDisclosure>,
}

/// The activity's lines with a control row that discloses or hides its
/// details, outside the selectable text.
pub(super) fn activity_layout(
    lines: ActivityTranscriptLines,
    width: u16,
    expanded: bool,
) -> TextLayout {
    let has_details = !lines.activity.is_empty() && (expanded || lines.disclosure.is_some());
    let layout = TextLayout::new(lines.activity, width);
    if !has_details {
        return layout;
    }
    let label = match (expanded, lines.disclosure) {
        (true, Some(ActivityDisclosure::OutputLines(_))) => "− Show less".to_owned(),
        (true, Some(ActivityDisclosure::Generic) | None) => "− Hide details".to_owned(),
        (false, Some(ActivityDisclosure::OutputLines(count))) => {
            let noun = if count == 1 { "line" } else { "lines" };
            format!("+ {count} {noun}")
        }
        (false, Some(ActivityDisclosure::Generic) | None) => "+ Show details".to_owned(),
    };
    layout.with_disclosure_control(label)
}

fn cell_content_layout(
    cell: &dyn HistoryCell,
    width: u16,
    mode: HistoryRenderMode,
    presentation: CellPresentation,
) -> TextLayout {
    if presentation.disclosure {
        activity_layout(
            ActivityTranscriptLines {
                activity: if presentation.expanded {
                    cell.expanded_lines(width)
                } else {
                    cell.compact_lines(width)
                },
                disclosure: cell.activity_disclosure(width),
            },
            width,
            presentation.expanded,
        )
    } else {
        TextLayout::new(
            transcript_entry_lines(
                cell,
                width,
                presentation.detailed,
                mode,
                presentation.retained,
            ),
            width,
        )
    }
}

/// A user prompt brings its own blank rows, so it needs no separator.
fn is_rich_user_prompt(cell: &dyn HistoryCell, mode: HistoryRenderMode) -> bool {
    mode == HistoryRenderMode::Rich && cell.as_any().is::<crate::history_cell::UserHistoryCell>()
}

#[derive(Default)]
pub(super) struct LayoutCache {
    entries: Vec<CachedLayout>,
    frame: u64,
}

struct CachedLayout {
    source: Weak<dyn HistoryCell>,
    revision: u64,
    width: u16,
    presentation: CellPresentation,
    layout: Arc<TextLayout>,
    rendered_frame: u64,
    animation_tick: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct CellPresentation {
    separated: bool,
    expanded: bool,
    disclosure: bool,
    detailed: bool,
    /// Laid out to be laid out again, not written into the scrollback.
    retained: bool,
}

impl TranscriptView {
    fn content_presentation(
        &mut self,
        cell: &dyn HistoryCell,
        full_content: bool,
        mode: HistoryRenderMode,
    ) -> CellPresentation {
        let ids = cell.activity_ids();
        let disclosure = !self.detailed && mode == HistoryRenderMode::Rich && !ids.is_empty();
        let manually_expanded = self.disclosure.is_expanded(&ids);
        if manually_expanded {
            self.disclosure.expanded.extend(ids);
        }
        CellPresentation {
            separated: false,
            expanded: disclosure && (full_content || manually_expanded),
            disclosure,
            detailed: full_content,
            retained: !self.scrollback_tail,
        }
    }

    /// The layout an entry is shown with: search's match, a pinned
    /// revision, or the current one.
    pub(super) fn layout(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        index: usize,
    ) -> Option<Arc<TextLayout>> {
        let snapshot = self.snapshot_cells();
        let cells = snapshot.as_deref().unwrap_or(cells);
        if index >= cells.len() {
            return None;
        }
        let key = self.entry_key(cells, index);
        if let Some(layout) = self.search.match_layout(key) {
            return Some(layout);
        }
        self.base_layout(cells, index)
    }

    /// The manual disclosure and retained revisions, without search's
    /// expansion of its match. Called with the snapshot's cells, if any.
    pub(super) fn base_layout(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        index: usize,
    ) -> Option<Arc<TextLayout>> {
        let key = self.entry_key(cells, index);
        if let Some(layout) = self
            .snapshot()
            .and_then(|snapshot| snapshot.pinned.get(&key))
        {
            return Some(Arc::clone(layout));
        }
        self.current_layout(cells, index)
    }

    /// The current revision, without a reader's retained snapshot.
    pub(super) fn current_layout(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        index: usize,
    ) -> Option<Arc<TextLayout>> {
        self.entry_layout(cells, index, self.detailed)
    }

    pub(super) fn entry_layout(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        index: usize,
        full_content: bool,
    ) -> Option<Arc<TextLayout>> {
        let width = self.area.width.max(1);
        let cell = cells.get(index)?;
        let mode = self.mode;
        let mut presentation = self.content_presentation(cell.as_ref(), full_content, mode);
        presentation.separated = (index > 0 || self.leading_separator)
            && !cell.is_stream_continuation()
            && !is_rich_user_prompt(cell.as_ref(), mode);
        Some(self.cache.get(cell, width, presentation, || {
            cell_content_layout(cell.as_ref(), width, mode, presentation)
        }))
    }
}

impl LayoutCache {
    pub(super) fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    pub(super) fn get(
        &mut self,
        cell: &Arc<dyn HistoryCell>,
        width: u16,
        presentation: CellPresentation,
        render: impl FnOnce() -> TextLayout,
    ) -> Arc<TextLayout> {
        let source = Arc::downgrade(cell);
        let revision = cell.revision();
        let animation_tick = cell.transcript_animation_tick();
        let frame = self.frame;
        let cached = self.entries.iter().position(|entry| {
            entry.width == width
                && entry.presentation == presentation
                && entry.source.ptr_eq(&source)
                && entry.revision == revision
                && (entry.rendered_frame == frame || entry.animation_tick == animation_tick)
        });
        if let Some(index) = cached {
            let layout = Arc::clone(&self.entries[index].layout);
            if index + 1 != self.entries.len() {
                let entry = self.entries.remove(index);
                self.entries.push(entry);
            }
            return layout;
        }
        let layout = render();
        let layout = Arc::new(if presentation.separated {
            layout.with_leading_separator()
        } else {
            layout
        });
        self.entries.retain(|entry| !entry.source.ptr_eq(&source));
        self.entries.push(CachedLayout {
            source,
            revision,
            width,
            presentation,
            layout: Arc::clone(&layout),
            rendered_frame: frame,
            animation_tick,
        });
        self.evict();
        layout
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    fn evict(&mut self) {
        let mut bytes = self
            .entries
            .iter()
            .map(|entry| entry.layout.text().len())
            .sum::<usize>();
        // A single oversized entry stays rather than be laid out again on
        // every frame it is visible.
        while self.entries.len() > 1
            && (self.entries.len() > MAX_CACHED_ENTRIES || bytes > MAX_CACHED_LAYOUT_BYTES)
        {
            bytes -= self.entries.remove(0).layout.text().len();
        }
    }
}

#[cfg(test)]
mod tests;
