use std::fs;

use hanafubuki_agent_update::test_support::{FakeRelease, FakeResponse};
use hanafubuki_agent_update::ManualReason;

use super::super::cache::UPDATE_CHECK_FILE;
use super::super::check::check;
use super::*;

const NOW: u64 = 1_763_107_200_000;

fn version(text: &str) -> Version {
    text.parse().unwrap()
}

fn cargo(command: &[&str]) -> UpdateAction {
    UpdateAction::Cargo {
        command: command.iter().map(|word| word.to_string()).collect(),
    }
}

fn first_option(action: &UpdateAction) -> Option<String> {
    offer(&version("0.3.0"), None, action, &version("0.2.0")).map(|prompt| prompt.update_now)
}

#[test]
fn the_first_option_says_what_updating_does_here() {
    assert_eq!(
        first_option(&UpdateAction::Replace).as_deref(),
        Some("Update now (downloads 0.3.0 and replaces this executable)")
    );
    let default_root = ["cargo", "install", "--locked", "hanafubuki-agent-cli"];
    assert_eq!(
        first_option(&cargo(&default_root)).as_deref(),
        Some("Update now (runs `cargo install --locked hanafubuki-agent-cli`)")
    );
    let elsewhere = [&default_root[..], &["--root", "/opt/tools"]].concat();
    assert_eq!(
        first_option(&cargo(&elsewhere)).as_deref(),
        Some("Update now (runs `cargo install --locked hanafubuki-agent-cli --root /opt/tools`)")
    );
}

/// The manual cases (a checkout, an unwritable directory, no release for
/// this platform, cargo on Windows) are not prompted: the notice names
/// their step.
#[test]
fn a_manual_step_is_not_prompted() {
    for reason in [
        ManualReason::Checkout,
        ManualReason::Unwritable,
        ManualReason::NoReleaseForPlatform,
        ManualReason::CargoOnWindows,
    ] {
        assert_eq!(
            first_option(&UpdateAction::Manual(reason)),
            None,
            "{reason:?}"
        );
    }
}

#[test]
fn only_the_dismissed_version_itself_is_not_prompted() {
    let current = version("0.2.0");
    let dismissed = version("0.3.0");
    let action = UpdateAction::Replace;
    assert_eq!(
        offer(&version("0.3.0"), Some(&dismissed), &action, &current),
        None
    );
    let newer = offer(&version("0.3.1"), Some(&dismissed), &action, &current).unwrap();
    assert_eq!(
        (newer.current.as_str(), newer.latest.as_str()),
        ("0.2.0", "0.3.1")
    );
    assert_eq!(
        newer.release_notes,
        "https://github.com/licyk/hanafubuki-agent/releases/tag/v0.3.1"
    );
}

/// The dismissal and another instance's check write the same file: each
/// keeps what the other wrote, whichever comes first.
#[test]
fn a_dismissal_and_a_concurrent_check_keep_each_other_s_fields() {
    let release = FakeRelease::start();
    let answer = release.release_json("v0.3.1", &[]);
    release.answer_latest(FakeResponse::json(&answer));
    let dir = tempfile::tempdir().unwrap();
    let client = release.client(None);

    // The dismissal, then a check another instance makes.
    dismiss(dir.path(), &version("0.3.0"));
    check(&client, dir.path(), &version("0.2.0"), NOW);
    let cache = UpdateCache::load(dir.path());
    assert_eq!(cache.dismissed_version, Some(version("0.3.0")));
    assert_eq!(cache.latest_version, Some(version("0.3.1")));
    assert_eq!(cache.checked_at_ms, Some(NOW));

    // A check written while the prompt was up, then the dismissal.
    let dir = tempfile::tempdir().unwrap();
    check(&client, dir.path(), &version("0.2.0"), NOW);
    dismiss(dir.path(), &version("0.3.0"));
    let cache = UpdateCache::load(dir.path());
    assert_eq!(cache.dismissed_version, Some(version("0.3.0")));
    assert_eq!(cache.latest_version, Some(version("0.3.1")));
    assert_eq!(cache.checked_at_ms, Some(NOW));
}

/// A cache that cannot be read is not written; the prompt only comes
/// again.
#[test]
fn a_dismissal_that_cannot_be_written_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(UPDATE_CHECK_FILE)).unwrap();
    dismiss(dir.path(), &version("0.3.0"));
    assert!(dir.path().join(UPDATE_CHECK_FILE).is_dir());
}
