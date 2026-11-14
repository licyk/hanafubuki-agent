//! When a root's wakes pause (D16): a person's interrupt of one of its
//! turns, or more than [`MAXIMUM_WOKEN_TURNS`] woken turns in its tree since
//! the person's last message. Events still queue; the person's next message
//! delivers them and clears the pause. Kept in memory: a new Agent instance
//! starts with no pause.

use std::collections::HashMap;
use std::sync::Arc;

use hanafubuki_agent_kernel::AgentSessionId;
use parking_lot::Mutex;
use serde::Serialize;

/// The most woken turns a tree runs between two of the person's messages.
pub const MAXIMUM_WOKEN_TURNS: usize = 100;

/// Why a root's wakes are paused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentWakePause {
    /// The person interrupted one of its turns.
    PersonInterrupted,
    /// Its tree ran [`MAXIMUM_WOKEN_TURNS`] woken turns since the person's
    /// last message.
    RunawayBound,
}

/// Whether a session is woken for its background work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum AgentWakeState {
    On,
    /// Its events queue and wait for the person's next message.
    Paused {
        reason: AgentWakePause,
    },
    /// `wakeOnBackgroundTasks` is off, or the session is never woken (a
    /// side conversation, a review).
    Off,
}

/// The roots' pauses.
#[derive(Clone)]
pub(crate) struct Pauses(Arc<Mutex<PauseState>>);

struct PauseState {
    roots: HashMap<AgentSessionId, RootWakes>,
    bound: usize,
}

#[derive(Default)]
struct RootWakes {
    interrupted: bool,
    woken: usize,
}

impl Default for Pauses {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(PauseState {
            roots: HashMap::new(),
            bound: MAXIMUM_WOKEN_TURNS,
        })))
    }
}

impl Pauses {
    /// The person interrupted a turn of `root`.
    pub(crate) fn interrupted(&self, root: AgentSessionId) {
        self.0.lock().roots.entry(root).or_default().interrupted = true;
    }

    /// The person sent `root` a message: its wakes resume.
    pub(crate) fn person_message(&self, root: AgentSessionId) {
        self.0.lock().roots.remove(&root);
    }

    /// A woken turn of `root`'s tree was admitted.
    pub(crate) fn woken(&self, root: AgentSessionId) {
        self.0.lock().roots.entry(root).or_default().woken += 1;
    }

    /// Why `root`'s wakes are paused, if they are.
    pub(crate) fn pause(&self, root: AgentSessionId) -> Option<AgentWakePause> {
        let state = self.0.lock();
        let wakes = state.roots.get(&root)?;
        if wakes.interrupted {
            Some(AgentWakePause::PersonInterrupted)
        } else if wakes.woken >= state.bound {
            Some(AgentWakePause::RunawayBound)
        } else {
            None
        }
    }

    /// A lower runaway bound, so a test need not run 100 woken turns.
    #[cfg(test)]
    pub(crate) fn set_bound(&self, bound: usize) {
        self.0.lock().bound = bound;
    }
}
