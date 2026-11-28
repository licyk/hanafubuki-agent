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
use hanafubuki_agent_host::GUEST_LOGGER_PREFIX;
use kardia_core::{Fiber, FiberState};
use kardia_hmr::{ExitCause, GuestExit, GuestHandle};
use kardia_loader::Loader;
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
        guest_of(&mount.loader, fiber)
    }

    pub(crate) fn on_status(self: &Arc<Self>, fiber: &Fiber, state: FiberState) {
        if !matches!(
            state,
            FiberState::Loading | FiberState::Active | FiberState::Failed | FiberState::Disposed
        ) {
            return;
        }
        let Some(id) = self.guest_of(fiber) else {
            return;
        };
        // `attempts` counts the restart being waited out; the ones made are
        // the ones before it.
        let recovery = fiber.recovery_state();
        let made = recovery
            .attempts
            .saturating_sub(u32::from(recovery.due_in.is_some()));
        if let Some(record) = self.records.lock().get_mut(&id) {
            record.restarts_made = usize::try_from(made).unwrap_or(usize::MAX);
        }
        // Registered before it is said to run, so a guest that runs has its
        // contributions.
        if state == FiberState::Active {
            self.contribute(&id, fiber);
            // Its first run installs kardia's policy: from then on the
            // fiber restarts itself after a fault, and disposal, a retry or
            // the policy's end cancels a pending restart.
            let install = {
                let mut records = self.records.lock();
                match records.get_mut(&id) {
                    Some(record) if !record.recovering => {
                        record.recovering = true;
                        Some(self.policy.recovery())
                    }
                    _ => None,
                }
            };
            if let Some(policy) = install {
                if let Err(error) = fiber.set_recovery(policy) {
                    log::warn!("plugin {id}: its guest takes no automatic restart: {error}");
                }
            }
        }
        let restart = {
            let mut records = self.records.lock();
            let Some(record) = records.get_mut(&id) else {
                return;
            };
            // Its contributions retire with every end of a start, and its
            // next start registers them again.
            if state != FiberState::Active {
                record.prompt = None;
            }
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
                    } else if record.recovering {
                        // kardia schedules the restart when the policy
                        // allows one; once it does not, the guest stays
                        // failed until it is retried.
                        // kardia schedules the restart from its own
                        // supervisor, after this status is reported, so the
                        // window decides whether one is still to come.
                        let pending = record.restarts_made < self.policy.delays.len();
                        record.state = if pending {
                            GuestState::Restarting
                        } else {
                            GuestState::Failed
                        };
                        log::warn!(
                            "plugin {id}: its guest failed; {}",
                            if pending {
                                "it restarts by itself"
                            } else {
                                "it stays stopped until it is retried"
                            }
                        );
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

fn guest_of(loader: &Loader, fiber: &Fiber) -> Option<String> {
    let entry = loader.entry_of(fiber.context())?;
    let id = entry.id();
    match entry.inner_fiber() {
        Some(current) if current == *fiber => return Some(id),
        Some(current) if !current.is_removed() => return None,
        _ => {}
    }
    // Loading is synchronous, and even Active can arrive before the loader
    // stores inner_fiber. The definition's own name also excludes loader
    // wrappers and guest children, whose Fiber::name() can inherit this name.
    let name = format!("{GUEST_LOGGER_PREFIX}{id}");
    (fiber.def()?.plugin_name() == Some(name.as_str())).then_some(id)
}

#[cfg(test)]
mod tests;

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
