//! The owner-group sweep, and (Linux) the process-group ledger: a recorded
//! group that still runs as recorded is reaped, one whose leader cannot be
//! confirmed is left alone.

use std::time::Duration;

use super::*;
use crate::executor::manager::{executor_registry_counts, start_executor, terminate_owner_group};
use crate::executor::{
    EnvironmentInheritance, ExecutorIoMode, ExecutorManager, ExecutorStartRequest,
};

fn sleep_request() -> ExecutorStartRequest {
    ExecutorStartRequest {
        executor_id: None,
        command: vec!["sleep".to_string(), "30".to_string()],
        cwd: None,
        environment_inheritance: EnvironmentInheritance::Inherit,
        env: std::collections::HashMap::new(),
        io_mode: ExecutorIoMode::Pipe,
        stream_stdin: false,
        stream_output: true,
        cols: None,
        rows: None,
        timeout_ms: None,
        output_bytes_cap: None,
    }
}

#[test]
fn the_owner_group_sweep_disposes_only_that_groups_sessions() {
    let manager = ExecutorManager::default();
    let group = ProcessOwnerGroup::new();
    let other = ProcessOwnerGroup::new();
    start_executor(&manager.owned_by(group.clone()), sleep_request()).unwrap();
    start_executor(&manager.owned_by(group.clone()), sleep_request()).unwrap();
    let kept = start_executor(&manager.owned_by(other.clone()), sleep_request()).unwrap();
    assert_eq!(executor_registry_counts(&manager).0, 3);

    let failures = terminate_owner_group(&manager, &group, Duration::from_secs(5));

    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(executor_registry_counts(&manager).0, 1);
    assert!(crate::executor::manager::get_executor_info(&manager, &kept.executor_id).is_ok());
    assert!(terminate_owner_group(&manager, &other, Duration::from_secs(5)).is_empty());
    assert_eq!(executor_registry_counts(&manager), (0, 0, 0));
}

#[cfg(target_os = "linux")]
mod linux {
    use std::os::unix::process::CommandExt;
    use std::path::PathBuf;
    use std::process::{Child, Command};
    use std::time::Instant;

    use super::*;

    fn record_file(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "hanafubuki-agent-process-groups-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory.join("process-groups.json")
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "{what} did not happen in time");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// `sleep` in its own session, as the Executor starts its children.
    fn orphan_like_sleep() -> Child {
        let mut command = Command::new("sleep");
        command.arg("30");
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        command.spawn().unwrap()
    }

    fn ended(child: &mut Child) -> bool {
        child.try_wait().unwrap().is_some()
    }

    #[test]
    fn a_recorded_group_that_still_runs_as_recorded_is_reaped() {
        let file = record_file("reaped");
        let record = ProcessGroupRecord::new(file.clone());
        let mut child = orphan_like_sleep();
        let pgid = child.id();
        assert!(record.add(pgid));
        assert!(file.exists());

        // The next lifetime, with a ledger of its own over the same file.
        let reap = ProcessGroupRecord::new(file.clone()).reap_leftovers();

        assert_eq!(reap.terminated, vec![pgid]);
        assert_eq!(reap.unconfirmed, 0);
        assert!(reap.failed.is_empty(), "{:?}", reap.failed);
        wait_until("the reaped group ending", || ended(&mut child));
        assert!(!file.exists(), "the reaped group stayed recorded");
    }

    /// Checked on the guard itself: a ledger naming init's or this process's
    /// real group would signal it if the guard regressed.
    #[test]
    fn the_kernel_init_and_own_groups_are_never_reaped() {
        let (own_group, own_pid) = unsafe { (libc::getpgrp(), libc::getpid()) };
        for pgid in [0, 1, own_group as u32, own_pid as u32] {
            assert!(is_protected_group(pgid), "group {pgid} is not protected");
        }
        let child = orphan_like_sleep();
        assert!(!is_protected_group(child.id()));
        drop_child(child);

        let file = record_file("protected");
        fs::write(
            &file,
            serde_json::to_vec(&RecordDocument {
                groups: vec![RecordedGroup {
                    pgid: 0,
                    leader_identity: "any".to_string(),
                }],
            })
            .unwrap(),
        )
        .unwrap();
        let reap = ProcessGroupRecord::new(file).reap_leftovers();
        assert!(reap.terminated.is_empty());
        assert_eq!(reap.unconfirmed, 1);
    }

    fn drop_child(mut child: Child) {
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn a_group_whose_leader_identity_does_not_match_is_left_alone() {
        let file = record_file("mismatched");
        let mut child = orphan_like_sleep();
        let pgid = child.id();
        // Recorded from another boot, or by a process that since reused the id.
        fs::write(
            &file,
            serde_json::to_vec(&RecordDocument {
                groups: vec![RecordedGroup {
                    pgid,
                    leader_identity: "another-boot:1".to_string(),
                }],
            })
            .unwrap(),
        )
        .unwrap();

        let reap = ProcessGroupRecord::new(file.clone()).reap_leftovers();

        assert!(reap.terminated.is_empty());
        assert_eq!(reap.unconfirmed, 1);
        std::thread::sleep(Duration::from_millis(400));
        assert!(!ended(&mut child), "an unconfirmed group was signalled");
        assert!(!file.exists(), "the unconfirmed entry was kept");
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn a_recorded_owner_group_records_its_sessions_until_their_process_ends() {
        let file = record_file("sessions");
        let manager = ExecutorManager::default();
        let group = ProcessOwnerGroup::recorded_in(ProcessGroupRecord::new(file.clone()));
        let started = start_executor(&manager.owned_by(group.clone()), sleep_request()).unwrap();
        let recorded: RecordDocument = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        assert_eq!(recorded.groups.len(), 1);
        // An unscoped session is never recorded.
        let unscoped = start_executor(&manager, sleep_request()).unwrap();
        let recorded: RecordDocument = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        assert_eq!(recorded.groups.len(), 1);

        assert!(terminate_owner_group(&manager, &group, Duration::from_secs(5)).is_empty());

        wait_until("the ended session leaving the record", || !file.exists());
        assert!(
            crate::executor::manager::get_executor_info(&manager, &started.executor_id).is_err()
        );
        crate::executor::manager::dispose_executor(&manager, &unscoped.executor_id).unwrap();
    }
}
