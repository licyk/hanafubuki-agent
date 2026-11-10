//! A guest that fails while it runs restarts under the restart policy, a
//! fourth fault in the window leaves it failed until it is retried, a start
//! that fails is not restarted, and a package whose content changed does
//! not start.

use std::time::Duration;

use hanafubuki_agent_plugin_packages::{GuestExitCause, GuestState};
use serde_json::Value;

use crate::fixture::{kill_hard, running, Fixture, Options};

pub async fn a_killed_guest_fails_and_the_restart_loop_brings_it_back() {
    let fixture = Fixture::start(Options::default()).await;
    let package = fixture.package("demo", "run");
    let id = package.id.key();
    fixture.apply(vec![package.clone()]).await;
    let first = fixture.wait(&id, "start", running).await;

    kill_hard(first.process_id.unwrap());

    let back = fixture
        .wait(&id, "restart", |status| {
            running(status) && status.process_id != first.process_id
        })
        .await;
    assert_eq!(back.restarts, 1);
    let exit = back.last_exit.unwrap();
    assert_eq!(exit.cause, GuestExitCause::Exited);
    if cfg!(unix) {
        assert!(exit.status.unwrap().contains("9"), "{:?}", back.error);
    }
    assert!(fixture
        .call(&package, "echo", Value::Bool(true))
        .await
        .is_ok());
    fixture.wait_for_processes(1).await;
}

pub async fn a_call_past_its_deadline_kills_the_guest_which_restarts() {
    let fixture = Fixture::start(Options::default()).await;
    let package = fixture.package("demo", "run");
    let id = package.id.key();
    fixture.apply(vec![package.clone()]).await;
    let first = fixture.wait(&id, "start", running).await;

    let error = fixture
        .host
        .call(&package.id, "hang", Value::Null, Duration::from_millis(300))
        .await
        .unwrap_err();
    assert!(error.contains("did not answer in time"), "{error}");

    let back = fixture
        .wait(&id, "restart", |status| {
            running(status) && status.process_id != first.process_id
        })
        .await;
    assert_eq!(back.restarts, 1);
    let exit = back.last_exit.unwrap();
    assert_eq!(exit.cause, GuestExitCause::Killed);
    assert!(exit.reason.unwrap().contains("did not answer in time"));
}

pub async fn a_fourth_fault_in_the_window_leaves_the_guest_failed_until_it_is_retried() {
    let fixture = Fixture::start(Options::default()).await;
    let package = fixture.package("demo", "run");
    let id = package.id.key();
    fixture.apply(vec![package.clone()]).await;
    let mut current = fixture.wait(&id, "start", running).await;

    for restarts in 1..=3 {
        let _ = fixture.call(&package, "exit", Value::Null).await;
        let previous = current.process_id;
        current = fixture
            .wait(&id, "restart", |status| {
                running(status) && status.process_id != previous && status.restarts == restarts
            })
            .await;
    }
    let _ = fixture.call(&package, "exit", Value::Null).await;
    let failed = fixture
        .wait(&id, "failure", |status| status.state == GuestState::Failed)
        .await;
    assert_eq!(failed.restarts, 3);
    assert_eq!(failed.last_exit.unwrap().cause, GuestExitCause::Exited);
    fixture.wait_for_processes(0).await;
    assert!(fixture.call(&package, "echo", Value::Null).await.is_err());
    // Nothing reaches the settings: being stopped by faults is not being off.
    assert!(fixture.settings.0.lock().is_empty());

    fixture.host.retry(&package.id).await.unwrap();
    let retried = fixture.wait(&id, "start after the retry", running).await;
    assert_eq!(retried.restarts, 0);
    assert!(fixture.host.retry(&package.id).await.is_err());
}

pub async fn a_changed_package_does_not_start() {
    let fixture = Fixture::start(Options::default()).await;
    let package = fixture.package("demo", "run");
    let id = package.id.key();
    std::fs::write(
        package.root.join("package.txt"),
        "changed after the install",
    )
    .unwrap();

    fixture.apply(vec![package.clone()]).await;

    let failed = fixture
        .wait(&id, "refusal", |status| status.state == GuestState::Failed)
        .await;
    assert!(
        failed
            .error
            .as_deref()
            .unwrap()
            .contains("changed since it was installed"),
        "{failed:?}"
    );
    assert_eq!(failed.last_exit, None);
    assert_eq!(fixture.processes(), 0);
}

pub async fn a_start_that_hangs_or_fails_stays_failed() {
    let fixture = Fixture::start(Options {
        call_timeout: Duration::from_secs(1),
        ..Options::default()
    })
    .await;
    let hangs = fixture.package("hangs", "hang-apply");
    let fails = fixture.package("fails", "fail-apply");
    fixture.apply(vec![hangs.clone(), fails.clone()]).await;

    let hung = fixture
        .wait(&hangs.id.key(), "a failed start", |status| {
            status.state == GuestState::Failed
        })
        .await;
    let exit = hung.last_exit.unwrap();
    assert_eq!(exit.cause, GuestExitCause::Killed);
    assert!(exit.reason.unwrap().contains("did not answer in time"));
    assert_eq!(hung.restarts, 0);

    let refused = fixture
        .wait(&fails.id.key(), "a failed start", |status| {
            status.state == GuestState::Failed
        })
        .await;
    assert!(
        refused
            .error
            .as_deref()
            .unwrap()
            .contains("refuses to apply"),
        "{refused:?}"
    );
    assert_eq!(refused.restarts, 0);
    fixture.wait_for_processes(0).await;
}
