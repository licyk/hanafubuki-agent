//! The panic hook's contract, observed from outside the process: an
//! unguarded panic aborts it, a contained one unwinds to its catch and is
//! recorded. Each scenario runs this test binary again with the hook
//! installed, because an abort cannot be observed from inside.

use std::process::{Command, Output};
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::background_worker::{
    spawn_contained_thread, BackgroundWorkerGroup,
};
use hanafubuki_agent_foundation::module_faults::{
    module_fault_handler, recorded_module_faults, FaultOwner,
};
use hanafubuki_agent_foundation::panic::{
    catch_panic, configure_crash_record, install_panic_hook, mark_thread_contained,
    read_crash_record, recorded_panics,
};

const SCENARIO: &str = "HANAFUBUKI_PANIC_HOOK_SCENARIO";
/// Where a child writes its crash record, when the scenario names one.
const CRASH_RECORD: &str = "HANAFUBUKI_PANIC_HOOK_CRASH_RECORD";
const CONTAINED_OK: &str = "contained scenario finished";

fn logging_never_blocked() -> bool {
    false
}

/// Runs only as the child of a test below.
#[test]
fn child_scenario() {
    let Ok(scenario) = std::env::var(SCENARIO) else {
        return;
    };
    install_panic_hook(logging_never_blocked);
    if let Ok(record) = std::env::var(CRASH_RECORD) {
        assert!(configure_crash_record(record.into()));
    }
    match scenario.as_str() {
        "unguarded-thread" => {
            let _ = std::thread::Builder::new()
                .name("plain-thread".to_string())
                .spawn(|| panic!("an unguarded panic"))
                .unwrap()
                .join();
            println!("the process outlived an unguarded panic");
        }
        "default-worker" => {
            let group = BackgroundWorkerGroup::default();
            group
                .spawn("plain-worker", |_| panic!("a default worker panic"))
                .unwrap();
            let _ = group.shutdown(Instant::now() + Duration::from_secs(5));
            println!("the process outlived a default worker's panic");
        }
        "contained" => {
            assert_eq!(
                catch_panic(|| panic!("inside a guard")),
                Err::<(), _>("inside a guard".to_string())
            );
            let marked = std::thread::Builder::new()
                .name("marked-thread".to_string())
                .spawn(|| {
                    mark_thread_contained();
                    std::panic::catch_unwind(|| panic!("on a marked thread")).is_err()
                })
                .unwrap()
                .join()
                .unwrap();
            assert!(marked);
            let group = BackgroundWorkerGroup::contained();
            let worker = group
                .spawn_observed("contained-worker", |_| panic!("in a contained worker"))
                .unwrap();
            let _ = group.shutdown(Instant::now() + Duration::from_secs(5));
            assert_eq!(
                worker.panic_message().as_deref(),
                Some("in a contained worker")
            );
            let faults = recorded_panics();
            assert_eq!(faults.count, 3);
            assert!(faults.recent.iter().all(|fault| fault.contained));
            assert_eq!(
                faults
                    .recent
                    .iter()
                    .map(|fault| (fault.thread.as_str(), fault.message.as_str()))
                    .collect::<Vec<_>>(),
                [
                    ("child_scenario", "inside a guard"),
                    ("marked-thread", "on a marked thread"),
                    ("contained-worker", "in a contained worker"),
                ]
            );
            println!("{CONTAINED_OK}");
        }
        "module-faults" => {
            // A module's worker group and a core thread outside any group,
            // as the application builds them.
            let group = BackgroundWorkerGroup::contained_with_panic_handler(module_fault_handler(
                FaultOwner::Module("probe-module"),
            ));
            let worker = group
                .spawn_observed("module-worker", |_| panic!("in a module worker"))
                .unwrap();
            let _ = group.shutdown(Instant::now() + Duration::from_secs(5));
            assert_eq!(
                worker.panic_message().as_deref(),
                Some("in a module worker")
            );
            let joined = spawn_contained_thread(FaultOwner::Core, "core-thread", || -> u8 {
                panic!("on a core thread")
            })
            .unwrap()
            .join();
            assert!(joined.is_err(), "join still sees the thread's panic");
            assert_eq!(
                spawn_contained_thread(FaultOwner::Core, "core-thread", || 7)
                    .unwrap()
                    .join()
                    .unwrap(),
                7
            );
            let faults = recorded_module_faults();
            assert_eq!(faults.count, 2);
            assert_eq!(
                faults
                    .recent
                    .iter()
                    .map(|fault| (
                        fault.owner,
                        fault.component.as_str(),
                        fault.message.as_str()
                    ))
                    .collect::<Vec<_>>(),
                [
                    (
                        FaultOwner::Module("probe-module"),
                        "module-worker",
                        "in a module worker"
                    ),
                    (FaultOwner::Core, "core-thread", "on a core thread"),
                ]
            );
            let panics = recorded_panics();
            assert_eq!(panics.count, 2);
            assert!(panics.recent.iter().all(|fault| fault.contained));
            println!("{CONTAINED_OK}");
        }
        unknown => panic!("unknown scenario {unknown}"),
    }
}

fn run(scenario: &str) -> Output {
    child(scenario).output().unwrap()
}

fn child(scenario: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "child_scenario",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(SCENARIO, scenario);
    command
}

fn assert_aborted(output: &Output, thread: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "the child exited normally: {stderr}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(output.status.signal(), Some(6), "not SIGABRT: {stderr}");
    }
    assert!(
        stderr.contains(&format!("thread '{thread}' panicked")) && stderr.contains("fatal"),
        "the hook did not report the panic: {stderr}"
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("outlived"),
        "the child kept running after the panic"
    );
}

#[test]
fn an_unguarded_panic_aborts_the_process() {
    assert_aborted(&run("unguarded-thread"), "plain-thread");
}

#[test]
fn a_fatal_panic_leaves_a_crash_record_and_a_contained_one_does_not() {
    let directory = tempfile::tempdir().unwrap();
    let record = directory.path().join("crash-record.json");

    let output = child("unguarded-thread")
        .env(CRASH_RECORD, &record)
        .output()
        .unwrap();

    assert_aborted(&output, "plain-thread");
    let crash = read_crash_record(&record).expect("the fatal panic left a crash record");
    assert_eq!(crash.thread, "plain-thread");
    assert_eq!(crash.message, "an unguarded panic");
    assert!(crash.location.contains("panic_hook.rs"), "{crash:?}");
    assert_ne!(crash.pid, std::process::id());

    std::fs::remove_file(&record).unwrap();
    let contained = child("contained")
        .env(CRASH_RECORD, &record)
        .output()
        .unwrap();
    assert!(contained.status.success());
    assert!(!record.exists(), "a contained panic wrote a crash record");
}

#[test]
fn a_default_worker_panic_aborts_the_process() {
    assert_aborted(&run("default-worker"), "plain-worker");
}

#[test]
fn contained_panics_unwind_to_their_catch_and_are_recorded() {
    let output = run("contained");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains(CONTAINED_OK),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(
        stderr.matches("contained: unwinding to its catch").count(),
        3
    );
}

#[test]
fn module_and_core_faults_unwind_and_reach_the_fault_sink() {
    let output = run("module-faults");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains(CONTAINED_OK),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    // Reported once each by the hook; resuming the core thread's unwind is
    // not a second panic.
    assert_eq!(
        stderr.matches("contained: unwinding to its catch").count(),
        2
    );
}
