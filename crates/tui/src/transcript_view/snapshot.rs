// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The displayed revisions a selection or a reader keeps while the cells
//! and the live tail move on. Pinned layouts keep manual disclosures;
//! search's expansion of its match is applied only when painting.

use std::collections::HashMap;

use super::*;

#[derive(Clone)]
pub(super) struct ViewSnapshot {
    pub(super) cells: Arc<[Arc<dyn HistoryCell>]>,
    pub(super) pinned: HashMap<EntryKey, Arc<TextLayout>>,
    pub(super) activities: HashMap<EntryKey, Arc<[String]>>,
    /// The live tail's full text, which its compact display cannot rebuild.
    pub(super) search_live: Option<Arc<TextLayout>>,
}

impl TranscriptView {
    pub(super) fn snapshot(&self) -> Option<&ViewSnapshot> {
        self.selection
            .as_ref()
            .map(|selection| &selection.snapshot)
            .or(self.held_reading.as_ref())
    }

    pub(super) fn snapshot_mut(&mut self) -> Option<&mut ViewSnapshot> {
        self.selection
            .as_mut()
            .map(|selection| &mut selection.snapshot)
            .or(self.held_reading.as_mut())
    }

    /// The snapshot's cells, as a handle cloned before the view's state
    /// changes, without copying the cells on each frame.
    pub(super) fn snapshot_cells(&self) -> Option<Arc<[Arc<dyn HistoryCell>]>> {
        self.snapshot().map(|snapshot| Arc::clone(&snapshot.cells))
    }

    pub(super) fn capture_snapshot(&mut self, cells: &[Arc<dyn HistoryCell>]) -> ViewSnapshot {
        let cells = self.snapshot_cells().unwrap_or_else(|| Arc::from(cells));
        let mut pinned = self
            .snapshot()
            .map(|snapshot| snapshot.pinned.clone())
            .unwrap_or_default();
        let mut activities = self
            .snapshot()
            .map(|snapshot| snapshot.activities.clone())
            .unwrap_or_default();
        pinned.extend(
            self.visible
                .iter()
                .map(|visible| (visible.key, Arc::clone(&visible.layout))),
        );
        activities.extend(
            self.visible
                .iter()
                .map(|visible| (visible.key, Arc::clone(&visible.activity_ids))),
        );
        if let Some(live) = self.base_layout(&cells, cells.len()) {
            pinned.insert(EntryKey::Live, live);
            activities.insert(
                EntryKey::Live,
                self.displayed_activity_ids(&cells, cells.len()),
            );
        }
        // `visible` may hold the expanded match: its underlying revision is
        // pinned even when the match is off screen, so a selection never
        // keeps a temporary expansion.
        if let Some(anchor) = self.search.match_anchor() {
            let index = self.resolve(&cells, anchor);
            let base = self
                .base_layout(&cells, index)
                .unwrap_or_else(|| Arc::new(TextLayout::new(Vec::new(), self.area.width)));
            pinned.insert(anchor.key, base);
        }
        ViewSnapshot {
            cells,
            pinned,
            activities,
            search_live: self
                .snapshot()
                .and_then(|snapshot| snapshot.search_live.clone())
                .or_else(|| self.search.live.clone()),
        }
    }

    /// Keeps the revision of a live or changing cell while the reader is
    /// inside it; scrolling rejoins the current cells when it reaches one
    /// that is still there.
    pub(super) fn hold_live_reading(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        layout: Arc<TextLayout>,
    ) {
        if self.selection.is_some() {
            return;
        }
        let Position::Reading(anchor) = self.position else {
            self.release_live_reading();
            return;
        };
        let mutable_cell = cells.get(anchor.index).is_some_and(|cell| {
            EntryKey::cell(cell) == anchor.key
                && (!cell.has_stable_transcript_height()
                    || cell.transcript_animation_tick().is_some())
        });
        if anchor.key == EntryKey::Live || mutable_cell {
            let already_held = self
                .held_reading
                .as_ref()
                .is_some_and(|snapshot| snapshot.pinned.contains_key(&anchor.key));
            if !already_held {
                self.held_reading = None;
                let mut snapshot = self.capture_snapshot(cells);
                if !self.is_search_expanded(anchor.key) {
                    snapshot.pinned.insert(anchor.key, layout);
                }
                self.held_reading = Some(snapshot);
            }
        } else if self.held_reading.is_some() {
            if let Some(index) = cells
                .iter()
                .position(|cell| EntryKey::cell(cell) == anchor.key)
            {
                self.release_live_reading();
                self.position = Position::Reading(Anchor { index, ..anchor });
            }
        }
    }

    /// Search's offsets belong to the held revision; they are refreshed
    /// when it is released.
    pub(super) fn release_live_reading(&mut self) {
        if self.held_reading.take().is_some() {
            self.invalidate_held_search();
        }
    }

    pub(super) fn rewrap_snapshot(&mut self, width: u16) {
        if let Some(snapshot) = self.snapshot_mut() {
            for layout in snapshot.pinned.values_mut() {
                *layout = Arc::new(layout.rewrap(width));
            }
            if let Some(layout) = &mut snapshot.search_live {
                *layout = Arc::new(layout.rewrap(width));
            }
        }
    }
}
