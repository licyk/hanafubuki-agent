// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `screenRecommendedUnits` of `kernel/src/recommend.ts` of acp-kernel 0.0.189
// at commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

use std::collections::HashSet;

use super::CompressibleRange;
use crate::apply::{evaluate_refold, RefoldDecision};
use crate::model::{Config, Message, State};
use crate::project::{project_unit, ProjectionContext, UnitRequest};

/// Keeps the units the apply step would fold, judged by the same
/// projection on `source`, the array it runs on: a unit only in the
/// protected zone, one that would split a turn or a pair, one with nothing
/// new to fold, or a message range that only consumes blocks without being
/// a refold in place would be refused every time it was offered. Dropped
/// units are not merged again, so the offer cannot oscillate.
pub fn screen_recommended_units(
    units: Vec<CompressibleRange>,
    source: &[Message],
    state: &State,
    config: &Config,
    protected_refs: &HashSet<String>,
) -> Vec<CompressibleRange> {
    if units.is_empty() {
        return units;
    }
    let coverage: HashSet<String> = state
        .covered_message_ids()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let context = ProjectionContext::new(source, state, config);
    units
        .into_iter()
        .filter(|unit| {
            let request = UnitRequest {
                start_ref: &unit.start_ref,
                end_ref: &unit.end_ref,
                pre_existing_coverage: &coverage,
                protected_refs: Some(protected_refs),
            };
            let Ok(projection) = project_unit(&context, &request, &mut Vec::new()) else {
                return false;
            };
            if projection.fatal.is_some() {
                return false;
            }
            if projection.filtered_ids.is_empty() {
                if projection.consumed_block_ids.is_empty() {
                    return false;
                }
                // A message range consuming blocks folds only as a refold in
                // place; a block range takes the apply step's block route.
                if !projection.is_block_boundary
                    && !matches!(
                        evaluate_refold(state, &unit.start_ref, &unit.end_ref),
                        RefoldDecision::Refold(_)
                    )
                {
                    return false;
                }
            }
            true
        })
        .collect()
}
