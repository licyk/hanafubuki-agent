// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! How many queued rows a commit tick drains. Smooth: one row a tick.
//! When the queue grows deep or old, catch-up drains the whole backlog at
//! once, so the display never lags far behind the model.
//!
//! The policy sees only the queue's depth and its oldest row's age. It
//! switches with hysteresis: catch-up starts past the enter thresholds and
//! ends only after the queue stayed below the exit thresholds for
//! [`EXIT_HOLD`]; after it ends, it does not start again for
//! [`REENTER_CATCH_UP_HOLD`] unless the backlog is severe.

use std::time::Duration;
use std::time::Instant;

/// Queue-depth threshold that allows entering catch-up mode.
///
/// Crossing this threshold alone is sufficient to leave smooth mode.
const ENTER_QUEUE_DEPTH_LINES: usize = 8;

/// Oldest-line age threshold that allows entering catch-up mode.
///
/// Crossing this threshold alone is sufficient to leave smooth mode.
const ENTER_OLDEST_AGE: Duration = Duration::from_millis(120);

/// Queue-depth threshold used when evaluating catch-up exit hysteresis.
///
/// Depth must be at or below this value before exit hold timing can begin.
const EXIT_QUEUE_DEPTH_LINES: usize = 2;

/// Oldest-line age threshold used when evaluating catch-up exit hysteresis.
///
/// Age must be at or below this value before exit hold timing can begin.
const EXIT_OLDEST_AGE: Duration = Duration::from_millis(40);

/// Minimum duration queue pressure must stay below exit thresholds to leave catch-up mode.
const EXIT_HOLD: Duration = Duration::from_millis(250);

/// Cooldown window after a catch-up exit that suppresses immediate re-entry.
///
/// Severe backlog still bypasses this hold to avoid unbounded queue-age growth.
const REENTER_CATCH_UP_HOLD: Duration = Duration::from_millis(250);

/// Queue-depth cutoff that marks backlog as severe for faster convergence.
///
/// This threshold is used to bypass re-entry hold after a recent catch-up exit.
const SEVERE_QUEUE_DEPTH_LINES: usize = 64;

/// Oldest-line age cutoff that marks backlog as severe for faster convergence.
const SEVERE_OLDEST_AGE: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ChunkingMode {
    /// Drain one line per baseline commit tick.
    #[default]
    Smooth,
    /// Drain multiple lines per tick according to queue pressure.
    CatchUp,
}

/// Captures queue pressure inputs used by adaptive chunking decisions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct QueueSnapshot {
    /// Number of queued stream lines waiting to be displayed.
    pub(crate) queued_lines: usize,
    /// Age of the oldest queued line at decision time.
    pub(crate) oldest_age: Option<Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrainPlan {
    /// Emit exactly one queued line.
    Single,
    /// Emit up to `usize` queued lines.
    Batch(usize),
}

/// Represents one policy decision for a specific queue snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChunkingDecision {
    /// Mode after applying hysteresis transitions for this decision.
    pub(crate) mode: ChunkingMode,
    /// Drain plan to execute for the current commit tick.
    pub(crate) drain_plan: DrainPlan,
}

/// Maintains adaptive chunking mode and hysteresis state across ticks.
#[derive(Debug, Default)]
pub(crate) struct AdaptiveChunkingPolicy {
    mode: ChunkingMode,
    below_exit_threshold_since: Option<Instant>,
    last_catch_up_exit_at: Option<Instant>,
}

impl AdaptiveChunkingPolicy {
    /// Returns the policy mode used by the most recent decision.
    #[cfg(test)]
    pub(crate) fn mode(&self) -> ChunkingMode {
        self.mode
    }

    /// Resets state to baseline smooth mode.
    pub(crate) fn reset(&mut self) {
        self.mode = ChunkingMode::Smooth;
        self.below_exit_threshold_since = None;
        self.last_catch_up_exit_at = None;
    }

