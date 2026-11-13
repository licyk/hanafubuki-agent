// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! One commit tick over the streams of an answer and of a plan: their
//! queues' pressure, the chunking policy's decision, and the rows each
//! commits. Rows are drained from the queues' fronts only; scheduling the
//! ticks and inserting the cells are the caller's.

use std::time::{Duration, Instant};

use super::chunking::{AdaptiveChunkingPolicy, ChunkingMode, DrainPlan, QueueSnapshot};
use super::controller::{PlanStreamController, StreamController};
use crate::history_cell::HistoryCell;

/// Which ticks commit rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommitTickScope {
    /// The animation's tick: always.
    AnyMode,
    /// After a delta queued rows: the policy decides, and rows are
    /// committed only in catch-up.
    CatchUpOnly,
}

/// What a tick committed.
#[derive(Default)]
pub(crate) struct CommitTickOutput {
    pub(crate) answer: Option<Box<dyn HistoryCell>>,
    pub(crate) plan: Option<Box<dyn HistoryCell>>,
    /// A stream was there.
    pub(crate) has_controller: bool,
    /// Their queues are empty after the tick.
    pub(crate) all_idle: bool,
}

/// One tick: the queues' snapshot, the policy's plan, the rows committed.
pub(crate) fn run_commit_tick(
    policy: &mut AdaptiveChunkingPolicy,
    stream_controller: Option<&mut StreamController>,
    plan_stream_controller: Option<&mut PlanStreamController>,
    scope: CommitTickScope,
    now: Instant,
) -> CommitTickOutput {
    let snapshot = stream_queue_snapshot(
        stream_controller.as_deref(),
        plan_stream_controller.as_deref(),
        now,
    );
    let decision = policy.decide(snapshot, now);
    if scope == CommitTickScope::CatchUpOnly && decision.mode != ChunkingMode::CatchUp {
        return CommitTickOutput {
            all_idle: true,
            ..CommitTickOutput::default()
        };
    }
    let mut output = CommitTickOutput {
        all_idle: true,
        ..CommitTickOutput::default()
    };
    if let Some(controller) = stream_controller {
        output.has_controller = true;
        let (cell, idle) = match decision.drain_plan {
            DrainPlan::Single => controller.on_commit_tick(),
            DrainPlan::Batch(max_lines) => controller.on_commit_tick_batch(max_lines),
        };
        output.answer = cell;
        output.all_idle &= idle;
    }
    if let Some(controller) = plan_stream_controller {
        output.has_controller = true;
        let (cell, idle) = match decision.drain_plan {
            DrainPlan::Single => controller.on_commit_tick(),
            DrainPlan::Batch(max_lines) => controller.on_commit_tick_batch(max_lines),
        };
        output.plan = cell;
        output.all_idle &= idle;
    }
    output
}

/// The queues' depth together, and the age of the oldest row in either.
fn stream_queue_snapshot(
    stream_controller: Option<&StreamController>,
    plan_stream_controller: Option<&PlanStreamController>,
    now: Instant,
) -> QueueSnapshot {
    let mut queued_lines = 0usize;
    let mut oldest_age: Option<Duration> = None;
    if let Some(controller) = stream_controller {
        queued_lines += controller.queued_lines();
        oldest_age = oldest_age.max(controller.oldest_queued_age(now));
    }
    if let Some(controller) = plan_stream_controller {
        queued_lines += controller.queued_lines();
        oldest_age = oldest_age.max(controller.oldest_queued_age(now));
    }
    QueueSnapshot {
        queued_lines,
        oldest_age,
    }
}

#[cfg(test)]
mod tests;
