// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Scanning for the query a bounded piece at a time, and showing a match:
//! only the current match gets an expanded layout (its activity's
//! details), and a reader's retained revisions keep theirs.

use super::*;

pub(super) const SCAN_BYTES: usize = 16 * 1024;
const ENTRIES_PER_FRAME: usize = 8;

impl TranscriptView {
    /// Scans on; `true` while another frame can make progress, so the
    /// caller schedules one.
    pub(crate) fn advance_search(&mut self, cells: &[Arc<dyn HistoryCell>]) -> bool {
        if self.selection.is_some() && self.select_mode.is_none() {
            return false;
        }
        let current_cells = cells;
        let snapshot = self.snapshot_cells();
        let cells = snapshot.as_deref().unwrap_or(cells);
        if matches!(self.search.progress, Progress::Restart) {
            self.search.progress = Progress::Idle;
            // A refined query does not leave an obsolete match at the top
            // while history loads.
            self.position = self.search.saved_position;
            if self.search.editor.is_empty() {
                self.held_reading = self.search.saved_snapshot.clone();
                self.rewrap_snapshot(self.area.width);
                return false;
            }
            self.start_search_scan(cells, Direction::Older);
        }
        if matches!(self.search.progress, Progress::AwaitingHistory)
            && matches!(
                self.history,
                TranscriptHistoryState::Complete | TranscriptHistoryState::Idle
            )
        {
            self.search.progress = Progress::Exhausted;
        }
        let mut scanned_bytes = 0;
        for _ in 0..ENTRIES_PER_FRAME {
            let Progress::Scanning(mut cursor) = self.search.progress else {
                return false;
            };
            cursor.anchor.index = self.resolve(cells, cursor.anchor);
            if self.scanned_entry_changed(cells, cursor.anchor) {
                // Its offsets belong to the revision the scan began in.
                self.restart_search();
                return matches!(self.search.progress, Progress::Restart);
            }
            let Some(layout) = self.search_layout(cells, cursor.anchor) else {
                self.advance_search_cursor(cells, cursor, 0..0, 0);
                return false;
            };
            let text = layout.text();
            cursor.anchor.offset = text.floor_char_boundary(cursor.anchor.offset.min(text.len()));
            let window = scan_window(text, cursor, self.search.folded_query.len());
            if let Some(range) = find_literal(
                &text[window.clone()],
                &self.search.folded_query,
                cursor.direction,
            ) {
                let anchor = Anchor {
                    offset: window.start + range.start,
                    ..cursor.anchor
                };
                self.install_search_match(current_cells, anchor, window.start + range.end, layout);
                return false;
            }
            scanned_bytes += window.len();
            self.advance_search_cursor(cells, cursor, window, text.len());
            // Short entries are batched, but a frame never scans an
            // unbounded transcript.
            if scanned_bytes >= SCAN_BYTES {
                break;
            }
        }
        matches!(self.search.progress, Progress::Scanning(_))
    }

    /// An older page arrived at `inserted`: a scan waiting for it goes on
    /// through it.
    pub(in crate::transcript_view) fn search_history_loaded(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        inserted: Range<usize>,
    ) {
        let index = inserted.end - 1;
        let cursor = Cursor {
            anchor: Anchor {
                key: self.entry_key(cells, index),
                index,
                offset: usize::MAX,
                row_bias: 0,
            },
            direction: Direction::Older,
        };
        let page_start = self.entry_key(cells, inserted.start);
        if let Some(saved) = &mut self.search.saved_select {
            if matches!(
                saved.progress,
                Progress::AwaitingHistory
                    | Progress::Scanning(Cursor {
                        direction: Direction::Older,
                        ..
                    })
            ) {
                if matches!(saved.progress, Progress::AwaitingHistory) {
                    saved.progress = Progress::Scanning(cursor);
                }
                saved.page_start = Some(page_start);
            }
        }
        if !matches!(self.search.progress, Progress::AwaitingHistory) {
            return;
        }
        self.search.progress = Progress::Scanning(cursor);
        self.search.scanning_layout = None;
        self.search.page_start = Some(page_start);
    }

    pub(in crate::transcript_view) fn render_search(&self, buf: &mut Buffer) {
        // A selected range takes precedence until the selection ends.
        if !self.search.is_active() || self.has_selection_range() {
            return;
        }
        let Some(found) = &self.search.current else {
            return;
        };
        for (y, visible) in self.visible.iter().enumerate() {
            if visible.key == found.anchor.key {
                let area = Rect::new(self.area.x, self.area.y + y as u16, self.area.width, 1);
                visible
                    .layout
                    .highlight(found.anchor.offset..found.end, area, buf, visible.row);
            }
        }
    }

    pub(in crate::transcript_view) fn is_search_expanded(&self, key: EntryKey) -> bool {
        self.search
            .match_anchor()
            .is_some_and(|anchor| anchor.key == key)
    }

