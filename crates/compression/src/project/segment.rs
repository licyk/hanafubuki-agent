// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/segment.ts` and `kernel/src/viable.ts` of acp-kernel 0.0.189
// at commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

/// An item of a range view, segmented by [`segment_groups`].
pub trait SegmentItem {
    /// A numbered message was skipped between this item and the previous one
    /// in the source array (array positions, never ref arithmetic).
    fn gap_before(&self) -> bool;
    /// A user message: a turn boundary candidate.
    fn is_user(&self) -> bool;
}

/// Groups items into contiguous segments: a gap always starts a new one,
/// and a user message does once the current one holds three items.
pub fn segment_groups<T: SegmentItem + Clone>(items: &[T]) -> Vec<Vec<T>> {
    let mut groups: Vec<Vec<T>> = Vec::new();
    let mut current: Vec<T> = Vec::new();
    for item in items {
        if !current.is_empty() && ((item.is_user() && current.len() >= 3) || item.gap_before()) {
            groups.push(std::mem::take(&mut current));
        }
        current.push(item.clone());
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

/// The smallest range worth recommending: below it the model cannot write a
/// meaningful summary, and one such range fails a whole batch.
pub const VIABLE_RANGE_MIN_TOKENS: u64 = 200;

/// The ranges of at least [`VIABLE_RANGE_MIN_TOKENS`] tokens.
pub fn viable_ranges<T>(ranges: Vec<T>, tokens: impl Fn(&T) -> u64) -> Vec<T> {
    ranges
        .into_iter()
        .filter(|range| tokens(range) >= VIABLE_RANGE_MIN_TOKENS)
        .collect()
}
