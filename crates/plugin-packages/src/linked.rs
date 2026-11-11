//! The packages linked for development (`plugins link`), watched while the
//! guest host runs. The OS reports changes under each linked directory
//! (`notify`); once the directories have been quiet for [`DEBOUNCE`] after
//! a change, each that changed counts one more revision, which names its
//! guest's loader entry anew, and [`GuestHost::linked_changes`] moves: the
//! facade then reloads the packages, which re-reads their content and,
//! through the new entry name, restarts the guest. Reading a file is not a
//! change, and neither is a change in a directory's `.git`.
//!
//! A linked directory that does not exist, or is removed (an export that
//! replaces it), is watched again once it is a directory, and that counts
//! as its change. The watch runs on a thread of the instance's contained
//! worker group; the group's shutdown, or the guest host's own stop, ends
//! it within [`POLL`].
//!
//! [`GuestHost::linked_changes`]: crate::GuestHost::linked_changes

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_foundation::paths::{PathKey, ResolvedPath};
use hanafubuki_agent_packages::PluginId;
use notify::{EventKind, RecursiveMode, Watcher};
use parking_lot::Mutex;
use tokio::sync::watch;

/// How long a linked directory must be quiet after a change: one save or
/// build writes many files.
pub(crate) const DEBOUNCE: Duration = Duration::from_millis(500);

/// How often the watch checks that it should stop.
const POLL: Duration = Duration::from_millis(100);

/// A package linked for development: its id and its directory, which is the
/// package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedPackage {
    pub id: PluginId,
    pub root: PathBuf,
}

pub(crate) struct LinkedWatch {
    workers: BackgroundWorkerGroup,
    revisions: Arc<Revisions>,
    watch: Mutex<WatchState>,
}

/// Each linked package's revision, and the counter the facade follows.
struct Revisions {
    counts: Mutex<BTreeMap<String, u64>>,
    changes: watch::Sender<u64>,
}

#[derive(Default)]
struct WatchState {
    packages: Vec<LinkedPackage>,
    mounted: bool,
    /// Stops the running watch.
    current: Option<CancellationToken>,
    /// The ids of the running watch's packages whose directory it watches
    /// now; a new set for each watch, so a stopping one never writes it.
    watching: Arc<Mutex<BTreeSet<String>>>,
}

/// A linked directory as the watch follows it.
struct Watched {
    id: String,
    root: PathBuf,
    /// The directory as it is watched; `None` while it is not: it did not
    /// exist, or it was removed.
    armed: Option<Armed>,
    /// Its watch's [`WatchState::watching`].
    watching: Arc<Mutex<BTreeSet<String>>>,
}

struct Armed {
    directory: ResolvedPath,
    git: PathKey,
}

impl LinkedWatch {
    pub(crate) fn new(workers: BackgroundWorkerGroup) -> Self {
        Self {
            workers,
            revisions: Arc::new(Revisions {
                counts: Mutex::default(),
                changes: watch::Sender::new(0),
            }),
            watch: Mutex::default(),
        }
    }

    /// Takes `packages` as the linked ones from now on, watching them in
    /// place of the last set while the guest host runs.
    pub(crate) fn follow(&self, packages: Vec<LinkedPackage>) {
        let mut state = self.watch.lock();
        if same_directories(&state.packages, &packages) {
            return;
        }
        state.packages = packages;
        if state.mounted {
            self.restart(&mut state);
        }
    }

    /// The guest host runs: the watch starts.
    pub(crate) fn start(&self) {
        let mut state = self.watch.lock();
        state.mounted = true;
        self.restart(&mut state);
    }

    /// The guest host stopped: the watch ends.
    pub(crate) fn stop(&self) {
        let mut state = self.watch.lock();
        state.mounted = false;
        if let Some(current) = state.current.take() {
            current.cancel();
        }
        state.watching = Arc::default();
    }