    /// From a match: on to the next one either way. Confirming a match
    /// does not reverse the first scan through older history.
    pub(super) fn next_search_match(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        direction: Direction,
    ) {
        if self.search.editor.is_empty() {
            return;
        }
        if matches!(self.search.progress, Progress::AwaitingHistory)
            && self.history == TranscriptHistoryState::Failed
            && direction == Direction::Older
        {
            self.history = TranscriptHistoryState::Partial;
        } else if let Some(found) = &self.search.current {
            self.search.scanning_layout = None;
            let offset = match direction {
                Direction::Older => found.anchor.offset,
                Direction::Newer => found.end,
            };
            let mut anchor = Anchor {
                offset,
                ..found.anchor
            };
            if direction == Direction::Newer && self.held_reading.is_some() {
                match self.rejoin_from_held_match(cells, anchor) {
                    Some(rejoined) => anchor = rejoined,
                    None => return,
                }
            }
            self.search.progress = Progress::Scanning(Cursor { anchor, direction });
        } else if matches!(self.search.progress, Progress::Idle | Progress::Exhausted) {
            self.start_search_scan(cells, direction);
        }
    }

    /// Searching on from a match in a held revision continues in the cell
    /// as it is now; `None` when the cell is gone and the search starts
    /// over.
    fn rejoin_from_held_match(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        anchor: Anchor,
    ) -> Option<Anchor> {
        self.held_reading.take()?;
        let previous = self.search.current.take()?;
        let Some(index) = cells
            .iter()
            .position(|cell| EntryKey::cell(cell) == anchor.key)
        else {
            self.restart_search();
            self.visible.clear();
            return None;
        };
        let key = anchor.key;
        let layout = self.search_layout(
            cells,
            Anchor {
                key,
                index,
                ..anchor
            },
        );
        let same_prefix = layout.as_ref().is_some_and(|layout| {
            previous
                .layout
                .text()
                .get(..anchor.offset)
                .is_some_and(|prefix| layout.text().starts_with(prefix))
        });
        let rejoined = Anchor {
            key,
            index,
            offset: if same_prefix { anchor.offset } else { 0 },
            row_bias: 0,
        };
        self.search.current = layout.filter(|_| same_prefix).map(|layout| Match {
            anchor: Anchor {
                offset: previous.anchor.offset,
                ..rejoined
            },
            end: previous.end,
            layout,
        });
        self.visible.clear();
        self.position = Position::Reading(
            self.search
                .current
                .as_ref()
                .map_or(rejoined, |found| found.anchor),
        );
        if same_prefix {
            if let Some(layout) = self.current_layout(cells, index) {
                self.hold_live_reading(cells, layout);
            }
        }
        Some(rejoined)
    }

    fn start_search_scan(&mut self, cells: &[Arc<dyn HistoryCell>], direction: Direction) {
        self.search.page_start = None;
        let count = cells.len();
        let index = if self.search.is_reading() {
            // Without a match, from the visible entry, not an end.
            self.start(cells).0
        } else {
            match direction {
                Direction::Older => count.saturating_sub(1),
                Direction::Newer => 0,
            }
        };
        let offset = match direction {
            Direction::Older => usize::MAX,
            Direction::Newer => 0,
        };
        self.search.progress = Progress::Scanning(Cursor {
            anchor: Anchor {
                key: self.entry_key(cells, index),
                index,
                offset,
                row_bias: 0,
            },
            direction,
        });
    }

    fn advance_search_cursor(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        cursor: Cursor,
        window: Range<usize>,
        text_len: usize,
    ) {
        let remaining = match cursor.direction {
            Direction::Older => window.start > 0,
            Direction::Newer => window.end < text_len,
        };
        if remaining {
            let offset = match cursor.direction {
                Direction::Older => cursor.anchor.offset.saturating_sub(SCAN_BYTES),
                Direction::Newer => cursor.anchor.offset.saturating_add(SCAN_BYTES),
            };
            self.search.progress = Progress::Scanning(Cursor {
                anchor: Anchor {
                    offset,
                    ..cursor.anchor
                },
                ..cursor
            });
            return;
        }
        self.search.scanning_layout = None;
        let count = cells.len();
        let next = match cursor.direction {
            Direction::Older if self.search.page_start == Some(cursor.anchor.key) => None,
            Direction::Older => cursor.anchor.index.checked_sub(1),
            Direction::Newer => {
                (cursor.anchor.index + 1 < count).then_some(cursor.anchor.index + 1)
            }
        };
        self.search.progress = match next {
            Some(index) => Progress::Scanning(Cursor {
                anchor: Anchor {
                    key: self.entry_key(cells, index),
                    index,
                    offset: if cursor.direction == Direction::Older {
                        usize::MAX
                    } else {
                        0
                    },
                    row_bias: 0,
                },
                direction: cursor.direction,
            }),
            None if cursor.direction == Direction::Older
                && matches!(
                    self.history,
                    TranscriptHistoryState::LoadingOlder
                        | TranscriptHistoryState::LoadingBeginning
                        | TranscriptHistoryState::Partial
                        | TranscriptHistoryState::Failed
                ) =>
            {
                Progress::AwaitingHistory
            }
            None => Progress::Exhausted,
        };
    }

