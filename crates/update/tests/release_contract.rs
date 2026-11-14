//! The asset names are a contract with the release workflow: the updater of
//! one version reads the assets of the next, so `release.yml`'s targets and
//! packaging lines and `about.toml`'s targets must stay what
//! `RELEASE_TARGETS`, `asset_name` and `entry_path` say. This test fails on
//! any edit of those lines, on purpose (releasing, "The asset names are a
//! contract"). It reads the repository's files and asserts nothing when they
//! are absent (the crate as published).

use std::collections::BTreeSet;
use std::path::Path;

use hanafubuki_agent_update::{asset_name, entry_path, this_target, Version, RELEASE_TARGETS};

fn repository_file(relative: &str) -> Option<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative);
    std::fs::read_to_string(path).ok()
}

/// The `binaries` job's matrix: each `- target:` with the `os:` after it.
fn matrix(workflow: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = workflow.lines().map(str::trim).collect();
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let target = line.strip_prefix("- target: ")?;
            let os = lines.get(index + 1)?.strip_prefix("os: ")?;
            Some((target.to_string(), os.to_string()))
        })
        .collect()
}

/// `about.toml`'s `targets = [ … ]`, one quoted target per line.
fn about_targets(about: &str) -> BTreeSet<String> {
    about
        .lines()
        .map(str::trim)
        .skip_while(|line| *line != "targets = [")
        .skip(1)
        .take_while(|line| *line != "]")
        .map(|line| line.trim_end_matches(',').trim_matches('"').to_string())
        .collect()
}

#[test]
fn the_release_workflow_builds_and_names_the_assets_the_updater_reads() {
    let (Some(workflow), Some(about)) = (
        repository_file(".github/workflows/release.yml"),
        repository_file("about.toml"),
    ) else {
        return;
    };
    let expected: BTreeSet<String> = RELEASE_TARGETS.iter().map(|t| t.to_string()).collect();

    let matrix = matrix(&workflow);
    let built: BTreeSet<String> = matrix.iter().map(|(target, _)| target.clone()).collect();
    assert_eq!(built, expected, "release.yml's matrix");
    assert_eq!(matrix.len(), RELEASE_TARGETS.len(), "release.yml's matrix");
    assert_eq!(about_targets(&about), expected, "about.toml's targets");

    let lines: BTreeSet<&str> = workflow.lines().map(str::trim).collect();
    for line in [
        r#"name="hanafubuki-agent-$VERSION-$TARGET""#,
        "binary=hanafubuki-agent",
        r#"if [ "$RUNNER_OS" = Windows ]; then binary=hanafubuki-agent.exe; fi"#,
        r#"mkdir -p "dist/$name""#,
        r#"cp "target/$TARGET/release/$binary" LICENSE NOTICE THIRD-PARTY-LICENSES.txt "dist/$name/""#,
        "cd dist",
        r#"if [ "$RUNNER_OS" = Windows ]; then"#,
        r#"archive="$name.zip""#,
        r#"7z a -tzip "$archive" "$name" > /dev/null"#,
        "else",
        r#"archive="$name.tar.gz""#,
        r#"COPYFILE_DISABLE=1 tar czf "$archive" "$name""#,
    ] {
        assert!(
            lines.contains(line),
            "release.yml's packaging no longer has the line\n  {line}\n\
             which every released updater reads: see the releasing page's asset contract"
        );
    }
    // The release is published as a release: never a draft nor a pre-release,
    // which `releases/latest` would not answer.
    assert!(!workflow.contains("--draft"), "release.yml makes a draft");
    assert!(
        !workflow.contains("--prerelease"),
        "release.yml makes a pre-release"
    );

    // Windows runners make the `.zip` and the `.exe`; the others the
    // `.tar.gz` and the bare binary.
    let version = Version::new(1, 2, 3);
    for (target, os) in &matrix {
        let windows = os.starts_with("windows");
        let directory = format!("hanafubuki-agent-1.2.3-{target}");
        let (archive, binary) = if windows {
            ("zip", "hanafubuki-agent.exe")
        } else {
            ("tar.gz", "hanafubuki-agent")
        };
        assert_eq!(
            asset_name(&version, target),
            format!("{directory}.{archive}")
        );
        assert_eq!(
            entry_path(&version, target),
            format!("{directory}/{binary}")
        );
    }
}

#[test]
fn the_names_follow_the_plans_table() {
    let version = Version::new(0, 3, 0);
    assert_eq!(
        asset_name(&version, "x86_64-unknown-linux-gnu"),
        "hanafubuki-agent-0.3.0-x86_64-unknown-linux-gnu.tar.gz"
    );
    assert_eq!(
        entry_path(&version, "aarch64-apple-darwin"),
        "hanafubuki-agent-0.3.0-aarch64-apple-darwin/hanafubuki-agent"
    );
    assert_eq!(
        asset_name(&version, "x86_64-pc-windows-msvc"),
        "hanafubuki-agent-0.3.0-x86_64-pc-windows-msvc.zip"
    );
    assert_eq!(
        entry_path(&version, "x86_64-pc-windows-msvc"),
        "hanafubuki-agent-0.3.0-x86_64-pc-windows-msvc/hanafubuki-agent.exe"
    );
}

#[test]
fn this_build_is_a_release_target_or_none() {
    let target = this_target();
    assert!(target.is_none_or(|target| RELEASE_TARGETS.contains(&target)));
    // The three systems CI runs on build releases.
    if cfg!(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    )) {
        assert_eq!(target, Some("x86_64-unknown-linux-gnu"));
    }
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        assert_eq!(target, Some("aarch64-apple-darwin"));
    }
    if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        assert_eq!(target, Some("x86_64-apple-darwin"));
    }
    if cfg!(all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc"
    )) {
        assert_eq!(target, Some("x86_64-pc-windows-msvc"));
    }
    if cfg!(any(
        target_env = "musl",
        target_arch = "riscv64",
        target_os = "freebsd"
    )) {
        assert_eq!(target, None);
    }
}
