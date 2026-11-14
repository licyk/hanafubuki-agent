use std::path::Path;

use serde_json::json;

use super::*;

const EXE: &str = std::env::consts::EXE_SUFFIX;

fn binary_name() -> String {
    format!("hanafubuki-agent{EXE}")
}

/// An executable at `directory/hanafubuki-agent[.exe]`.
fn executable_in(directory: &Path) -> PathBuf {
    fs::create_dir_all(directory).unwrap();
    let path = directory.join(binary_name());
    fs::write(&path, b"binary").unwrap();
    path
}

fn resolved(path: &Path) -> ResolvedPath {
    ResolvedPath::resolve(path).unwrap()
}

/// A cargo root at `root` that installed `bins` of the CLI's crate from
/// `source`.
fn crates2(root: &Path, source: &str, bins: &[String]) {
    let record = json!({
        "installs": {
            format!("{CLI_CRATE} 0.2.0 ({source})"): {
                "version_req": null,
                "bins": bins,
                "features": [],
                "all_features": false,
                "no_default_features": false,
                "profile": "release",
                "target": "x86_64-unknown-linux-gnu",
                "rustc": "rustc 1.99.0",
                "future": true,
            },
            "ripgrep 14.1.0 (registry+https://github.com/rust-lang/crates.io-index)": {
                "bins": ["rg"],
            },
        },
    });
    fs::write(root.join(".crates2.json"), record.to_string()).unwrap();
}

#[test]
fn a_cargo_profile_directory_is_a_checkout() {
    let temp = tempfile::tempdir().unwrap();
    for profile in ["target/debug", "target/x86_64-pc-windows-msvc/release"] {
        let directory = temp.path().join(profile);
        fs::create_dir_all(directory.join("deps")).unwrap();
        fs::create_dir_all(directory.join(".fingerprint")).unwrap();
        let executable = executable_in(&directory);
        assert_eq!(
            detect_kind(&resolved(&executable), None),
            InstallKind::Checkout,
            "{profile}"
        );
    }
    // Only one of the two is not a profile directory.
    let directory = temp.path().join("half");
    fs::create_dir_all(directory.join("deps")).unwrap();
    let executable = executable_in(&directory);
    assert_eq!(
        detect_kind(&resolved(&executable), None),
        InstallKind::Release
    );
}

#[test]
fn a_cargo_install_from_a_registry_is_cargo_with_its_root() {
    let temp = tempfile::tempdir().unwrap();
    for source in [
        "registry+https://github.com/rust-lang/crates.io-index",
        "sparse+https://index.crates.io/",
    ] {
        let root = temp.path().join(source.split('+').next().unwrap());
        let executable = executable_in(&root.join("bin"));
        crates2(&root, source, &[binary_name()]);
        let root = resolved(&root);
        assert_eq!(
            detect_kind(&resolved(&executable), None),
            InstallKind::Cargo {
                root: root.native().to_path_buf(),
                default_root: false,
            }
        );
        assert_eq!(
            detect_kind(&resolved(&executable), Some(&root)),
            InstallKind::Cargo {
                root: root.native().to_path_buf(),
                default_root: true,
            }
        );
    }
}

#[test]
fn a_cargo_install_from_a_path_or_git_is_a_checkout() {
    let temp = tempfile::tempdir().unwrap();
    for source in [
        "path+file:///home/me/src/hanafubuki-agent/crates/cli",
        "git+https://github.com/licyk/hanafubuki-agent#c44a6c8",
    ] {
        let root = temp.path().join(source.split('+').next().unwrap());
        let executable = executable_in(&root.join("bin"));
        crates2(&root, source, &[binary_name()]);
        assert_eq!(
            detect_kind(&resolved(&executable), None),
            InstallKind::Checkout,
            "{source}"
        );
    }
}

