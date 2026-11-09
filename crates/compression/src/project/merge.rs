// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the turn-integrity gate and the merge recovery ladder of
// `kernel/src/unit-projection.ts` of acp-kernel 0.0.189 at commit
// `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

use std::collections::{HashMap, HashSet};

use super::carve::filter_protected_tool_messages;
use super::integrity::withdrawals;
use super::{
    project_excluding, Draft, Fatal, FatalKind, ProjectionContext, UnitProjection, UnitRequest,
};
use crate::boundaries::BoundaryError;
use crate::refs::BLOCKED_REF;

/// The most stranded partners the ladder's second rung folds in.
const MAX_EXTENSION: usize = 16;
/// The most nested blocks the ladder's first rung excludes.
const MAX_EXCLUSIONS: usize = 64;

/// The turn-integrity gate. A fresh fold withdraws what would split a turn
/// or a tool pair. A merge (a unit consuming blocks) judges the coverage
/// after the fold and never unfolds an id whose only cover is a consumed
/// block; when it cannot settle, it tolerates a conflict it does not touch,
/// skips a merge whose turn can never close, excludes the offending blocks,
/// or folds the stranded partners in, before refusing.
pub(super) fn integrity_gate(
    mut draft: Draft,
    context: &ProjectionContext<'_>,
    request: &UnitRequest<'_>,
    excluded: &HashSet<String>,
    warnings: &mut Vec<String>,
) -> Result<UnitProjection, BoundaryError> {
    let state = context.state;
    let is_merge = !draft.consumed_block_ids.is_empty();
    let mut consumed_coverage: HashSet<String> = HashSet::new();
    let mut surviving_coverage: HashSet<String> = HashSet::new();
    if is_merge {
        for block in state.active_blocks() {
            let target = if draft.consumed_block_ids.contains(&block.block_id) {
                &mut consumed_coverage
            } else {
                &mut surviving_coverage
            };
            target.extend(block.effective_message_ids.iter().cloned());
        }
    }
    let mut folded: HashSet<String> = draft.effective.iter().map(str::to_owned).collect();
    if is_merge {
        folded.extend(surviving_coverage.iter().cloned());
    }
    // A fresh fold judges its own ids alone, so other blocks' coverage is
    // invisible: a covered call cannot strand its turn's reasoning.
    let initial = withdrawals(
        &context.integrity,
        &folded,
        (!is_merge).then_some(request.pre_existing_coverage),
    );
    draft.split_turn_count = initial.split_turn_count;
    draft.split_pair_count = initial.split_pair_count;
    if initial.withdrawn.is_empty() {
        return Ok(draft.finish(None));
    }
    let split = split_description(draft.split_turn_count, draft.split_pair_count);

    if !is_merge {
        let withdrawn: HashSet<&String> = initial.withdrawn.iter().collect();
        for id in &initial.withdrawn {
            draft.effective.remove(id);
        }
        let before = draft.filtered.len();
        draft.filtered.retain(|id| !withdrawn.contains(id));
        let removed = before - draft.filtered.len();
        draft.withdrawn = initial.withdrawn;
        if draft.filtered.is_empty() && draft.consumed_block_ids.is_empty() {
            return Ok(draft.finish(Some(Fatal {
                kind: FatalKind::Split,
                message: format!("Range would split {split} at the protected-zone boundary: a visible tool-call must keep its reasoning run and its results (strict providers reject a rebuilt request that lost either). Shrink the range to end before the turn starts, or wait until the whole turn ages out of the protected zone."),
            })));
        }
        warnings.push(format!(
            "Withdrawn {removed} message(s) from compression range to keep {split} intact (visible tool-call would lose its reasoning run or its results)."
        ));
        return Ok(draft.finish(None));
    }

    let unfoldable: HashSet<String> = draft
        .effective
        .iter()
        .filter(|id| !consumed_coverage.contains(*id))
        .map(str::to_owned)
        .collect();
    let mut unfolded: Vec<String> = Vec::new();
    let mut current = initial.withdrawn;
    let mut guard = unfoldable.len() + 2;
    let mut stuck = false;
    while !current.is_empty() && guard > 0 {
        guard -= 1;
        let mut progress = false;
        for id in &current {
            if unfoldable.contains(id) && !unfolded.contains(id) {
                unfolded.push(id.clone());
                progress = true;
            }
        }
        if !progress {
            stuck = true;
            break;
        }
        let mut recheck: HashSet<String> = draft.effective.iter().map(str::to_owned).collect();
        for id in &unfolded {
            recheck.remove(id);
        }
        recheck.extend(surviving_coverage.iter().cloned());
        current = withdrawals(&context.integrity, &recheck, None).withdrawn;
    }

    if stuck || !current.is_empty() {
        let conflict = Conflict {
            consumed: &consumed_coverage,
            surviving: &surviving_coverage,
            unfolded: &unfolded,
            current: &current,
        };
        draft = match settle_merge(draft, context, request, excluded, &conflict, warnings)? {
            Settled::Done(projection) => return Ok(projection),
            Settled::Continue(draft) => draft,
        };
    }

    let unfolded_set: HashSet<&String> = unfolded.iter().collect();
    for id in &unfolded {
        draft.effective.remove(id);
    }
    let before = draft.filtered.len();
    draft.filtered.retain(|id| !unfolded_set.contains(id));
    let removed = before - draft.filtered.len();
    draft.withdrawn = unfolded;
    if removed > 0 {
        warnings.push(format!(
            "Withdrew {removed} newly-folded message(s) from the merge to keep {split} intact while preserving the existing coverage of {}.",
            draft.consumed_block_ids.join(", ")
        ));
    }
    Ok(draft.finish(None))
}

