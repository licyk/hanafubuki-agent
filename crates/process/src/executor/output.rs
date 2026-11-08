//! Bounded, stream-tagged output buffer with monotonic byte offsets.
//!
//! Stores output chunks (each tagged with its [`OutputStream`]) in arrival order
//! with a global, monotonically increasing byte offset shared across streams.
//! This gives offset-based frontend polling with a single offset while still
//! preserving which stream each chunk came from. Oldest chunks are dropped once
//! the retained size exceeds the cap (long-running commands stay bounded).
//!
//! A PTY session's buffer also keeps its [`TerminalSizeTimeline`], trimmed with
//! the bytes, so every read can say which sizes its bytes were produced at.
//!
//! Readers push only whole UTF-8 scalars ([`Utf8Carry`]), trimming never keeps
//! the tail of a scalar it cut, a read never starts inside a scalar, and a read
//! capped by `max_bytes` ends on a scalar boundary, so a reader that decodes each read as text never sees a
//! character split between two reads.

use std::collections::VecDeque;

use super::model::{OutputDelta, OutputStream};
use crate::terminal_size::{TerminalResize, TerminalSize, TerminalSizeTimeline};

pub const DEFAULT_OUTPUT_CAP: usize = 4 * 1024 * 1024;

struct Chunk {
    stream: OutputStream,
    start: u64,
    end: u64,
    bytes: Vec<u8>,
}

pub struct OutputBuffer {
    chunks: VecDeque<Chunk>,
    retained: usize,
    cap: usize,
    next_offset: u64,
    /// Total bytes ever produced (including dropped) — the frontend can detect
    /// truncation by comparing a requested offset against the retained window.
    truncated_any: bool,
    /// Present for PTY output only: pipes have no grid.
    sizes: Option<TerminalSizeTimeline>,
}

impl OutputBuffer {
    pub fn new(cap: usize, terminal_size: Option<TerminalSize>) -> Self {
        Self {
            chunks: VecDeque::new(),
            retained: 0,
            cap: cap.max(1),
            next_offset: 0,
            truncated_any: false,
            sizes: terminal_size.map(TerminalSizeTimeline::new),
        }
    }

    /// The PTY's current size, or `None` for pipe output.
    pub fn terminal_size(&self) -> Option<TerminalSize> {
        self.sizes.as_ref().map(TerminalSizeTimeline::current)
    }

    /// Record that output from the next byte on is produced at `size`. Returns
    /// `false` when the size is unchanged or the output has no grid.
    ///
    /// The caller resizes the PTY while holding this buffer's lock, so no byte
    /// produced after the resize can be pushed ahead of its record.
    pub fn record_resize(&mut self, size: TerminalSize) -> bool {
        let offset = self.next_offset;
        self.sizes
            .as_mut()
            .is_some_and(|sizes| sizes.record(offset, size))
    }

    /// The size in effect at `start` and every change inside `start..end`, for
    /// a read that returned exactly that range.
    pub fn terminal_sizes(
        &self,
        start: u64,
        end: u64,
    ) -> (Option<TerminalSize>, Vec<TerminalResize>) {
        match &self.sizes {
            Some(sizes) => (
                Some(sizes.size_at(start)),
                sizes.changes_between(start, end),
            ),
            None => (None, Vec::new()),
        }
    }

    pub fn push(&mut self, stream: OutputStream, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let n = bytes.len();
        let orig_start = self.next_offset;
        let end = orig_start.saturating_add(n as u64);
        self.next_offset = end;

        // If a single chunk exceeds the cap, retain only its newest `cap` bytes.
        let (store, store_start) = if n > self.cap {
            self.truncated_any = true;
            let drop = skip_continuation_bytes(bytes, n - self.cap);
            (&bytes[drop..], orig_start + drop as u64)
        } else {
            (bytes, orig_start)
        };

        self.retained = self.retained.saturating_add(store.len());
        self.chunks.push_back(Chunk {
            stream,
            start: store_start,
            end,
            bytes: store.to_vec(),
        });

        // Trim oldest bytes (partially within a chunk) until within the cap, so
        // the newest tail is always preserved and stream tags stay correct.
        while self.retained > self.cap {
            let overflow = self.retained - self.cap;
            let front = self
                .chunks
                .front_mut()
                .expect("retained > 0 implies a chunk");
            if front.bytes.len() <= overflow {
                let popped = self.chunks.pop_front().expect("front exists");
                self.retained -= popped.bytes.len();
            } else {
                let drop = skip_continuation_bytes(&front.bytes, overflow);
                front.bytes.drain(0..drop);
                front.start += drop as u64;
                self.retained -= drop;
            }
            self.truncated_any = true;
        }
        let oldest = self.oldest_offset();
        if let Some(sizes) = self.sizes.as_mut() {
            sizes.trim_before(oldest);
        }
    }

