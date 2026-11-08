//! With the panic hook installed, a panic on an Agent runtime unwinds to its
//! task and is recorded instead of aborting the process. The scenario runs
//! this test binary again, because an abort cannot be observed from inside.

use std::process::{Command, Output};
use std::time::Duration;

use hanafubuki_agent_foundation::panic::{install_panic_hook, recorded_panics};
use hanafubuki_agent_runtime::{ContainedRuntime, RuntimeTaskFailure, AGENT_RUNTIME_THREAD_NAME};

const SCENARIO: &str = "HANAFUBUKI_CONTAINED_RUNTIME_PANIC_SCENARIO";
const CONTAINED_OK: &str = "contained runtime scenario finished";

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
    match scenario.as_str() {
        "agent-runtime" => panics_are_task_failures(AGENT_RUNTIME_THREAD_NAME),
        unknown => panic!("unknown scenario {unknown}"),
    }
}

/// A panic in a task and in blocking work of a runtime named `thread_name`
/// is that work's failure, recorded as contained on one of its threads.
fn panics_are_task_failures(thread_name: &'static str) {
    let runtime = ContainedRuntime::start(thread_name).unwrap();
    assert_eq!(
        runtime.block_on(async { panic!("in a contained task") }),
        Err::<(), _>(RuntimeTaskFailure::Panicked(
            "in a contained task".to_string()
        ))
    );
    let blocking_runtime = runtime.clone();
    let blocking = runtime
        .block_on(async move {
            blocking_runtime
                .run_blocking(|| -> () { panic!("in contained blocking work") })
                .await
        })
        .unwrap();
    assert_eq!(
        blocking,
        Err(RuntimeTaskFailure::Panicked(
            "in contained blocking work".to_string()
        ))
    );
    let faults = recorded_panics();
    assert_eq!(faults.count, 2);
    assert!(faults
        .recent
        .iter()
        .all(|fault| fault.contained && fault.thread == thread_name));
    runtime.shutdown(Duration::from_secs(5)).unwrap();
    println!("{CONTAINED_OK}");
}

fn run(scenario: &str) -> Output {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "child_scenario",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(SCENARIO, scenario)
        .output()
        .unwrap()
}

fn assert_contained(scenario: &str) {
    let output = run(scenario);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains(CONTAINED_OK),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_panic_on_the_agent_runtime_is_its_task_s_failure() {
    assert_contained("agent-runtime");
}