/// What the merge's withdrawal loop left unsettled.
struct Conflict<'c> {
    /// The coverage of the blocks the merge consumes.
    consumed: &'c HashSet<String>,
    /// The coverage of the other active blocks.
    surviving: &'c HashSet<String>,
    unfolded: &'c [String],
    /// The ids still withdrawn.
    current: &'c [String],
}

enum Settled {
    Done(UnitProjection),
    /// The gate's common withdrawal finishes it: a tolerated conflict, or a
    /// fold extended over the stranded partners.
    Continue(Draft),
}

fn settle_merge(
    mut draft: Draft,
    context: &ProjectionContext<'_>,
    request: &UnitRequest<'_>,
    excluded: &HashSet<String>,
    conflict: &Conflict<'_>,
    warnings: &mut Vec<String>,
) -> Result<Settled, BoundaryError> {
    // A conflict binds this merge only when it touches what the merge newly
    // folds or re-owns; one that pre-dates it is inherited as it is.
    let merge_owned: HashSet<&String> = draft
        .filtered
        .iter()
        .chain(conflict.consumed.iter())
        .collect();
    let relevant: Vec<&String> = conflict
        .current
        .iter()
        .filter(|id| merge_owned.contains(id))
        .collect();
    if relevant.is_empty() {
        let refs: Vec<String> = conflict
            .current
            .iter()
            .take(6)
            .map(|id| context.ref_or_id(id))
            .collect();
        warnings.push(format!(
            "Inherited a pre-existing turn-integrity conflict this merge does not touch ({}): the partner(s) are already covered by other blocks or still visible. Left as-is — fold the visible partner(s) into coverage to clear it.",
            refs.join(", ")
        ));
        return Ok(Settled::Continue(draft));
    }

    let visible_stranded: Vec<String> = conflict
        .current
        .iter()
        .filter(|id| !conflict.surviving.contains(*id) && !conflict.consumed.contains(*id))
        .cloned()
        .collect();
    let never_foldable = |id: &String| is_never_foldable(id, context, request);
    let doomed: Vec<&String> = visible_stranded
        .iter()
        .filter(|id| never_foldable(id))
        .collect();
    if !doomed.is_empty() {
        // The turn can never close: skip the merge as an idempotent no-op.
        let foldable: Vec<String> = visible_stranded
            .iter()
            .filter(|id| !never_foldable(id))
            .map(|id| context.ref_or_id(id))
            .collect();
        let doomed_refs: Vec<String> = doomed.iter().map(|id| context.ref_or_id(id)).collect();
        let verb = if doomed.len() == 1 { "is" } else { "are" };
        let partners = if foldable.is_empty() {
            String::new()
        } else {
            format!(" The foldable partner(s) {} can only close their turn together with the protected member(s) — they stay visible until the protection lifts.", foldable.join(", "))
        };
        warnings.push(format!(
            "Merge of {} skipped — its turn cannot be closed now: {} {verb} protected (never foldable) while the rest of their turn is covered by block(s). Do not retry this merge.{partners}",
            draft.consumed_block_ids.join(", "),
            doomed_refs.join(", "),
        ));
        draft.consumed_block_ids.clear();
        draft.filtered.clear();
        return Ok(Settled::Done(draft.finish(None)));
    }

    // Rung 1: keep the blocks that own the conflict active and merge around
    // them. Rung 2: fold the stranded partners in so the turn closes.
    let mut retried = None;
    if excluded.len() < MAX_EXCLUSIONS {
        let offending = offending_blocks(&draft, context, conflict, &relevant);
        if !offending.is_empty() {
            let mut widened = excluded.clone();
            widened.extend(offending.iter().cloned());
            let projection = project_excluding(context, request, &widened, warnings)?;
            let folded_nothing = projection.fatal.is_none()
                && projection.filtered_ids.is_empty()
                && projection.consumed_block_ids.is_empty();
            if projection.fatal.is_none() && !folded_nothing {
                warnings.push(format!(
                    "Excluded {} from this merge — their coverage cuts through a turn group whose partner(s) are still visible outside the span, and unfolding them would drop their only coverage. They stay ACTIVE (coverage intact); the merge proceeded around them.",
                    offending.join(", ")
                ));
                return Ok(Settled::Done(projection));
            }
            retried = Some(projection);
        }
    }
    if extend_over_stranded(&mut draft, context, conflict, &visible_stranded, warnings) {
        return Ok(Settled::Continue(draft));
    }
    if let Some(retried) = retried.filter(|retried| retried.fatal.is_none()) {
        // Rung 1 folded nothing and rung 2 had nothing to fold in: the
        // idempotent no-op.
        return Ok(Settled::Done(retried));
    }
    let message = actionable_split_error(&draft.consumed_block_ids, conflict.current, context);
    Ok(Settled::Done(draft.finish(Some(Fatal {
        kind: FatalKind::Split,
        message,
    }))))
}