    /// How many changes of the linked package `id`'s directory were seen.
    pub(crate) fn revision(&self, id: &str) -> u64 {
        self.revisions.counts.lock().get(id).copied().unwrap_or(0)
    }

    pub(crate) fn changes(&self) -> watch::Receiver<u64> {
        self.revisions.changes.subscribe()
    }

    /// The ids of the linked packages whose directory the OS watch follows
    /// now.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn watched(&self) -> BTreeSet<String> {
        self.watch.lock().watching.lock().clone()
    }

    fn restart(&self, state: &mut WatchState) {
        if let Some(previous) = state.current.take() {
            previous.cancel();
        }
        state.watching = Arc::default();
        if state.packages.is_empty() {
            return;
        }
        let packages = state.packages.clone();
        let revisions = self.revisions.clone();
        let watching = state.watching.clone();
        let cancellation = CancellationToken::default();
        let started = self.workers.spawn_with_cancellation(
            "plugin-linked-watcher",
            cancellation.clone(),
            move |cancellation| watch_directories(&packages, &revisions, &watching, &cancellation),
        );
        match started {
            Ok(_) => state.current = Some(cancellation),
            Err(error) => log::warn!("the linked plugins are not watched: {error}"),
        }
    }
}

fn same_directories(known: &[LinkedPackage], packages: &[LinkedPackage]) -> bool {
    let keys = |packages: &[LinkedPackage]| {
        packages
            .iter()
            .map(|package| (package.id.key(), PathKey::of(&package.root)))
            .collect::<BTreeSet<_>>()
    };
    keys(known) == keys(packages)
}

/// Watches the directories of `packages` until `cancellation`, counting
/// their changes into `revisions`.
// TODO: on macOS (FSEvents, notify 8) a watch armed here once reported no
// change for 60 s in CI; the cause is open (todo.md, M7), and the facade's
// linked-package test is skipped there until it is fixed.
fn watch_directories(
    packages: &[LinkedPackage],
    revisions: &Revisions,
    watching: &Arc<Mutex<BTreeSet<String>>>,
    cancellation: &CancellationToken,
) {
    let (events, received) = mpsc::channel();
    // Dropped with this worker, which ends the OS watch.
    let watcher = notify::recommended_watcher(move |event| {
        let _ = events.send(event);
    });
    let mut watcher = match watcher {
        Ok(watcher) => watcher,
        Err(error) => {
            log::warn!("the linked plugins are not watched: {error}");
            return;
        }
    };
    let mut watched = packages
        .iter()
        .map(|package| Watched {
            id: package.id.key(),
            root: package.root.clone(),
            armed: None,
            watching: watching.clone(),
        })
        .collect::<Vec<_>>();
    for directory in &mut watched {
        if !directory.arm(&mut watcher) {
            log::warn!(
                "plugin {}: its linked directory {} is not a directory; it is watched once it is",
                directory.id,
                directory.root.display()
            );
        }
    }
    count_changes(
        &mut watched,
        &mut watcher,
        &received,
        cancellation,
        revisions,
    );
}

impl Watched {
    /// Watches its directory, when it is one now; whether it is watched.
    fn arm(&mut self, watcher: &mut impl Watcher) -> bool {
        let Ok(directory) = ResolvedPath::resolve(&self.root) else {
            return false;
        };
        if !directory.system().is_dir() {
            return false;
        }
        if let Err(error) = watcher.watch(directory.system(), RecursiveMode::Recursive) {
            log::warn!(
                "plugin {}: its linked directory is not watched: {error}",
                self.id
            );
            return false;
        }
        self.armed = Some(Armed {
            git: PathKey::of(&directory.system().join(".git")),
            directory,
        });
        self.watching.lock().insert(self.id.clone());
        true
    }

