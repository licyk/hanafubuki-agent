use super::*;

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Generous: a slow CI filesystem may take a while to report a change.
const EVENT_DEADLINE: Duration = Duration::from_secs(20);

#[derive(Default)]
struct CountingCatalog {
    invalidations: AtomicUsize,
    armed: AtomicBool,
}

impl SkillCatalogInvalidationPort for CountingCatalog {
    fn invalidate_root(&self, _root: &Path) -> Result<(), String> {
        self.invalidations.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn set_watcher_diagnostic(&self, message: Option<String>) {
        self.armed.store(message.is_none(), Ordering::SeqCst);
    }
}

fn wait_until(condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + EVENT_DEADLINE;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    condition()
}

/// Starts a watch of `roots` and waits until the OS watch is armed, so a
/// change made afterwards is reported.
fn armed_watch(roots: &[PathBuf]) -> (SkillCatalogWatcher, Arc<CountingCatalog>) {
    let watcher = SkillCatalogWatcher::idle();
    let catalog = Arc::new(CountingCatalog::default());
    watcher.restart(roots, catalog.clone());
    assert!(
        wait_until(|| catalog.armed.load(Ordering::SeqCst)),
        "the watch did not arm"
    );
    (watcher, catalog)
}

#[test]
fn a_change_under_a_root_invalidates_it_after_the_debounce() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("user");
    std::fs::create_dir_all(root.join("demo")).unwrap();
    // A root that does not exist yet is left out rather than failing the
    // watch; catalog reads find it once it appears.
    let (watcher, catalog) = armed_watch(&[root.clone(), temporary.path().join("absent")]);

    std::fs::write(root.join("demo/SKILL.md"), "first").unwrap();
    std::fs::write(root.join("demo/SKILL.md"), "a longer second value").unwrap();

    assert!(wait_until(
        || catalog.invalidations.load(Ordering::SeqCst) > 0
    ));
    assert!(watcher
        .shutdown(Instant::now() + Duration::from_secs(5))
        .is_empty());
}

#[test]
fn reading_a_skill_is_not_a_change() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("user");
    std::fs::create_dir_all(root.join("demo")).unwrap();
    std::fs::write(root.join("demo/SKILL.md"), "instructions").unwrap();
    let (watcher, catalog) = armed_watch(std::slice::from_ref(&root));

    assert_eq!(
        std::fs::read_to_string(root.join("demo/SKILL.md")).unwrap(),
        "instructions"
    );
    // Long past the debounce: a read reported as a change would have
    // invalidated by now. Waiting cannot make this fail spuriously.
    std::thread::sleep(DEBOUNCE * 6);

    assert_eq!(catalog.invalidations.load(Ordering::SeqCst), 0);
    assert!(watcher
        .shutdown(Instant::now() + Duration::from_secs(5))
        .is_empty());
}

#[test]
fn a_restarted_watch_stops_the_previous_one() {
    let temporary = tempfile::tempdir().unwrap();
    let first = temporary.path().join("first");
    let second = temporary.path().join("second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let (watcher, previous) = armed_watch(std::slice::from_ref(&first));
    let current = Arc::new(CountingCatalog::default());
    watcher.restart(std::slice::from_ref(&second), current.clone());
    assert!(wait_until(|| current.armed.load(Ordering::SeqCst)));

    std::fs::write(first.join("SKILL.md"), "unwatched").unwrap();
    std::fs::write(second.join("SKILL.md"), "watched").unwrap();

    assert!(wait_until(
        || current.invalidations.load(Ordering::SeqCst) > 0
    ));
    assert_eq!(previous.invalidations.load(Ordering::SeqCst), 0);
    assert!(watcher
        .shutdown(Instant::now() + Duration::from_secs(5))
        .is_empty());
}