    /// Computes a drain decision from the current queue snapshot.
    ///
    /// The decision is deterministic for a given `(mode, snapshot, now)` triple. Callers should
    /// avoid inventing synthetic snapshots; stale queue age data can cause premature catch-up exits.
    pub(crate) fn decide(&mut self, snapshot: QueueSnapshot, now: Instant) -> ChunkingDecision {
        if snapshot.queued_lines == 0 {
            self.note_catch_up_exit(now);
            self.mode = ChunkingMode::Smooth;
            self.below_exit_threshold_since = None;
            return ChunkingDecision {
                mode: self.mode,
                drain_plan: DrainPlan::Single,
            };
        }

        match self.mode {
            ChunkingMode::Smooth => self.maybe_enter_catch_up(snapshot, now),
            ChunkingMode::CatchUp => self.maybe_exit_catch_up(snapshot, now),
        }

        let drain_plan = match self.mode {
            ChunkingMode::Smooth => DrainPlan::Single,
            ChunkingMode::CatchUp => DrainPlan::Batch(snapshot.queued_lines.max(1)),
        };

        ChunkingDecision {
            mode: self.mode,
            drain_plan,
        }
    }

    /// Switches from `Smooth` to `CatchUp` when enter thresholds are crossed.
    fn maybe_enter_catch_up(&mut self, snapshot: QueueSnapshot, now: Instant) {
        if !should_enter_catch_up(snapshot) {
            return;
        }
        if self.reentry_hold_active(now) && !is_severe_backlog(snapshot) {
            return;
        }
        self.mode = ChunkingMode::CatchUp;
        self.below_exit_threshold_since = None;
        self.last_catch_up_exit_at = None;
    }

    /// Applies exit hysteresis while in `CatchUp` mode.
    ///
    /// The policy requires queue pressure to stay below exit thresholds for the
    /// full `EXIT_HOLD` window before returning to `Smooth`.
    fn maybe_exit_catch_up(&mut self, snapshot: QueueSnapshot, now: Instant) {
        if !should_exit_catch_up(snapshot) {
            self.below_exit_threshold_since = None;
            return;
        }

        match self.below_exit_threshold_since {
            Some(since) if now.saturating_duration_since(since) >= EXIT_HOLD => {
                self.mode = ChunkingMode::Smooth;
                self.below_exit_threshold_since = None;
                self.last_catch_up_exit_at = Some(now);
            }
            Some(_) => {}
            None => {
                self.below_exit_threshold_since = Some(now);
            }
        }
    }

    fn note_catch_up_exit(&mut self, now: Instant) {
        if self.mode == ChunkingMode::CatchUp {
            self.last_catch_up_exit_at = Some(now);
        }
    }

    fn reentry_hold_active(&self, now: Instant) -> bool {
        self.last_catch_up_exit_at
            .is_some_and(|exit| now.saturating_duration_since(exit) < REENTER_CATCH_UP_HOLD)
    }
}

/// Returns whether current queue pressure warrants entering catch-up mode.
///
/// Either depth or age pressure is sufficient to trigger catch-up.
fn should_enter_catch_up(snapshot: QueueSnapshot) -> bool {
    snapshot.queued_lines >= ENTER_QUEUE_DEPTH_LINES
        || snapshot
            .oldest_age
            .is_some_and(|oldest| oldest >= ENTER_OLDEST_AGE)
}

/// Returns whether queue pressure is low enough to begin exit hysteresis.
///
/// Both depth and age must be below thresholds; this prevents oscillation when
/// one signal is still under load.
fn should_exit_catch_up(snapshot: QueueSnapshot) -> bool {
    snapshot.queued_lines <= EXIT_QUEUE_DEPTH_LINES
        && snapshot
            .oldest_age
            .is_some_and(|oldest| oldest <= EXIT_OLDEST_AGE)
}

/// Returns whether backlog is severe enough to use a faster catch-up target.
///
/// Severe pressure bypasses re-entry hold to avoid queue-age growth after a
/// recent catch-up exit.
fn is_severe_backlog(snapshot: QueueSnapshot) -> bool {
    snapshot.queued_lines >= SEVERE_QUEUE_DEPTH_LINES
        || snapshot
            .oldest_age
            .is_some_and(|oldest| oldest >= SEVERE_OLDEST_AGE)
}

#[cfg(test)]
mod tests;
