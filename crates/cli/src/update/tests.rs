use super::*;

fn installation() -> (tempfile::TempDir, Installation) {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("hanafubuki-agent");
    std::fs::write(&executable, b"installed").unwrap();
    let installation = Installation::at(&executable).unwrap();
    (directory, installation)
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