/// The consumed blocks that own the unresolved ids, in sorted order.
fn offending_blocks(
    draft: &Draft,
    context: &ProjectionContext<'_>,
    conflict: &Conflict<'_>,
    relevant: &[&String],
) -> Vec<String> {
    let mut owner: HashMap<&str, &str> = HashMap::new();
    for block in draft
        .consumed_block_ids
        .iter()
        .filter_map(|id| context.state.block(id))
    {
        for id in &block.effective_message_ids {
            owner.insert(id, &block.block_id);
        }
    }
    let mut offending: Vec<String> = relevant
        .iter()
        .filter(|id| !conflict.surviving.contains(**id))
        .filter_map(|id| owner.get(id.as_str()).map(|block| (*block).to_owned()))
        .collect::<HashSet<String>>()
        .into_iter()
        .collect();
    offending.sort();
    offending
}

/// Folds the stranded visible partners into the merge when the turn then
/// closes with nothing withdrawn from the fold.
fn extend_over_stranded(
    draft: &mut Draft,
    context: &ProjectionContext<'_>,
    conflict: &Conflict<'_>,
    visible_stranded: &[String],
    warnings: &mut Vec<String>,
) -> bool {
    if visible_stranded.is_empty() || visible_stranded.len() > MAX_EXTENSION {
        return false;
    }
    let mut extended: HashSet<String> = draft.effective.iter().map(str::to_owned).collect();
    for id in conflict.unfolded {
        extended.remove(id);
    }
    extended.extend(visible_stranded.iter().cloned());
    let mut check: HashSet<String> = extended
        .iter()
        .chain(conflict.surviving.iter())
        .cloned()
        .collect();
    let mut clean = false;
    for _ in 0..8 {
        let withdrawn = withdrawals(&context.integrity, &check, None).withdrawn;
        if withdrawn.is_empty() {
            clean = true;
            break;
        }
        if withdrawn.iter().any(|id| extended.contains(id)) {
            break;
        }
        let before = check.len();
        for id in &withdrawn {
            check.remove(id);
        }
        if check.len() == before {
            break;
        }
    }
    if !clean {
        return false;
    }
    for id in visible_stranded {
        draft.effective.insert(id);
        if !draft.direct.contains(id) {
            draft.direct.push(id.clone());
        }
        if !draft.filtered.contains(id) {
            draft.filtered.push(id.clone());
        }
    }
    let refs: Vec<String> = visible_stranded
        .iter()
        .map(|id| context.ref_or_id(id))
        .collect();
    warnings.push(format!(
        "Extended this merge to fold stranded partner(s) {} so the turn closes (billion-context#5919) — the new block's coverage includes them.",
        refs.join(", ")
    ));
    true
}

