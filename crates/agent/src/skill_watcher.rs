//! The Skill catalog's content-blind change watcher, which the Agent starts
//! with each running instance and stops at its shutdown.
//!
//! The OS reports changes under the watched roots (`notify`: inotify,
//! FSEvents, ReadDirectoryChangesW); a burst of them, such as a checkout or an
//! editor's save, becomes one invalidation per root once the roots have been
//! quiet for [`DEBOUNCE`]. Only roots that exist when the watch starts are
//! watched. A root that appears later, or a repository root the workspace
//! gains, is still read correctly: every catalog read re-resolves the
//! repository roots and re-stamps every root, and the watcher only makes a
//! change visible without waiting for that stamp to differ.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::background_worker::{
    BackgroundWorkerGroup, WorkerShutdownDiagnostic,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_foundation::paths::PathKey;
use notify::{EventKind, RecursiveMode, Watcher};
use parking_lot::Mutex;

use crate::SkillCatalogInvalidationPort;

const DEBOUNCE: Duration = Duration::from_millis(250);

/// Watches the Skill roots of the running Agent. It exists with the Agent
/// (`DesktopAgent`), idle, and starts once an instance runs, since the roots
/// are that instance's; each instance that runs replaces the last one's
/// watch.
#[derive(Clone)]
pub(crate) struct SkillCatalogWatcher {
    workers: BackgroundWorkerGroup,
    /// Stops the current watch.
    current: Arc<Mutex<Option<WatchHandle>>>,
}

struct WatchHandle {
    cancellation: CancellationToken,
    signals: Sender<Signal>,
}

impl WatchHandle {
    fn stop(self) {
        self.cancellation.cancel();
        // Wakes the worker, which otherwise waits for the next change.
        let _ = self.signals.send(Signal::Stop);
    }
}

enum Signal {
    Changed(notify::Result<notify::Event>),
    Stop,
}

impl SkillCatalogWatcher {
    pub(crate) fn idle() -> Self {
        // Contained: the watcher calls into the Agent's Skill catalog.
        Self {
            workers: BackgroundWorkerGroup::contained(),
            current: Arc::default(),
        }
    }

    /// Watches `roots` in place of the current watch, telling `catalog` (the
    /// Agent slot, resolved per change) what changed.
    pub(crate) fn restart(
        &self,
        roots: &[PathBuf],
        catalog: Arc<dyn SkillCatalogInvalidationPort>,
    ) {
        let cancellation = CancellationToken::default();
        let (signals, receiver) = mpsc::channel();
        let previous = self.current.lock().replace(WatchHandle {
            cancellation: cancellation.clone(),
            signals: signals.clone(),
        });
        if let Some(previous) = previous {
            previous.stop();
        }
        let roots = roots
            .iter()
            .filter(|root| root.is_dir())
            .cloned()
            .collect::<Vec<_>>();
        let worker_catalog = catalog.clone();
        let result = self.workers.spawn_with_cancellation(
            "agent-skill-catalog-watcher",
            cancellation,
            move |cancellation| {
                // Dropped with this worker, which ends the OS watch.
                let _watcher = match watch(&roots, signals) {
                    Ok(watcher) => watcher,
                    Err(error) => {
                        log::warn!("Skill watcher initialization failed: {error}");
                        worker_catalog.set_watcher_diagnostic(Some(
                            "Skill change watcher is unavailable; use force refresh".to_string(),
                        ));
                        return;
                    }
                };
                worker_catalog.set_watcher_diagnostic(None);
                invalidate_on_change(&roots, &receiver, &cancellation, &*worker_catalog);
            },
        );
        if let Err(error) = result {
            log::warn!("failed to start Skill catalog watcher: {error}");
            catalog.set_watcher_diagnostic(Some(
                "Skill change watcher could not start; use force refresh".to_string(),
            ));
        }
    }

    pub(crate) fn shutdown(&self, deadline: Instant) -> Vec<WorkerShutdownDiagnostic> {
        if let Some(current) = self.current.lock().take() {
            current.stop();
        }
        self.workers.shutdown(deadline)
    }
}

fn watch(roots: &[PathBuf], signals: Sender<Signal>) -> notify::Result<impl Watcher> {
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = signals.send(Signal::Changed(event));
    })?;
    for root in roots {
        watcher.watch(root, RecursiveMode::Recursive)?;
    }
    Ok(watcher)
}

/// Collects the roots each change touched and invalidates them once the
/// changes have stopped for [`DEBOUNCE`].
fn invalidate_on_change(
    roots: &[PathBuf],
    receiver: &Receiver<Signal>,
    cancellation: &CancellationToken,
    catalog: &dyn SkillCatalogInvalidationPort,
) {
    let keys = roots
        .iter()
        .map(|root| PathKey::of(root))
        .collect::<Vec<_>>();
    let mut changed = vec![false; roots.len()];
    let mut quiet_at: Option<Instant> = None;
    while !cancellation.is_cancelled() {
        let signal = match quiet_at {
            None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(at) => receiver.recv_timeout(at.saturating_duration_since(Instant::now())),
        };
        match signal {
            Ok(Signal::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(Signal::Changed(Ok(event))) => {
                // Reading a Skill (the catalog's own reads included) is an
                // access, not a change.
                if matches!(event.kind, EventKind::Access(_)) {
                    continue;
                }
                let mut attributed = !event.paths.is_empty() && !event.need_rescan();
                for path in &event.paths {
                    let key = PathKey::of(path);
                    match keys.iter().position(|root| key.starts_with(root)) {
                        Some(index) => changed[index] = true,
                        None => attributed = false,
                    }
                }
                // A change that names no root (an overflow, a rescan, a path
                // the OS spelled differently) may have touched any of them.
                if !attributed {
                    changed.fill(true);
                }
                quiet_at = Some(Instant::now() + DEBOUNCE);
            }
            Ok(Signal::Changed(Err(error))) => {
                log::warn!("Skill watcher reported an error: {error}");
                changed.fill(true);
                quiet_at = Some(Instant::now() + DEBOUNCE);
            }
            Err(RecvTimeoutError::Timeout) => {
                for (root, changed) in roots.iter().zip(changed.iter_mut()) {
                    if std::mem::take(changed) {
                        if let Err(error) = catalog.invalidate_root(root) {
                            log::warn!("Skill catalog invalidation failed: {error}");
                        }
                    }
                }
                quiet_at = None;
            }
        }
    }
}

#[cfg(test)]
mod tests;
