//! A panic on one of kardia-hmr's guest threads is contained under the
//! Agent's panic hook, which aborts every panic it does not know to be
//! caught: the guest host registers kardia-hmr's containment check, so the
//! panic unwinds to kardia-hmr's catch, which kills the guest, and the
//! guest restarts. The hook is installed in a child process of its own
//! (this binary with [`CONTAINED_PANIC_ARG`]), so that an abort is seen as
//! the child's end and the other checks keep the default hook.

use std::process::Command;

use hanafubuki_agent_foundation::panic::{install_panic_hook, recorded_panics};
use serde_json::{json, Value};

use crate::fixture::{running, Fixture, Options};
use crate::logging::PANIC_ON_STDERR;

/// The first argument that makes this binary the child of
/// [`a_panic_on_a_guest_thread_is_contained_under_the_agents_hook`].
pub const CONTAINED_PANIC_ARG: &str = "--hanafubuki-test-contained-panic";

pub async fn a_panic_on_a_guest_thread_is_contained_under_the_agents_hook() {
    let output = Command::new(std::env::current_exe().unwrap())
        .arg(CONTAINED_PANIC_ARG)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "the child ended with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The child: with the Agent's hook installed, a guest's stderr line makes
/// the test logger panic on kardia-hmr's `guest-stderr` thread.
pub async fn child() {
    install_panic_hook(|| false);
    let fixture = Fixture::start(Options::default()).await;
    let package = fixture.package("demo", "run");
    let id = package.id.key();
    fixture.apply(vec![package.clone()]).await;
    let first = fixture.wait(&id, "start", running).await;

    fixture
        .call(&package, "stderr", json!(PANIC_ON_STDERR))
        .await
        .unwrap();

    let back = fixture
        .wait(&id, "restart", |status| {
            running(status) && status.process_id != first.process_id
        })
        .await;
    assert_eq!(back.restarts, 1);
    let reason = back.last_exit.and_then(|exit| exit.reason);
    assert!(
        reason
            .as_deref()
            .unwrap_or_default()
            .contains("the guest-stderr thread panicked"),
        "{reason:?}"
    );
    let panics = recorded_panics().recent;
    assert!(
        panics
            .iter()
            .any(|fault| fault.thread == "guest-stderr" && fault.contained),
        "{panics:?}"
    );
    assert_eq!(
        fixture.call(&package, "echo", Value::Bool(true)).await,
        Ok(Value::Bool(true))
    );
}