/// A partner that can never fold: blocked, gone, in the recent zone, a
/// protected tool message.
fn is_never_foldable(
    id: &String,
    context: &ProjectionContext<'_>,
    request: &UnitRequest<'_>,
) -> bool {
    let r#ref = context.ref_or_id(id);
    if r#ref == BLOCKED_REF || context.message(id).is_none() {
        return true;
    }
    if request
        .protected_refs
        .is_some_and(|zone| zone.contains(&r#ref))
    {
        return true;
    }
    filter_protected_tool_messages(std::slice::from_ref(id), context).is_empty()
}

/// The refusal for a merge the ladder could not complete: the conflicted
/// refs, who covers them, and the next move.
fn actionable_split_error(
    consumed: &[String],
    unresolved: &[String],
    context: &ProjectionContext<'_>,
) -> String {
    let mut consumed_owners: HashMap<&str, &str> = HashMap::new();
    let mut surviving_owners: HashMap<&str, &str> = HashMap::new();
    for block in context.state.active_blocks() {
        let owners = if consumed.contains(&block.block_id) {
            &mut consumed_owners
        } else {
            &mut surviving_owners
        };
        for id in &block.effective_message_ids {
            owners.insert(id, &block.block_id);
        }
    }
    let detail: Vec<String> = unresolved
        .iter()
        .take(6)
        .map(|id| {
            let r#ref = context.ref_or_id(id);
            match consumed_owners
                .get(id.as_str())
                .or_else(|| surviving_owners.get(id.as_str()))
            {
                Some(owner) => format!("{} (covered by {owner})", r#ref),
                None => format!("{} (visible)", r#ref),
            }
        })
        .collect();
    let mut surviving_offenders: Vec<&str> = Vec::new();
    for owner in unresolved
        .iter()
        .filter_map(|id| surviving_owners.get(id.as_str()))
    {
        if !surviving_offenders.contains(owner) {
            surviving_offenders.push(owner);
        }
    }
    let mut visible_refs: Vec<String> = unresolved
        .iter()
        .filter(|id| {
            !consumed_owners.contains_key(id.as_str())
                && !surviving_owners.contains_key(id.as_str())
        })
        .map(|id| context.ref_or_id(id))
        .collect();
    visible_refs.sort_by(|a, b| sort_number(a).cmp(&sort_number(b)).then_with(|| a.cmp(b)));
    visible_refs.truncate(6);
    let unblock = match (visible_refs.first(), visible_refs.last()) {
        (Some(first), Some(last)) => format!(" The exact unblock: one plain message-range compress over {first}..{last} (it may cover extra messages — that is fine), then retry this merge — the conflict clears once those partner(s) are covered."),
        _ => " The exact unblock: fold the stranded partner(s) listed above into coverage with a plain message-range compress, then retry this merge.".to_owned(),
    };
    let tail = if surviving_offenders.is_empty() {
        " Merge a sub-span that stops before or resumes after the offending block instead."
            .to_owned()
    } else {
        format!(
            " Block(s) {} stay active with their current coverage; this merge will not touch them.",
            surviving_offenders.join(", ")
        )
    };
    format!(
        "Merging {} cannot satisfy turn integrity without dropping existing coverage: folding would strand {}. Do not retry the same span — fold the stranded visible partner(s) first with a plain message-range compress so the turn closes.{unblock}{tail}",
        consumed.join(", "),
        detail.join(", "),
    )
}

/// `2 turn(s) and 1 tool call/result pair(s)`.
fn split_description(turns: usize, pairs: usize) -> String {
    let mut parts = Vec::new();
    if turns > 0 {
        parts.push(format!("{turns} turn(s)"));
    }
    if pairs > 0 {
        parts.push(format!("{pairs} tool call/result pair(s)"));
    }
    parts.join(" and ")
}

/// A ref's number for sorting; anything else sorts last.
fn sort_number(r#ref: &str) -> u64 {
    r#ref
        .strip_prefix('m')
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(u64::MAX)
}
