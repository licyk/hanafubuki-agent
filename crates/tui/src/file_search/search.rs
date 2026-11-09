// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

//! Codex's `codex-file-search` crate, the session half the interface uses:
//! the workspace roots walked once (honouring `.gitignore`) into a `nucleo`
//! matcher, and every query update re-ranked against what was walked so far.
//!
//! Ported without its command line, its one-shot `run`, the exclusion
//! globs, the shared cancellation flag and the matches' scores (nucleo
//! 0.5 on crates.io has no accessor for them; Codex pins a git revision of
//! nucleo that does, and nothing here reads them); the matcher loop waits on a std
//! channel with a deadline in place of `crossbeam_channel::select!`, and a
//! root nested in another is walked once, as its own root.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use ignore::WalkBuilder;
use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config, Injector, Matcher, Nucleo, Utf32String};

/// How many of the best matches a snapshot carries.
const LIMIT: u32 = 20;
/// The walker's and the matcher's threads.
const THREADS: usize = 2;
/// How long one matcher tick may take, and how soon after the matcher
/// reports progress the next snapshot is taken.
const TICK_TIMEOUT_MS: u64 = 10;
/// How often an idle matcher looks at the shutdown flag.
const IDLE_CHECK: Duration = Duration::from_millis(100);

/// A single match result returned from the search.
///
/// * `path`  – Path to the matched file (relative to `root`).
/// * `indices` – The character indices of `path` that matched the query,
///   unique and sorted in ascending order, for highlighting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileMatch {
    pub(crate) path: PathBuf,
    pub(crate) root: PathBuf,
    /// Whether `root` is the first workspace root, the one a relative path
    /// is read against.
    pub(crate) primary: bool,
    pub(crate) indices: Vec<u32>,
}

impl FileMatch {
    /// What a mention of the file inserts: the path relative to the first
    /// root, or the whole path for a file in another root.
    pub(crate) fn mention(&self) -> String {
        if self.primary {
            self.path.to_string_lossy().into_owned()
        } else {
            self.root.join(&self.path).to_string_lossy().into_owned()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct FileSearchSnapshot {
    pub(crate) query: String,
    pub(crate) matches: Vec<FileMatch>,
    pub(crate) total_match_count: usize,
    pub(crate) scanned_file_count: usize,
    pub(crate) walk_complete: bool,
}

pub(crate) trait SessionReporter: Send + Sync + 'static {
    /// Called when the debounced top-N changes.
    fn on_update(&self, snapshot: &FileSearchSnapshot);

    /// Called when the session becomes idle or is shut down. Guaranteed to
    /// be called at least once per update_query.
    fn on_complete(&self);
}

pub(crate) struct FileSearchSession {
    inner: Arc<SessionInner>,
}

impl FileSearchSession {
    /// Update the query. This should be cheap relative to re-walking.
    pub(crate) fn update_query(&self, pattern_text: &str) {
        let _ = self
            .inner
            .work_tx
            .send(WorkSignal::QueryUpdated(pattern_text.to_string()));
    }
}

impl Drop for FileSearchSession {
    fn drop(&mut self) {
        self.inner.shutdown.store(true, Ordering::Relaxed);
        let _ = self.inner.work_tx.send(WorkSignal::Shutdown);
    }
}

/// Starts walking `roots` (the first is the primary one) on background
/// threads; the matches of each query reach `reporter`.
pub(crate) fn create_session(
    roots: Vec<PathBuf>,
    reporter: Arc<dyn SessionReporter>,
) -> FileSearchSession {
    let (work_tx, work_rx) = channel();

    let notify_tx = work_tx.clone();
    let notify = Arc::new(move || {
        let _ = notify_tx.send(WorkSignal::NucleoNotify);
    });
    let nucleo = Nucleo::new(Config::DEFAULT.match_paths(), notify, Some(THREADS), 1);
    let injector = nucleo.injector();

    let inner = Arc::new(SessionInner {
        search_directories: roots,
        shutdown: Arc::new(AtomicBool::new(false)),
        reporter,
        work_tx,
    });

    let matcher_inner = inner.clone();
    thread::spawn(move || matcher_worker(matcher_inner, work_rx, nucleo));

    let walker_inner = inner.clone();
    thread::spawn(move || walker_worker(walker_inner, injector));

    FileSearchSession { inner }
}

struct SessionInner {
    search_directories: Vec<PathBuf>,
    shutdown: Arc<AtomicBool>,
    reporter: Arc<dyn SessionReporter>,
    work_tx: Sender<WorkSignal>,
}

enum WorkSignal {
    QueryUpdated(String),
    NucleoNotify,
    WalkComplete,
    Shutdown,
}

fn get_file_path<'a>(path: &'a Path, search_directories: &[PathBuf]) -> Option<(usize, &'a str)> {
    let mut best_match: Option<(usize, &Path)> = None;
    for (idx, root) in search_directories.iter().enumerate() {
        if let Ok(rel_path) = path.strip_prefix(root) {
            let root_depth = root.components().count();
            match best_match {
                Some((best_idx, _))
                    if search_directories[best_idx].components().count() >= root_depth => {}
                _ => {
                    best_match = Some((idx, rel_path));
                }
            }
        }
    }

    let (root_idx, rel_path) = best_match?;
    rel_path.to_str().map(|p| (root_idx, p))
}

fn walker_worker(inner: Arc<SessionInner>, injector: Injector<Arc<str>>) {
    let Some(first_root) = inner.search_directories.first() else {
        let _ = inner.work_tx.send(WorkSignal::WalkComplete);
        return;
    };

    let mut walk_builder = WalkBuilder::new(first_root);
    for root in inner.search_directories.iter().skip(1) {
        walk_builder.add(root);
    }
    let roots = inner.search_directories.clone();
    walk_builder
        .threads(THREADS)
        // Allow hidden entries.
        .hidden(false)
        // Follow symlinks to search their contents.
        .follow_links(true)
        // Don't require git to be present to apply to apply git-related ignore rules.
        .require_git(false)
        // An entry's path is its walk root joined with what follows, so a
        // root nested in another is met as the very path it was given; it
        // is walked as its own root, not twice.
        .filter_entry(move |entry| entry.depth() == 0 || !roots.iter().any(|r| entry.path() == r));

    let walker = walk_builder.build_parallel();

    walker.run(|| {
        const CHECK_INTERVAL: usize = 1024;
        let mut n = 0;
        let search_directories = inner.search_directories.clone();
        let injector = injector.clone();
        let shutdown = inner.shutdown.clone();

        Box::new(move |entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => return ignore::WalkState::Continue,
            };
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                return ignore::WalkState::Continue;
            }
            let path = entry.path();
            let Some(full_path) = path.to_str() else {
                return ignore::WalkState::Continue;
            };
            if let Some((_, relative_path)) = get_file_path(path, &search_directories) {
                injector.push(Arc::from(full_path), |_, cols| {
                    cols[0] = Utf32String::from(relative_path);
                });
            }
            n += 1;
            if n >= CHECK_INTERVAL {
                if shutdown.load(Ordering::Relaxed) {
                    return ignore::WalkState::Quit;
                }
                n = 0;
            }
            ignore::WalkState::Continue
        })
    });
    let _ = inner.work_tx.send(WorkSignal::WalkComplete);
}

