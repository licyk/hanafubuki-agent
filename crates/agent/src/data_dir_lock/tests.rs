use std::process::Command;

use super::*;

const CHILD_DIRECTORY: &str = "HANAFUBUKI_AGENT_TEST_LOCK_DIRECTORY";

/// The second process of the test below: tries the lock of the directory it
/// is given, and says how that went. Run alone, it has nothing to do.
#[test]
fn child_takes_the_lock() {
    let Some(directory) = std::env::var_os(CHILD_DIRECTORY) else {
        return;
    };
    match DataDirLock::acquire(Path::new(&directory)) {
        Ok(_) => println!("CHILD-LOCKED"),
        Err(error) => println!("CHILD-REFUSED {error}"),
    }
}

fn second_process(directory: &Path) -> String {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "data_dir_lock::tests::child_takes_the_lock",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_DIRECTORY, directory)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_second_process_is_refused_a_held_data_directory_until_it_is_released() {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let lock = DataDirLock::acquire(&data_dir).unwrap();
    assert_eq!(lock.directory(), data_dir);
    assert!(data_dir.join(DATA_DIR_LOCK_FILE).is_file());

    let refused = second_process(&data_dir);
    assert!(refused.contains("CHILD-REFUSED"), "{refused}");
    assert!(
        refused.contains(&format!(
            "the Agent data directory {} is in use by another process",
            data_dir.display()
        )),
        "{refused}"
    );
    // Another handle in this process is refused as well.
    assert!(DataDirLock::acquire(&data_dir).is_err());

    drop(lock);
    let taken = second_process(&data_dir);
    assert!(taken.contains("CHILD-LOCKED"), "{taken}");
    DataDirLock::acquire(&data_dir).unwrap();
}

#[test]
fn a_dropped_lock_is_released_while_a_copy_of_its_file_is_still_open() {
    let directory = tempfile::tempdir().unwrap();
    let lock = DataDirLock::acquire(directory.path()).unwrap();
    // The copy a process forked to spawn a child holds until it execs: a
    // duplicate of the same open file.
    let inherited = lock.file.try_clone().unwrap();

    drop(lock);
    DataDirLock::acquire(directory.path()).unwrap();
    drop(inherited);
}
