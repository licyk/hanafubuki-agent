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
fn multiple_processes_share_a_data_directory_but_exclusive_maintenance_blocks_them() {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let lock = DataDirLock::acquire(&data_dir).unwrap();
    assert_eq!(lock.directory(), data_dir);
    assert!(second_process(&data_dir).contains("CHILD-LOCKED"));
    let second = DataDirLock::acquire(&data_dir).unwrap();
    drop(second);
    drop(lock);
    let maintenance = OpenOptions::new()
        .read(true)
        .write(true)
        .open(data_dir.join(DATA_DIR_LOCK_FILE))
        .unwrap();
    maintenance.lock().unwrap();
    assert!(second_process(&data_dir).contains("CHILD-REFUSED"));
    assert!(DataDirLock::acquire(&data_dir).is_err());
    maintenance.unlock().unwrap();
    assert!(second_process(&data_dir).contains("CHILD-LOCKED"));
}

#[test]
fn a_dropped_lock_is_released_while_a_copy_of_its_file_is_still_open() {
    let directory = tempfile::tempdir().unwrap();
    let lock = DataDirLock::acquire(directory.path()).unwrap();
    // The copy a process forked to spawn a child holds until it execs: a
    // duplicate of the same open file.
    let inherited = lock.file.try_clone().unwrap();

    drop(lock);
    inherited.try_lock().unwrap();
    inherited.unlock().unwrap();
    drop(inherited);
}
