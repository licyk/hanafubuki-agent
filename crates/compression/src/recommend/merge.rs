// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `mergeRangesToThreshold` of `kernel/src/recommend.ts` of acp-kernel 0.0.189
// at commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

use super::CompressibleRange;
use crate::number::js_round;

/// Merges adjacent ranges into batches of at least `min_chars` characters,
/// the apply step's own measure, so no offered range is one the apply step
/// would refuse as too small. Every returned batch clears `min_chars` on its
/// own: a short tail joins the batch before it when contiguous, and is
/// dropped when nothing precedes it. A position gap (a block, protected or
/// pruned boundary) or a dead-ref hole closes a batch and is never bridged.
pub fn merge_ranges_to_threshold(
    ranges: Vec<CompressibleRange>,
    min_chars: usize,
) -> Vec<CompressibleRange> {
    if min_chars == 0 || ranges.is_empty() {
        return ranges;
    }
    let mut result: Vec<CompressibleRange> = Vec::new();
    let mut batch: Vec<CompressibleRange> = Vec::new();
    let mut batch_chars = 0;
    let mut close = |batch: &mut Vec<CompressibleRange>, batch_chars: &mut usize| {
        if !batch.is_empty() {
            if *batch_chars >= min_chars {
                result.push(merge_batch(batch));
            }
            batch.clear();
            *batch_chars = 0;
        }
    };
    for range in ranges {
        if range.no_merge_before {
            close(&mut batch, &mut batch_chars);
        }
        let gap = batch.last().is_some_and(|previous| {
            matches!((previous.end_index, range.start_index), (Some(end), Some(start)) if start > end + 1)
        });
        if gap {
            close(&mut batch, &mut batch_chars);
        }
        batch_chars += range.effective_chars();
        batch.push(range);
        if batch_chars >= min_chars {
            close(&mut batch, &mut batch_chars);
        }
    }
    drop(close);
    if let (Some(first), Some(previous)) = (batch.first(), result.last()) {
        let contiguous = !first.no_merge_before
            && match (previous.end_index, first.start_index) {
                (Some(end), Some(start)) => start <= end + 1,
                _ => true,
            };
        if contiguous {
            let previous = result.pop().unwrap_or_default();
            let mut tail = vec![previous];
            tail.append(&mut batch);
            result.push(merge_batch(&tail));
        } else if batch_chars >= min_chars {
            result.push(merge_batch(&batch));
        }
    }
    result
}

fn merge_batch(batch: &[CompressibleRange]) -> CompressibleRange {
    let first = &batch[0];
    let last = &batch[batch.len() - 1];
    let count: usize = batch.iter().map(|range| range.count).sum();
    let tool_share: f64 = batch
        .iter()
        .map(|range| f64::from(range.tool_pct) * range.count as f64)
        .sum();
    let tool_pct = js_round(tool_share / count as f64) as u32;
    let (start_index, end_index) = match (first.start_index, last.end_index) {
        (Some(_), Some(_)) => (
            batch.iter().filter_map(|range| range.start_index).min(),
            batch.iter().filter_map(|range| range.end_index).max(),
        ),
        _ => (None, None),
    };
    CompressibleRange {
        start_ref: first.start_ref.clone(),
        end_ref: last.end_ref.clone(),
        count,
        tokens: batch.iter().map(|range| range.tokens).sum(),
        chars: Some(batch.iter().map(CompressibleRange::effective_chars).sum()),
        tool_pct,
        text_pct: 100 - tool_pct,
        user_msgs: batch.iter().map(|range| range.user_msgs).sum(),
        start_index,
        end_index,
        // A leading dead-hole boundary stays one after the merge.
        no_merge_before: first.no_merge_before,
    }
}