fn matcher_worker(
    inner: Arc<SessionInner>,
    work_rx: Receiver<WorkSignal>,
    mut nucleo: Nucleo<Arc<str>>,
) {
    let config = Config::DEFAULT.match_paths();
    let mut indices_matcher = Matcher::new(config);
    let shutdown_requested = || inner.shutdown.load(Ordering::Relaxed);

    let mut last_query = String::new();
    // When the next snapshot is due; `None` while none is.
    let mut next_notify: Option<Instant> = None;
    let mut walk_complete = false;

    loop {
        // A due snapshot is taken before more signals are read, so a
        // stream of the matcher's notifications cannot hold it off.
        if next_notify.is_some_and(|at| at <= Instant::now()) {
            next_notify = None;
            let status = nucleo.tick(TICK_TIMEOUT_MS);
            if status.changed {
                let snapshot = snapshot(
                    &inner,
                    &nucleo,
                    &mut indices_matcher,
                    &last_query,
                    walk_complete,
                );
                inner.reporter.on_update(&snapshot);
            }
            if !status.running && walk_complete {
                inner.reporter.on_complete();
            }
        }
        let wait = next_notify.map_or(IDLE_CHECK, |at| {
            at.saturating_duration_since(Instant::now())
        });
        match work_rx.recv_timeout(wait) {
            Ok(WorkSignal::QueryUpdated(query)) => {
                let append = query.starts_with(&last_query);
                nucleo.pattern.reparse(
                    0,
                    &query,
                    CaseMatching::Smart,
                    Normalization::Smart,
                    append,
                );
                last_query = query;
                next_notify = Some(Instant::now());
            }
            Ok(WorkSignal::NucleoNotify) => {
                next_notify
                    .get_or_insert_with(|| Instant::now() + Duration::from_millis(TICK_TIMEOUT_MS));
            }
            Ok(WorkSignal::WalkComplete) => {
                walk_complete = true;
                next_notify.get_or_insert_with(Instant::now);
            }
            Ok(WorkSignal::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            // Occasionally check the shutdown flag.
            Err(RecvTimeoutError::Timeout) => {}
        }

        if shutdown_requested() {
            break;
        }
    }

    // If we exited the loop, make sure the reporter is notified.
    inner.reporter.on_complete();
}

/// The best matches of the current pattern, with their highlighted
/// characters.
fn snapshot(
    inner: &SessionInner,
    nucleo: &Nucleo<Arc<str>>,
    indices_matcher: &mut Matcher,
    query: &str,
    walk_complete: bool,
) -> FileSearchSnapshot {
    let snapshot = nucleo.snapshot();
    let limit = snapshot.matched_item_count().min(LIMIT);
    let pattern = snapshot.pattern().column_pattern(0);
    let matches = snapshot
        .matched_items(..limit)
        .filter_map(|item| {
            let full_path = item.data.as_ref();
            let (root_idx, relative_path) =
                get_file_path(Path::new(full_path), &inner.search_directories)?;
            let mut indices = Vec::<u32>::new();
            let haystack = item.matcher_columns[0].slice(..);
            let _ = pattern.indices(haystack, indices_matcher, &mut indices);
            indices.sort_unstable();
            indices.dedup();
            Some(FileMatch {
                path: PathBuf::from(relative_path),
                root: inner.search_directories[root_idx].clone(),
                primary: root_idx == 0,
                indices,
            })
        })
        .collect();
    FileSearchSnapshot {
        query: query.to_string(),
        matches,
        total_match_count: snapshot.matched_item_count() as usize,
        scanned_file_count: snapshot.item_count() as usize,
        walk_complete,
    }
}

#[cfg(test)]
mod tests;