    pub fn next_offset(&self) -> u64 {
        self.next_offset
    }

    fn oldest_offset(&self) -> u64 {
        self.chunks
            .front()
            .map(|chunk| chunk.start)
            .unwrap_or(self.next_offset)
    }

    /// Read output from `from` (default 0), capped at `max_bytes` total returned.
    /// Returns `(deltas, start_offset, next_offset, truncated)` where
    /// `start_offset` is where the returned data actually begins (clamped to the
    /// retained window) and `truncated` is true when `from` predated it.
    pub fn read_from(
        &self,
        from: u64,
        max_bytes: Option<usize>,
    ) -> (Vec<OutputDelta>, u64, u64, bool) {
        let oldest = self.oldest_offset();
        let truncated = from < oldest;
        let mut start = from.max(oldest);
        // A read that starts inside a scalar begins after it instead.
        if let Some(chunk) = self
            .chunks
            .iter()
            .find(|chunk| chunk.start <= start && start < chunk.end)
        {
            let index = (start - chunk.start) as usize;
            start += (skip_continuation_bytes(&chunk.bytes, index) - index) as u64;
        }
        let budget = max_bytes.unwrap_or(usize::MAX);

        let mut deltas: Vec<OutputDelta> = Vec::new();
        let mut produced: usize = 0;

        for chunk in &self.chunks {
            if produced >= budget || chunk.end <= start {
                continue;
            }
            // Absolute offset where this chunk's contribution begins. Chunks are
            // contiguous, so this is `start` for the chunk containing it and
            // `chunk.start` for later chunks.
            let chunk_from = start.max(chunk.start);
            let idx = (chunk_from - chunk.start) as usize;
            if idx >= chunk.bytes.len() {
                continue;
            }
            let available = &chunk.bytes[idx..];
            let mut take = available.len().min(budget - produced);
            let capped = take < available.len();
            if capped {
                // Stop before a scalar the budget would cut, unless the budget
                // cannot hold even one, so every read still makes progress.
                let complete = utf8_complete_len(&available[..take]);
                if complete > 0 || produced > 0 {
                    take = complete;
                }
            }
            if take == 0 {
                break;
            }
            let slice = &available[..take];
            produced += take;

            // Coalesce consecutive same-stream slices.
            match deltas.last_mut() {
                Some(last) if last.stream == chunk.stream => last.data.extend_from_slice(slice),
                _ => deltas.push(OutputDelta {
                    stream: chunk.stream,
                    data: slice.to_vec(),
                }),
            }
            if capped {
                break;
            }
        }

        let next = start + produced as u64;
        (deltas, start, next, truncated)
    }
}

/// Holds back an unfinished UTF-8 scalar at the end of each read, so every
/// chunk a reader pushes ends on a character boundary. Malformed bytes pass
/// through unchanged; only a valid but incomplete scalar (at most three bytes)
/// waits for the next read.
#[derive(Default)]
pub(crate) struct Utf8Carry {
    pending: Vec<u8>,
}

impl Utf8Carry {
    /// Pass the complete prefix of everything read so far to `sink`.
    pub(crate) fn push(&mut self, bytes: &[u8], mut sink: impl FnMut(&[u8])) {
        if self.pending.is_empty() {
            let complete = utf8_complete_len(bytes);
            sink(&bytes[..complete]);
            self.pending.extend_from_slice(&bytes[complete..]);
            return;
        }
        self.pending.extend_from_slice(bytes);
        let complete = utf8_complete_len(&self.pending);
        let ready: Vec<u8> = self.pending.drain(..complete).collect();
        sink(&ready);
    }

    /// Pass whatever is still held back once the stream has ended.
    pub(crate) fn finish(self, mut sink: impl FnMut(&[u8])) {
        sink(&self.pending);
    }
}

/// Length of the longest prefix of `bytes` that does not end inside a UTF-8
/// scalar. Malformed bytes count as complete, so at most three bytes of a
/// valid but unfinished scalar are excluded.
pub(crate) fn utf8_complete_len(bytes: &[u8]) -> usize {
    let mut boundary = bytes.len().saturating_sub(char::MAX.len_utf8() - 1);
    while boundary < bytes.len() {
        match std::str::from_utf8(&bytes[boundary..]) {
            Ok(text) => return boundary + text.len(),
            Err(error) => {
                boundary += error.valid_up_to();
                match error.error_len() {
                    Some(invalid) => boundary += invalid,
                    None => return boundary,
                }
            }
        }
    }
    bytes.len()
}

/// Move a trim point past the continuation bytes of a scalar it cut (at most
/// three), so retained output never starts with the tail of a character.
fn skip_continuation_bytes(bytes: &[u8], from: usize) -> usize {
    let mut index = from;
    while index < bytes.len()
        && index - from < char::MAX.len_utf8() - 1
        && bytes[index] & 0xC0 == 0x80
    {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests;
