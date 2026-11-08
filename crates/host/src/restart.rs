//! When something that faulted starts again on its own: a backoff per
//! restart and a window the restarts are counted in. The Agent's slot
//! restarts the Agent after a kernel fault with it; the application host
//! remounts a module after a fault with it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// The delays before the automatic restarts in one window, in order; their
/// count is how many the window allows.
pub const AUTOMATIC_RESTART_DELAYS: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(10),
    Duration::from_secs(30),
];
/// The window the automatic restarts are counted in.
pub const AUTOMATIC_RESTART_WINDOW: Duration = Duration::from_secs(10 * 60);

/// After a fault, the `n`-th automatic restart within `window` follows
/// after `delays[n]`; once they are used up, what faulted stays stopped
/// until the user restarts it, which also empties the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestartPolicy {
    pub delays: Vec<Duration>,
    pub window: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            delays: AUTOMATIC_RESTART_DELAYS.to_vec(),
            window: AUTOMATIC_RESTART_WINDOW,
        }
    }
}

/// The automatic restarts one supervised thing made in the current window.
#[derive(Debug, Default)]
pub struct RestartWindow {
    restarts: VecDeque<Instant>,
}

impl RestartWindow {
    /// The automatic restarts made within `policy`'s window before `now`,
    /// and the delay before the next one, if the window allows one.
    pub fn next(&mut self, policy: &RestartPolicy, now: Instant) -> (usize, Option<Duration>) {
        while self
            .restarts
            .front()
            .is_some_and(|restart| now.duration_since(*restart) >= policy.window)
        {
            self.restarts.pop_front();
        }
        let made = self.restarts.len();
        (made, policy.delays.get(made).copied())
    }

    /// Counts an automatic restart made at `at`.
    pub fn record(&mut self, at: Instant) {
        self.restarts.push_back(at);
    }

    /// The user restarted it: the window starts empty.
    pub fn clear(&mut self) {
        self.restarts.clear();
    }

    /// The automatic restarts made within `policy`'s window before `now`.
    pub fn count(&self, policy: &RestartPolicy, now: Instant) -> usize {
        self.restarts
            .iter()
            .filter(|restart| now.duration_since(**restart) < policy.window)
            .count()
    }
}
