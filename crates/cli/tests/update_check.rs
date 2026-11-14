//! The update check runs only at the terminal interface's launch: `exec`
//! and a launch without a terminal never ask, even from an installation
//! the check would run for (a copy outside the build's profile directory,
//! with the test release URL set and the setting on).

mod support;

use std::path::PathBuf;
use std::time::Duration;

use hanafubuki_agent::test_support::{assistant_events, FakeServer};
use hanafubuki_agent_update::test_support::{FakeRelease, FakeResponse};
use support::Home;

/// The built binary, hard-linked outside `target/debug` so it is not a
/// build from a checkout; `None` where the file system cannot link it
/// (a copy of the debug binary is too large to make).
fn released_copy() -> Option<(tempfile::TempDir, PathBuf)> {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let source = PathBuf::from(env!("CARGO_BIN_EXE_hanafubuki-agent"));
    let copy = dir.path().join(source.file_name().unwrap());
    match std::fs::hard_link(&source, &copy) {
        Ok(()) => Some((dir, copy)),
        Err(error) => {
            eprintln!("skipped: the binary was not linked: {error}");
            None
        }
    }
}

/// A release newer than any build, served for the check.
fn newer_release() -> FakeRelease {
    let release = FakeRelease::start();
    let answer = release.release_json("v99.0.0", &[]);
    release.answer_latest(FakeResponse::json(&answer));
    release
}

#[test]
fn exec_and_a_launch_without_a_terminal_make_no_request() {
    let Some((_dir, binary)) = released_copy() else {
        return;
    };
    let release = newer_release();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let model = runtime.block_on(FakeServer::start(vec![(
        assistant_events("Done."),
        Duration::ZERO,
    )]));
    let home = Home::new().with_provider(&model.base_url);
    let run = |arguments: &[&str]| {
        let mut command = home.command();
        command
            .env("HANAFUBUKI_AGENT_TEST_RELEASE_URL", release.api_url())
            .env_remove("HANAFUBUKI_AGENT_NO_UPDATE_CHECK")
            .args(arguments);
        // The linked copy in place of the built binary.
        let mut linked = std::process::Command::new(&binary);
        linked
            .args(command.get_args())
            .current_dir(command.get_current_dir().unwrap())
            .stdin(std::process::Stdio::null());
        for (key, value) in command.get_envs() {
            match value {
                Some(value) => linked.env(key, value),
                None => linked.env_remove(key),
            };
        }
        linked.output().unwrap()
    };

    let exec = run(&["exec", "Say done."]);
    assert!(
        exec.status.success(),
        "{}",
        String::from_utf8_lossy(&exec.stderr)
    );
    let no_terminal = run(&[]);
    assert_eq!(no_terminal.status.code(), Some(2));
    let resumed = run(&["resume", "--last"]);
    assert_eq!(resumed.status.code(), Some(2));

    // A check would have asked by now; give a stray thread time anyway.
    std::thread::sleep(Duration::from_millis(500));
    assert!(release.requests().is_empty(), "{:?}", release.requests());
    assert!(!home.home.join("update-check.json").exists());
}
