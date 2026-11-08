//! Restarting a row that faulted, on a host with a restart policy
//! (`ModuleHostOptions::automatic_restarts`).
//!
//! A fault stops the row as `Failed` (`faults.rs`); the host then schedules
//! its remount after the policy's next delay, while the row's window
//! allows one. The remount is the one a retry makes: it waits for the old
//! instance's teardown and is bounded by the start timeout. A start that
//! fails is not restarted again by itself (a constructor that fails will
//! fail again); it stays `Failed` until a retry. Turning the row off, a
//! retry, a new configuration and shutdown cancel a pending restart; a
//! retry also empties the window.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::task::JoinHandle;

use super::row::HostedRow;
use super::{HostProfile, HostRoot, ModuleHost};
use crate::PluginError;

/// One scheduled automatic restart.
pub(super) struct PendingRestart {
    id: u64,
    at_unix_millis: u64,
    task: JoinHandle<Option<()>>,
}

impl PendingRestart {
    pub(super) fn at_unix_millis(&self) -> u64 {
        self.at_unix_millis
    }

    /// The task may already be past its wait; it then finds itself no
    /// longer pending and does nothing.
    pub(super) fn cancel(self) {
        self.task.abort();
    }
}

impl<K: HostProfile> ModuleHost<K> {
    /// Schedules the automatic restart of row `index`, which a fault just
    /// stopped, under the row lock. Returns its delay; `None` without a
    /// policy or once the window's restarts are used up.
    pub(super) fn schedule_restart(
        self: &Arc<Self>,
        index: usize,
        row: &mut HostedRow<K>,
    ) -> Option<Duration> {
        let policy = self.restart_policy.as_ref()?;
        let (made, delay) = row.restarts.next(policy, Instant::now());
        let Some(delay) = delay else {
            log::error!(
                "{} {} used up its {made} automatic restarts in the window",
                K::NOUN,
                row.name
            );
            return None;
        };
        let id = self.next_restart.fetch_add(1, Ordering::Relaxed);
        let host = Arc::downgrade(self);
        let task = self.spawn_task(async move {
            tokio::time::sleep(delay).await;
            if let Some(host) = host.upgrade() {
                host.restart_automatically(index, id).await;
            }
        });
        row.cancel_restart();
        row.pending_restart = Some(PendingRestart {
            id,
            at_unix_millis: unix_millis(SystemTime::now() + delay),
            task,
        });
        Some(delay)
    }

    /// Mounts row `index` again if restart `id` is still its pending one
    /// and it is still enabled and stopped by its fault.
    async fn restart_automatically(self: &Arc<Self>, index: usize, id: u64) {
        let (name, settle) = {
            let mut state = self.state();
            let HostRoot::Running(root) = &state.root else {
                return;
            };
            let root = root.clone();
            let row = &mut state.rows[index];
            if row.pending_restart.as_ref().map(|pending| pending.id) != Some(id) {
                return;
            }
            // This task is the pending restart: dropping its handle detaches
            // it, which is what it is doing anyway.
            row.pending_restart = None;
            if !row.enabled || !row.is_faulted() {
                return;
            }
            row.restarts.record(Instant::now());
            log::warn!("{} {} starts again after its fault", K::NOUN, row.name);
            (row.name.clone(), self.enable(&root, index, row))
        };
        self.profile.modules_changed();
        let Some(settle) = settle else {
            return;
        };
        match self.notifying(settle).await {
            Ok(()) => log::info!("{} {name} started again after its fault", K::NOUN),
            Err(error) => log::error!(
                "{} {name} did not start again after its fault and stays failed until it \
                 is retried: {}",
                K::NOUN,
                PluginError::from(error)
            ),
        }
    }
}

fn unix_millis(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}