    /// Stops watching its directory, which `event` removed or which is gone:
    /// the OS watch went with it, and the directory made again is watched
    /// anew.
    fn disarm_if_gone(&mut self, watcher: &mut impl Watcher, event: Option<&notify::Event>) {
        let Some(armed) = &self.armed else {
            return;
        };
        let removed = event.is_some_and(|event| {
            matches!(event.kind, EventKind::Remove(_))
                && event
                    .paths
                    .iter()
                    .any(|path| PathKey::of(path) == *armed.directory.key())
        });
        if removed || !armed.directory.system().is_dir() {
            let _ = watcher.unwatch(armed.directory.system());
            self.armed = None;
            self.watching.lock().remove(&self.id);
        }
    }
}

/// Collects the directories each change touched, and counts a revision of
/// each once the changes have stopped for [`DEBOUNCE`]. A directory not
/// watched is tried again at every [`POLL`], and its appearance is a
/// change.
fn count_changes(
    watched: &mut [Watched],
    watcher: &mut impl Watcher,
    received: &Receiver<notify::Result<notify::Event>>,
    cancellation: &CancellationToken,
    revisions: &Revisions,
) {
    let mut changed = BTreeSet::<usize>::new();
    let mut quiet_at: Option<Instant> = None;
    while !cancellation.is_cancelled() {
        let wait = quiet_at.map_or(POLL, |at| {
            at.saturating_duration_since(Instant::now()).min(POLL)
        });
        let mut touched = BTreeSet::new();
        match received.recv_timeout(wait) {
            Ok(Ok(event)) => {
                touched = touched_by(watched, &event);
                for directory in watched.iter_mut() {
                    directory.disarm_if_gone(watcher, Some(&event));
                }
            }
            Ok(Err(error)) => {
                log::warn!("the linked plugins' watch reported an error: {error}");
                touched.extend(0..watched.len());
            }
            Err(RecvTimeoutError::Timeout) => {
                for (index, directory) in watched.iter_mut().enumerate() {
                    directory.disarm_if_gone(watcher, None);
                    if directory.armed.is_none() && directory.arm(watcher) {
                        touched.insert(index);
                    }
                }
                if touched.is_empty() && quiet_at.is_some_and(|at| Instant::now() >= at) {
                    let ids = std::mem::take(&mut changed)
                        .into_iter()
                        .map(|index| watched[index].id.clone());
                    revisions.count(ids);
                    quiet_at = None;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if !touched.is_empty() {
            changed.extend(touched);
            quiet_at = Some(Instant::now() + DEBOUNCE);
        }
    }
}

/// The directories `event` changed: none for a read or a change in a
/// `.git`; every one for a change that names no path under one (an
/// overflow, a rescan, a path the OS spelled differently).
fn touched_by(watched: &[Watched], event: &notify::Event) -> BTreeSet<usize> {
    if matches!(event.kind, EventKind::Access(_)) {
        return BTreeSet::new();
    }
    let mut touched = BTreeSet::new();
    let mut attributed = !event.paths.is_empty() && !event.need_rescan();
    for path in &event.paths {
        let key = PathKey::of(path);
        let mut armed = watched.iter().map(|directory| directory.armed.as_ref());
        if armed
            .clone()
            .flatten()
            .any(|armed| key.starts_with(&armed.git))
        {
            continue;
        }
        match armed
            .position(|armed| armed.is_some_and(|armed| key.starts_with(armed.directory.key())))
        {
            Some(index) => {
                touched.insert(index);
            }
            None => attributed = false,
        }
    }
    if !attributed {
        touched.extend(0..watched.len());
    }
    touched
}

impl Revisions {
    fn count(&self, ids: impl Iterator<Item = String>) {
        let mut counts = self.counts.lock();
        for id in ids {
            let next = counts.get(&id).copied().unwrap_or(0) + 1;
            log::info!("plugin {id}: its linked directory changed");
            counts.insert(id, next);
        }
        drop(counts);
        self.changes.send_modify(|revision| *revision += 1);
    }
}
