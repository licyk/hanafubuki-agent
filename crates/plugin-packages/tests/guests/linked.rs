//! A linked package's guest runs from its own directory, which no content
//! hash pins; once that directory changes and is quiet for the debounce,
//! the guest host signals, and the packages applied again (the facade's
//! reload) start the guest anew. A directory replaced by another is
//! watched anew; one no longer linked is no longer watched.

use std::fs;
use std::time::{Duration, Instant};

use hanafubuki_agent_plugin_packages::{GuestPackage, LinkedPackage};
use tokio::sync::watch;

use crate::fixture::{running, Fixture, Options, DEADLINE};
use crate::watchdog::step;

/// The guest host's debounce, less what the clock reads of the event's
/// delivery before the write is timed.
const AT_LEAST: Duration = Duration::from_millis(400);

pub async fn a_linked_guest_starts_anew_once_its_directory_changes() {
    let fixture = Fixture::start(Options::default()).await;
    let mut package = fixture.package("demo", "run");
    package.linked = true;
    package.content_hash = None;
    let id = package.id.key();
    fixture.host.follow_linked(vec![linked(&package)]);
    let mut changes = fixture.host.linked_changes();
    fixture.apply(vec![package.clone()]).await;
    let first = fixture.wait(&id, "start", running).await;

    let elapsed = change_until_signalled(&package, &mut changes).await;
    assert!(elapsed >= AT_LEAST, "signalled after {elapsed:?}");
    // Applied again as the facade's reload does: the package is not as it
    // was when its guest started, and starts all the same.
    fixture.apply(vec![package.clone()]).await;
    let second = fixture
        .wait(&id, "start anew", |status| {
            running(status) && status.process_id != first.process_id
        })
        .await;
    assert_eq!(second.restarts, 0);
    assert_eq!(second.error, None);
    // Without a change, applying the packages leaves the guest running.
    fixture.apply(vec![package.clone()]).await;
    let kept = fixture.host.status(&id).unwrap();
    assert_eq!(kept.process_id, second.process_id);

    // Replaced as an export replaces it: removed, a copy moved in. The new
    // directory is watched: a change in it is seen.
    let copy = package.root.with_extension("next");
    copy_dir(&package.root, &copy);
    fs::remove_dir_all(&package.root).unwrap();
    fs::rename(&copy, &package.root).unwrap();
    change_until_signalled(&package, &mut changes).await;
    // Once quiet, a change made in the replaced directory is seen too.
    tokio::time::sleep(Duration::from_secs(1)).await;
    change_until_signalled(&package, &mut changes).await;

    fixture.host.follow_linked(Vec::new());
    changes.borrow_and_update();
    fs::write(package.root.join("package.txt"), "unwatched").unwrap();
    // Long past the debounce: a watched change would have signalled by now.
    let unwatched = tokio::time::timeout(Duration::from_secs(2), changes.changed()).await;
    assert!(unwatched.is_err(), "an unlinked directory signalled");
}

fn copy_dir(source: &std::path::Path, target: &std::path::Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn linked(package: &GuestPackage) -> LinkedPackage {
    LinkedPackage {
        id: package.id.clone(),
        root: package.root.clone(),
    }
}

/// Changes a file in `package`'s directory until the guest host signals,
/// and how long after the change that was. The OS watch is armed on its own
/// thread, so a change made before it is lost: it is made again.
async fn change_until_signalled(
    package: &GuestPackage,
    changes: &mut watch::Receiver<u64>,
) -> Duration {
    let deadline = Instant::now() + DEADLINE;
    let mut attempt = 0;
    loop {
        attempt += 1;
        changes.borrow_and_update();
        fs::write(
            package.root.join("package.txt"),
            format!("changed {attempt}"),
        )
        .unwrap();
        let written = Instant::now();
        step(format!("changed the linked directory ({attempt})"));
        if let Ok(signalled) = tokio::time::timeout(Duration::from_secs(3), changes.changed()).await
        {
            signalled.unwrap();
            return written.elapsed();
        }
        assert!(Instant::now() < deadline, "no signal within {DEADLINE:?}");
    }
}
