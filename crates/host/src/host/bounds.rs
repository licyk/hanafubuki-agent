//! Bounds on the host's waits for kardia.
//!
//! kardia never cancels a plugin's `apply`, and disposing a fiber whose
//! `apply` has not returned waits for it, so a plugin that hangs would hold
//! whatever waits on it. The host bounds both waits instead: a start (the
//! first mount, a configuration update, a retry, a remount) that has not
//! settled within [`PLUGIN_START_TIMEOUT`] fails its row and disposes its
//! fiber, and a disposal still running after [`PLUGIN_STOP_BOUND`] reports
//! its row stuck. Neither wait blocks the host: start-up moves on to the
//! next row, and the teardown keeps running on its own (a fiber stuck for
//! good is dropped with the runtime the host runs on).

use std::sync::Arc;
use std::time::Duration;

use kardia_core::{Disposal, Fiber, Settle};
use tokio::sync::watch;

use super::row::Mount;
use super::{HostProfile, ModuleHost};
use crate::status::ModuleGroup;
use crate::PluginError;

/// How long a row's start may take to settle before the row is failed.
/// Every plugin's `apply` only validates, registers and spawns
/// (`site/content/docs/plugin-rules.en.mdx`), so none needs a bound of its
/// own.
pub const PLUGIN_START_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a disposal may run before its row is reported stuck: more than
/// a plugin's own teardown takes (`DRAIN_GRACE`, `RETIRE_CANCEL_BOUND`,
/// `WORKER_STOP_BOUND`).
pub const PLUGIN_STOP_BOUND: Duration = Duration::from_secs(10);

/// Where a disposed fiber's teardown is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Teardown {
    Stopping,
    /// Still running after [`PLUGIN_STOP_BOUND`]; it may yet end.
    Stuck,
    /// Ended: its registrations are gone.
    Stopped,
}

pub(super) fn start_timed_out() -> String {
    format!("did not start within {} s", PLUGIN_START_TIMEOUT.as_secs())
}

pub(super) fn stop_stuck() -> String {
    format!(
        "its teardown did not end within {} s",
        PLUGIN_STOP_BOUND.as_secs()
    )
}

/// Waits for `disposal` in a task of its own and reports how far it got:
/// `Stuck` once [`PLUGIN_STOP_BOUND`] passes, `Stopped` when it ends.
/// `profile` hears of both.
pub(super) fn watch_disposal<K: HostProfile>(
    name: String,
    disposal: Disposal,
    profile: Arc<K>,
) -> watch::Receiver<Teardown> {
    let (teardown, watched) = watch::channel(Teardown::Stopping);
    tokio::spawn(async move {
        let mut disposal = disposal;
        let outcome = match tokio::time::timeout(PLUGIN_STOP_BOUND, &mut disposal).await {
            Ok(outcome) => outcome,
            Err(_) => {
                log::error!(
                    "{} {name} {}; it is reported stuck while it keeps stopping",
                    K::NOUN,
                    stop_stuck()
                );
                teardown.send_replace(Teardown::Stuck);
                profile.modules_changed();
                disposal.await
            }
        };
        if let Err(error) = outcome {
            log::warn!(
                "{} {name} did not stop cleanly: {}",
                K::NOUN,
                PluginError::from(error)
            );
        }
        teardown.send_replace(Teardown::Stopped);
        profile.modules_changed();
    });
    watched
}

impl<K: HostProfile> ModuleHost<K> {
    /// `settle` of row `index`'s `fiber`, bounded by
    /// [`PLUGIN_START_TIMEOUT`]: past it, the row fails and its fiber is
    /// disposed ([`Self::time_out`]).
    pub(super) fn bounded(self: &Arc<Self>, index: usize, fiber: Fiber, settle: Settle) -> Settle {
        let host = self.clone();
        Box::pin(async move {
            match tokio::time::timeout(PLUGIN_START_TIMEOUT, settle).await {
                Ok(outcome) => outcome,
                Err(_) => {
                    host.time_out(index, &fiber);
                    Err(kardia_core::Error::msg(start_timed_out()))
                }
            }
        })
    }

    /// Fails row `index` whose `fiber` did not settle in time and disposes
    /// it, if that fiber is still the row's. The host runs without it until
    /// a retry or a new configuration, once its teardown ends. A core row is
    /// only logged.
    fn time_out(&self, index: usize, fiber: &Fiber) {
        let retired = {
            let mut state = self.state();
            let row = &mut state.rows[index];
            match &row.mount {
                // A core row stays mounted, as with a fault: the other rows
                // do not run without it.
                Mount::Mounted(current) if current == fiber && row.group.is_core() => {
                    log::error!(
                        "core {} {} {}; it stays mounted",
                        K::NOUN,
                        row.name,
                        start_timed_out()
                    );
                    false
                }
                Mount::Mounted(current) if current == fiber => {
                    log::error!(
                        "{} {} {}; it is stopped and {} runs without it",
                        K::NOUN,
                        row.name,
                        start_timed_out(),
                        K::HOST
                    );
                    row.retire(self.profile.clone(), Some(start_timed_out()))
                }
                _ => false,
            }
        };
        if retired {
            self.profile.modules_changed();
        }
    }
}
