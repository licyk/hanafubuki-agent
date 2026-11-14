//! The watch of a linked directory against a watcher that, as Windows'
//! does, follows the directory it opened rather than its path: a directory
//! replaced (removed, another moved in) reports only its files' removal,
//! never its own, and the path is a directory again by the next poll.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use notify::{Config, Event, EventHandler, EventKind, RecursiveMode, Watcher, WatcherKind};

use super::*;

/// Records what it is asked to watch; its events are the test's.
#[derive(Default)]
struct HandleWatcher {
    watched: Arc<Mutex<Vec<PathBuf>>>,
}

impl Watcher for HandleWatcher {
    fn new<F: EventHandler>(_handler: F, _config: Config) -> notify::Result<Self> {
        Ok(Self::default())
    }

    fn watch(&mut self, path: &Path, _mode: RecursiveMode) -> notify::Result<()> {
        self.watched.lock().push(path.to_path_buf());
        Ok(())
    }

    fn unwatch(&mut self, _path: &Path) -> notify::Result<()> {
        Ok(())
    }

    fn kind() -> WatcherKind {
        WatcherKind::NullWatcher
    }
}

#[test]
fn a_replaced_directory_is_watched_anew_though_its_removal_is_not_reported() {
    let root = std::env::temp_dir().join(format!(
        "hanafubuki-linked-replaced-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    fs_reset(&root);
    std::fs::write(root.join("package.txt"), "first").unwrap();
    let watcher = HandleWatcher::default();
    let watched_paths = watcher.watched.clone();
    let revisions = Arc::new(Revisions {
        counts: Mutex::default(),
        changes: watch::Sender::new(0),
    });
    let (events, received) = mpsc::channel();
    let cancellation = CancellationToken::default();
    let worker = {
        let (revisions, cancellation, root) =
            (revisions.clone(), cancellation.clone(), root.clone());
        std::thread::spawn(move || {
            let mut watcher = watcher;
            let mut watched = vec![Watched {
                id: "demo@local".to_string(),
                root,
                armed: None,
                watching: Arc::default(),
            }];
            assert!(watched[0].arm(&mut watcher));
            count_changes(
                &mut watched,
                &mut watcher,
                &received,
                &cancellation,
                &revisions,
            );
        })
    };
    wait_for_watches(&watched_paths, 1, &worker);

    // Replaced as an export replaces it, between two polls: the watcher
    // reports the file's removal only.
    let copy = root.with_extension("next");
    fs_reset(&copy);
    std::fs::write(copy.join("package.txt"), "second").unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::rename(&copy, &root).unwrap();
    events
        .send(Ok(Event::new(EventKind::Remove(
            notify::event::RemoveKind::File,
        ))
        .add_path(root.join("package.txt"))))
        .unwrap();

    // The new directory is watched: its path is asked of the watcher again.
    wait_for_watches(&watched_paths, 2, &worker);
    cancellation.cancel();
    worker.join().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

fn fs_reset(directory: &Path) {
    let _ = std::fs::remove_dir_all(directory);
    std::fs::create_dir_all(directory).unwrap();
}

/// Waits up to 5 s until the watcher was asked to watch `count` paths; a
/// timeout says what it was asked and whether the watch still runs.
fn wait_for_watches(
    watched: &Mutex<Vec<PathBuf>>,
    count: usize,
    worker: &std::thread::JoinHandle<()>,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let paths = watched.lock().clone();
        if paths.len() == count {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{count} watches expected within 5 s; asked to watch {paths:?}, the watch {}",
            if worker.is_finished() {
                "ended"
            } else {
                "still runs"
            }
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
