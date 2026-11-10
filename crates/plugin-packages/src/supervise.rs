//! Following the guests: each guest fiber's transitions set its state, its
//! process's start and exit keep its handle and exit report, and a guest
//! that fails after it ran restarts under the restart policy, until the
//! policy's window is used up.
//!
//! A guest's own words (its stderr, its errors, why it was killed) are
//! logged only at `trace`, bounded; the rest is the Agent's.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_contracts::CancellationToken;
use hanafubuki_agent_foundation::trace_content::bounded_trace_content;
use kardia_core::{Fiber, FiberState};
use kardia_hmr::{ExitCause, GuestExit, GuestHandle};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::guests::{restart_fiber, Shared};
use crate::state::GuestState;

/// A restart the policy allows, after its delay.
pub(crate) struct Restart {
    id: String,
    fiber: Fiber,
    epoch: u64,
    delay: Duration,
}

impl Shared {
    /// The package of a guest fiber of this host's loader.
    fn guest_of(&self, fiber: &Fiber) -> Option<String> {
        let mount = self.mount()?;
        let entry = mount.loader.entry_of(fiber.context())?;
        (entry.inner_fiber().as_ref() == Some(fiber)).then(|| entry.id())
    }

    pub(crate) fn on_status(&self, fiber: &Fiber, state: FiberState) {
        if !matches!(
            state,
            FiberState::Loading | FiberState::Active | FiberState::Failed | FiberState::Disposed
        ) {
            return;
        }
        let Some(id) = self.guest_of(fiber) else {
            return;
        };
        let restart = {
            let mut records = self.records.lock();
            let Some(record) = records.get_mut(&id) else {
                return;
            };
            match state {
                FiberState::Loading => {
                    record.state = GuestState::Starting;
                    record.started = false;
                    None
                }
                FiberState::Active => {
                    record.state = GuestState::Running;
                    record.error = None;
                    record.started = true;
                    log::info!("plugin {id}: its guest runs");
                    None
                }
                // A new fiber's first load is announced before its entry
                // holds it, so the next start's begins here.
                FiberState::Disposed => {
                    if record.state != GuestState::Blocked {
                        record.state = GuestState::Stopped;
                    }
                    record.started = false;
                    None
                }
                _ => {
                    let error = fiber
                        .error()
                        .map(|error| error.to_string())
                        .unwrap_or_else(|| "it failed".to_string());
                    log::trace!(
                        "plugin {id}: its guest failed: {}",
                        bounded_trace_content(&error)
                    );
                    record.error = Some(error);
                    if let Some(exit) = record.handle.as_ref().and_then(GuestHandle::exit) {
                        record.last_exit = Some(exit);
                    }
                    if !record.started {
                        record.state = GuestState::Failed;
                        log::warn!("plugin {id}: its guest did not start");
                        None
                    } else {
                        let (made, delay) = record.window.next(&self.policy, Instant::now());
                        match delay {
                            Some(delay) => {
                                record.state = GuestState::Restarting;
                                record.epoch += 1;
                                log::warn!(
                                    "plugin {id}: its guest failed; it restarts in {} s",
                                    delay.as_secs()
                                );
                                Some(Restart {
                                    id: id.clone(),
                                    fiber: fiber.clone(),
                                    epoch: record.epoch,
                                    delay,
                                })
                            }
                            None => {
                                record.state = GuestState::Failed;
                                log::warn!(
                                    "plugin {id}: its guest failed again after {made} restarts; \
                                     it stays stopped until it is retried"
                                );
                                None
                            }
                        }
                    }
                }
            }
        };
        if let Some(restart) = restart {
            if let Some(mount) = self.mount() {
                let _ = mount.restarts.send(restart);
            }
        }
        self.changed();
    }

    pub(crate) fn on_started(&self, fiber: &Fiber, handle: &GuestHandle) {
        let Some(id) = self.guest_of(fiber) else {
            return;
        };
        if let Some(record) = self.records.lock().get_mut(&id) {
            record.handle = Some(handle.clone());
        }
        self.changed();
    }

    pub(crate) fn on_exited(&self, fiber: &Fiber, exit: &Arc<GuestExit>) {
        let Some(id) = self.guest_of(fiber) else {
            return;
        };
        let (cause, reason) = match &exit.cause {
            ExitCause::Killed(reason) => ("was killed", Some(reason.as_str())),
            _ => ("exited", None),
        };
        match exit.status {
            Some(status) => log::warn!("plugin {id}: its guest process {cause} ({status})"),
            None => log::warn!("plugin {id}: its guest process {cause}"),
        }
        if let Some(reason) = reason {
            log::trace!(
                "plugin {id}: its guest process was killed: {}",
                bounded_trace_content(reason)
            );
        }
        log::trace!(
            "plugin {id}: its guest's stderr: {}",
            bounded_trace_content(&String::from_utf8_lossy(&exit.stderr_tail))
        );
        if let Some(record) = self.records.lock().get_mut(&id) {
            record.last_exit = Some(exit.clone());
        }
        self.changed();
    }

    /// Restarts the guest `restart` names, unless something was decided
    /// about it since (a new package, a retry, a stop) or it is no longer
    /// the failed fiber of its entry.
    async fn restart(&self, restart: Restart) {
        if self.current_fiber(&restart.id).as_ref() != Some(&restart.fiber)
            || restart.fiber.state() != FiberState::Failed
        {
            return;
        }
        {
            let mut records = self.records.lock();
            let Some(record) = records.get_mut(&restart.id) else {
                return;
            };
            if record.epoch != restart.epoch || record.state != GuestState::Restarting {
                return;
            }
            record.window.record(Instant::now());
        }
        log::info!("plugin {}: its guest restarts", restart.id);
        restart_fiber(&restart.fiber).await;
    }
}

/// The guest host's worker: it applies the packages once the loader runs,
/// then runs each restart after its delay, until the guest host stops
/// (its plugin drops the sender).
pub(crate) async fn run(
    shared: Arc<Shared>,
    mut restarts: mpsc::UnboundedReceiver<Restart>,
    _cancellation: CancellationToken,
) {
    shared.reconcile().await;
    let mut pending = JoinSet::new();
    loop {
        tokio::select! {
            restart = restarts.recv() => match restart {
                Some(restart) => {
                    let shared = shared.clone();
                    pending.spawn(async move {
                        tokio::time::sleep(restart.delay).await;
                        shared.restart(restart).await;
                    });
                }
                None => break,
            },
            Some(_) = pending.join_next(), if !pending.is_empty() => {}
        }
    }
}