#[test]
fn a_binary_cargo_does_not_list_is_a_release() {
    let temp = tempfile::tempdir().unwrap();
    let cases: [(&str, Option<String>); 7] = [
        // cargo installed another binary of the crate, not this one.
        (
            "other-bin",
            Some(
                json!({ "installs": { format!("{CLI_CRATE} 0.2.0 (registry+https://x)"): { "bins": ["other"] } } })
                    .to_string(),
            ),
        ),
        // Another crate lists a binary of the same name.
        (
            "other-crate",
            Some(
                json!({ "installs": { "imposter 1.0.0 (registry+https://x)": { "bins": [binary_name()] } } })
                    .to_string(),
            ),
        ),
        ("unknown-source", Some(
            json!({ "installs": { format!("{CLI_CRATE} 0.2.0 (local+somewhere)"): { "bins": [binary_name()] } } })
                .to_string(),
        )),
        ("not-json", Some("not json".to_string())),
        ("not-an-object", Some("[1, 2]".to_string())),
        ("bins-not-a-list", Some(
            json!({ "installs": { format!("{CLI_CRATE} 0.2.0 (registry+https://x)"): { "bins": "hanafubuki-agent" } } })
                .to_string(),
        )),
        // No record at all: a released binary someone put in a `bin`.
        ("no-record", None),
    ];
    for (case, record) in cases {
        let root = temp.path().join(case);
        let executable = executable_in(&root.join("bin"));
        if let Some(record) = record {
            fs::write(root.join(".crates2.json"), record).unwrap();
        }
        assert_eq!(
            detect_kind(&resolved(&executable), None),
            InstallKind::Release,
            "{case}"
        );
    }
}

#[test]
fn a_record_beside_a_directory_that_is_not_bin_is_not_read() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let executable = executable_in(&root.join("tools"));
    crates2(&root, "registry+https://x", &[binary_name()]);
    assert_eq!(
        detect_kind(&resolved(&executable), None),
        InstallKind::Release
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_record_is_not_cargo() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let executable = executable_in(&root.join("bin"));
    crates2(&root, "registry+https://x", &[binary_name()]);
    let record = root.join(".crates2.json");
    fs::set_permissions(&record, fs::Permissions::from_mode(0o000)).unwrap();
    let readable = fs::read(&record).is_ok();
    let kind = detect_kind(&resolved(&executable), None);
    fs::set_permissions(&record, fs::Permissions::from_mode(0o644)).unwrap();
    if readable {
        // Root reads it whatever its mode: nothing to test here.
        return;
    }
    assert_eq!(kind, InstallKind::Release);
}

#[test]
fn a_missing_executable_cannot_be_found() {
    let temp = tempfile::tempdir().unwrap();
    // How Linux names a running executable that was replaced.
    let replaced = temp.path().join("hanafubuki-agent (deleted)");
    assert_eq!(
        Installation::at(&replaced).unwrap_err(),
        UpdateError::ExecutableNotFound
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_resolves_to_its_target() {
    let temp = tempfile::tempdir().unwrap();
    let executable = executable_in(&temp.path().join("versions/0.2.0"));
    let link = temp.path().join("hanafubuki-agent");
    std::os::unix::fs::symlink(&executable, &link).unwrap();
    let installation = Installation::at(&link).unwrap();
    assert_eq!(installation.executable(), &resolved(&executable));
    assert_eq!(installation.kind(), &InstallKind::Release);
}

#[test]
fn a_writable_directory_is_probed_and_left_clean() {
    let temp = tempfile::tempdir().unwrap();
    let executable = executable_in(temp.path());
    let installation = Installation::at(&executable).unwrap();
    assert!(installation.writable());
    let names: Vec<_> = fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, [std::ffi::OsString::from(binary_name())]);
}

#[cfg(unix)]
#[test]
fn a_read_only_directory_is_not_writable() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("bin");
    let executable = executable_in(&directory);
    let installation = Installation::at(&executable).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o555)).unwrap();
    let root = fs::write(directory.join("probe"), b"").is_ok();
    let writable = installation.writable();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    if root {
        // Root writes it whatever its mode: nothing to test here.
        return;
    }
    assert!(!writable);
}
