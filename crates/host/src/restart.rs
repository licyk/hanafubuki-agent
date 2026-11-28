//! When something that faulted starts again on its own: a backoff per
//! restart and a window the restarts are counted in. The Agent's slot
//! restarts the Agent after a kernel fault with it; the application host
//! remounts a module after a fault with it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use kardia_core::RecoveryPolicy;

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

impl RestartPolicy {
    /// The kardia policy these delays and window describe: each restart
    /// waits the next entry of the table, and a restart leaves the window's
    /// count once it is that old.
    pub fn recovery(&self) -> RecoveryPolicy {
        let delays = self.delays.clone();
        let window = self.window;
        let made = std::sync::Arc::new(parking_lot::Mutex::new(RestartWindow::default()));
        // `kardia` asks a host policy for its decision; the window is this
        // policy's, since a host policy carries none.
        RecoveryPolicy::host(move |failure: kardia_core::Failure| {
            let delays = delays.clone();
            let made = made.clone();
            let policy = RestartPolicy {
                delays: delays.clone(),
                window,
            };
            async move {
                // A constructor that fails fails the same way on every
                // retry, so it is not restarted by itself.
                if failure.phase == kardia_core::Phase::Start {
                    return Ok(kardia_core::RecoveryAction::KeepFailed);
                }
                let mut made = made.lock();
                let (_, delay) = made.next(&policy, Instant::now());
                match delay {
                    Some(delay) => {
                        made.record(Instant::now());
                        Ok(kardia_core::RecoveryAction::RetryAfter(delay))
                    }
                    None => Ok(kardia_core::RecoveryAction::KeepFailed),
                }
            }
        })
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