    /// Shows a match with a third of the view of context above it, keeping
    /// the presentation it had before the match revealed it.
    fn install_search_match(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        anchor: Anchor,
        end: usize,
        layout: Arc<TextLayout>,
    ) {
        let current_cells = cells;
        let snapshot = self.snapshot_cells();
        let cells = snapshot.as_deref().unwrap_or(cells);
        let base = self
            .base_layout(cells, anchor.index)
            .unwrap_or_else(|| Arc::new(TextLayout::new(Vec::new(), self.area.width)));
        self.position = Position::Reading(anchor);
        self.hold_live_reading(current_cells, base);
        self.search.current = Some(Match {
            anchor,
            end,
            layout: Arc::clone(&layout),
        });
        if self.search.is_reading() {
            self.move_select_cursor_to_search_match(anchor, &layout, current_cells);
        }
        let row = layout.row_for_offset(anchor.offset);
        let match_rows = layout.row_for_offset(end - 1) - row + 1;
        let context_rows = usize::from(self.area.height / 3)
            .min(usize::from(self.area.height).saturating_sub(match_rows));
        let (index, row) = self.move_rows(cells, anchor.index, row, -(context_rows as isize));
        if let Some(context) = self.layout(cells, index) {
            let offset = context.position_at(row, 0);
            self.position = Position::Reading(Anchor {
                key: self.entry_key(cells, index),
                index,
                offset,
                row_bias: context.row_for_offset(offset) as isize - row as isize,
            });
        }
        self.search.progress = Progress::Found;
        self.search.scanning_layout = None;
    }

    /// The text search reads for an entry: its full content, whatever the
    /// compact presentation shows.
    fn search_layout(
        &mut self,
        cells: &[Arc<dyn HistoryCell>],
        anchor: Anchor,
    ) -> Option<Arc<TextLayout>> {
        if let Some((key, _, layout)) = &self.search.scanning_layout {
            if *key == anchor.key {
                return Some(Arc::clone(layout));
            }
        }
        let layout = if let Some(layout) = self.search.match_layout(anchor.key) {
            layout
        } else if self.detailed {
            self.layout(cells, anchor.index)?
        } else {
            // A candidate replaces the current match when found, so its
            // spacing does not depend on an expansion about to go.
            let current = self.search.current.take();
            let layout = self.entry_layout(cells, anchor.index, true);
            self.search.current = current;
            layout?
        };
        let revision = cells.get(anchor.index).map_or(0, |cell| cell.revision());
        self.search.scanning_layout = Some((anchor.key, revision, Arc::clone(&layout)));
        Some(layout)
    }

    /// Whether the entry being scanned changed in place since the scan
    /// read it; a reader's retained revision does not.
    fn scanned_entry_changed(&self, cells: &[Arc<dyn HistoryCell>], anchor: Anchor) -> bool {
        self.held_reading.is_none()
            && self
                .search
                .scanning_layout
                .as_ref()
                .is_some_and(|(key, revision, _)| {
                    *key == anchor.key
                        && cells
                            .get(anchor.index)
                            .is_some_and(|cell| cell.revision() != *revision)
                })
    }
}

fn scan_window(text: &str, cursor: Cursor, query_len: usize) -> Range<usize> {
    // A character can take four bytes while its lowercase form takes one.
    let overlap = query_len.saturating_mul(4);
    let extent = SCAN_BYTES.saturating_add(overlap);
    match cursor.direction {
        Direction::Older => {
            text.floor_char_boundary(cursor.anchor.offset.saturating_sub(extent))
                ..cursor.anchor.offset
        }
        Direction::Newer => {
            cursor.anchor.offset
                ..text.floor_char_boundary(
                    cursor.anchor.offset.saturating_add(extent).min(text.len()),
                )
        }
    }
}

/// The first (newer) or last (older) case-insensitive occurrence of the
/// lowercased `query` in `text`, as a byte range of `text`.
pub(super) fn find_literal(text: &str, query: &str, direction: Direction) -> Option<Range<usize>> {
    let mut folded = String::new();
    let mut spans = Vec::new();
    for (start, character) in text.char_indices() {
        for lowercase in character.to_lowercase() {
            folded.push(lowercase);
            spans.push((folded.len(), start..start + character.len_utf8()));
        }
    }
    let start = match direction {
        Direction::Older => folded.rfind(query),
        Direction::Newer => folded.find(query),
    }?;
    let first = spans.partition_point(|(end, _)| *end <= start);
    let last = spans.partition_point(|(end, _)| *end < start + query.len());
    Some(spans.get(first)?.1.start..spans.get(last)?.1.end)
}
