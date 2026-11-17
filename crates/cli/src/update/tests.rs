use super::*;
use hanafubuki_agent_update::test_support::{FakeRelease, FakeResponse};

fn installation() -> (tempfile::TempDir, Installation) {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("hanafubuki-agent");
    std::fs::write(&executable, b"installed").unwrap();
    let installation = Installation::at(&executable).unwrap();
    (directory, installation)
}

#[test]
fn a_manual_check_refreshes_the_launch_cache_and_a_failure_keeps_it() {
    let (directory, installation) = installation();
    let cache = directory.path().join("update-check.json");
    std::fs::write(
        &cache,
        r#"{"latestVersion":"0.1.0","checkedAtMs":1,"dismissedVersion":"0.2.0","future":true}"#,
    )
    .unwrap();
    let server = FakeRelease::start();
    server.answer_latest(FakeResponse::json(&server.release_json("v9999.0.0", &[])));
    let client = server.client(None);
    assert_eq!(
        report(&installation, &client, Some(directory.path()), None).unwrap(),
        SUCCESS
    );
    let written = std::fs::read_to_string(&cache).unwrap();
    let state: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(state["latestVersion"], "9999.0.0");
    assert!(state["checkedAtMs"].as_u64().unwrap() > 1);
    assert_eq!(state["dismissedVersion"], "0.2.0");
    assert_eq!(state["future"], true);
    assert_eq!(server.requests().len(), 1);

    server.answer_latest(FakeResponse::status(503));
    assert!(report(&installation, &client, Some(directory.path()), None).is_err());
    assert_eq!(std::fs::read_to_string(&cache).unwrap(), written);
}

#[test]
fn the_manual_cases_say_what_to_do_instead() {
    let (_directory, installation) = installation();
    let shown = installation
        .executable()
        .native()
        .parent()
        .unwrap()
        .display()
        .to_string();
    let latest = Version::new(0, 3, 0);
    assert_eq!(
        manual(&installation, ManualReason::Unwritable, &latest),
        format!(
            "An update is available, but {shown} is not writable: install 0.3.0 by hand \
             (https://licyk.github.io/hanafubuki-agent/docs/installation/), or run the update \
             as a user who can write there."
        )
    );
    assert_eq!(
        manual(&installation, ManualReason::NoReleaseForPlatform, &latest),
        "An update is available, but there is no released binary for this platform: install \
         0.3.0 the way you installed this one."
    );
    assert_eq!(
        installed_as(
            &installation,
            &UpdateAction::Manual(ManualReason::Unwritable)
        ),
        "a released binary (its directory is not writable)"
    );
}

#[test]
fn cargo_is_named_the_same_on_every_system() {
    let command = InstallKind::Cargo {
        root: "/opt/tools".into(),
        default_root: false,
    }
    .cargo_command()
    .unwrap();
    assert_eq!(
        run_to_install(&command, &Version::new(0, 3, 0)),
        "An update is available: run `cargo install --locked hanafubuki-agent-cli --root \
         /opt/tools` to install 0.3.0."
    );
}
