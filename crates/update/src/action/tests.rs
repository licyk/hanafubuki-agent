use std::path::PathBuf;

use super::*;
use crate::target::RELEASE_TARGETS;

const LINUX: Option<&str> = Some(RELEASE_TARGETS[0]);

fn cargo(root: &str, default_root: bool) -> InstallKind {
    InstallKind::Cargo {
        root: PathBuf::from(root),
        default_root,
    }
}

fn never_probed() -> bool {
    panic!("only a released binary's directory is probed")
}

#[test]
fn a_released_binary_is_replaced_where_it_can_be() {
    assert_eq!(
        action_for(&InstallKind::Release, LINUX, false, || true),
        UpdateAction::Replace
    );
    assert_eq!(
        action_for(&InstallKind::Release, LINUX, true, || true),
        UpdateAction::Replace
    );
    assert_eq!(
        action_for(&InstallKind::Release, LINUX, false, || false),
        UpdateAction::Manual(ManualReason::Unwritable)
    );
    assert_eq!(
        action_for(&InstallKind::Release, None, false, never_probed),
        UpdateAction::Manual(ManualReason::NoReleaseForPlatform)
    );
}

#[test]
fn cargo_runs_on_unix_with_its_root_only_off_the_default() {
    let command = |kind: &InstallKind| match action_for(kind, LINUX, false, never_probed) {
        UpdateAction::Cargo { command } => command,
        other => panic!("not cargo: {other:?}"),
    };
    assert_eq!(
        command(&cargo("/home/me/.cargo", true)),
        ["cargo", "install", "--locked", "hanafubuki-agent-cli"]
    );
    assert_eq!(
        command(&cargo("/opt/tools root", false)),
        [
            "cargo",
            "install",
            "--locked",
            "hanafubuki-agent-cli",
            "--root",
            "/opt/tools root"
        ]
    );
    // cargo is run from a cargo installation whatever the platform's release.
    assert!(matches!(
        action_for(&cargo("/r", true), None, false, never_probed),
        UpdateAction::Cargo { .. }
    ));
}

#[test]
fn cargo_on_windows_and_a_checkout_are_manual() {
    assert_eq!(
        action_for(&cargo("C:\\cargo", true), LINUX, true, never_probed),
        UpdateAction::Manual(ManualReason::CargoOnWindows)
    );
    for windows in [false, true] {
        assert_eq!(
            action_for(&InstallKind::Checkout, LINUX, windows, never_probed),
            UpdateAction::Manual(ManualReason::Checkout)
        );
    }
}

#[test]
fn an_installation_on_this_machine_has_the_action_of_its_kind() {
    let temp = tempfile::tempdir().unwrap();
    let executable = temp
        .path()
        .join(format!("hanafubuki-agent{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&executable, b"binary").unwrap();
    let installation = Installation::at(&executable).unwrap();
    let expected = if this_target().is_some() {
        UpdateAction::Replace
    } else {
        UpdateAction::Manual(ManualReason::NoReleaseForPlatform)
    };
    assert_eq!(installation.action(), expected);
}
