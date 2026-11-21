//! An npm source: the marketplace entry says which package, and an install
//! runs `npm pack --ignore-scripts` through the Agent's processes, unpacks
//! the one archive it wrote and installs what the package holds.
//!
//! The registry itself is not reached: the `npm` the store finds on its
//! inherited `PATH` is a fixture that answers `pack` with a prepared archive
//! and records what it was asked. A host without `npm` skips these.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use flate2::write::GzEncoder;
use flate2::Compression;
use tar::Builder;

use hanafubuki_agent_packages::{load_marketplace, PackageSource};

use super::{fixture, write};

/// Writes a gzipped tar of `entries` at `path`, each under its own name.
fn tar_gz(path: &Path, entries: &[(&str, &str)]) {
    let file = fs::File::create(path).unwrap();
    let mut archive = Builder::new(GzEncoder::new(file, Compression::default()));
    for (name, contents) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, name, contents.as_bytes())
            .unwrap();
    }
    archive
        .into_inner()
        .unwrap()
        .finish()
        .unwrap()
        .flush()
        .unwrap();
}

fn npm_is_installed() -> bool {
    let program = if cfg!(windows) { "npm.cmd" } else { "npm" };
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// A directory holding an `npm` that answers `pack` with `archive`, first on
/// the `PATH` the store's processes inherit.
fn scripted_npm(directory: &Path, archive: &Path) -> PathBuf {
    let bin = directory.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let script = bin.join(if cfg!(windows) { "npm.cmd" } else { "npm" });
    let archive = archive.display();
    let body = if cfg!(windows) {
        format!(
            "@echo off\r\n\
             echo %* > \"%~dp0args.txt\"\r\n\
             :loop\r\n\
             if \"%~2\"==\"\" goto done\r\n\
             if /I \"%~2\"==\"--pack-destination\" (\r\n\
               copy /y \"{archive}\" \"%~3\\acme-plugin-1.2.0.tgz\" > nul\r\n\
               shift\r\n\
             )\r\n\
             shift\r\n\
             goto loop\r\n\
             :done\r\n\
             exit /b 0\r\n"
        )
    } else {
        format!(
            "#!/bin/sh\n\
             echo \"$@\" > \"$(dirname \"$0\")/args.txt\"\n\
             destination=\"\"\n\
             previous=\"\"\n\
             for argument in \"$@\"; do\n\
               if [ \"$previous\" = \"--pack-destination\" ]; then destination=\"$argument\"; fi\n\
               previous=\"$argument\"\n\
             done\n\
             cp '{archive}' \"$destination/acme-plugin-1.2.0.tgz\"\n"
        )
    };
    write(&script, &body);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

/// The archive `npm pack` would write for `@acme/plugin` 1.2.0.
fn packed_plugin(directory: &Path) -> PathBuf {
    let archive = directory.join("acme-plugin.tgz");
    tar_gz(
        &archive,
        &[
            (
                "package/package.json",
                r#"{"name":"@acme/plugin","version":"1.2.0"}"#,
            ),
            (
                "package/.codex-plugin/plugin.json",
                r#"{"name":"acme-plugin","version":"1.2.0","description":"An npm-published plugin","skills":"./skills"}"#,
            ),
            (
                "package/skills/acme-greet/SKILL.md",
                "---\nname: acme-greet\ndescription: Greets.\n---\n\nSay hello.\n",
            ),
        ],
    );
    archive
}

fn npm_marketplace(root: &Path, source: serde_json::Value) -> PathBuf {
    let path = root.join("market/.agents/plugins/marketplace.json");
    write(
        &path,
        &serde_json::json!({
            "name": "npm-market",
            "plugins": [{"name": "acme-plugin", "source": source}],
        })
        .to_string(),
    );
    path
}

/// Runs `body` with `bin` first on the `PATH` the store's processes inherit.
/// The tests of one binary share this process, so they take turns and the
/// `PATH` is put back afterwards.
fn with_npm_on_path<T>(bin: &Path, body: impl FnOnce() -> T) -> T {
    static TURNS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _turn = TURNS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    struct Restore(String);
    impl Drop for Restore {
        fn drop(&mut self) {
            std::env::set_var("PATH", &self.0);
        }
    }

    let separator = if cfg!(windows) { ';' } else { ':' };
    let _restore = Restore(std::env::var("PATH").unwrap_or_default());
    std::env::set_var(
        "PATH",
        format!("{}{separator}{}", bin.display(), _restore.0),
    );
    body()
}

#[test]
fn an_npm_source_is_packed_unpacked_and_installed() {
    if !npm_is_installed() {
        eprintln!("skipped: no npm on this host");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let bin = scripted_npm(directory.path(), &packed_plugin(directory.path()));
    let fixture = fixture();
    let marketplace = load_marketplace(&npm_marketplace(
        directory.path(),
        serde_json::json!({
            "source": "npm",
            "package": "@acme/plugin",
            "version": "1.2.0",
            "registry": "https://registry.example.com",
        }),
    ))
    .unwrap();

    let outcome = with_npm_on_path(&bin, || {
        fixture
            .store
            .install_from_marketplace(&marketplace, "acme-plugin")
            .unwrap()
    });

    assert_eq!(outcome.version, "1.2.0");
    assert!(outcome.root.join(".codex-plugin/plugin.json").is_file());
    let arguments = fs::read_to_string(directory.path().join("bin/args.txt")).unwrap();
    assert!(arguments.contains("--ignore-scripts"), "{arguments}");
    assert!(arguments.contains("--"), "{arguments}");
    assert!(arguments.contains("@acme/plugin@1.2.0"), "{arguments}");
    assert!(
        arguments.contains("--registry https://registry.example.com"),
        "{arguments}"
    );
    let record = fixture.store.read(&outcome.id).unwrap();
    assert!(matches!(
        record.record.and_then(|record| record.source),
        Some(PackageSource::Npm { .. })
    ));
    fixture.assert_no_process_left();
}

#[test]
fn an_npm_package_whose_name_is_not_the_requested_one_fails() {
    if !npm_is_installed() {
        eprintln!("skipped: no npm on this host");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let archive = directory.path().join("wrong.tgz");
    tar_gz(
        &archive,
        &[
            (
                "package/package.json",
                r#"{"name":"someone-else","version":"1.2.0"}"#,
            ),
            (
                "package/.codex-plugin/plugin.json",
                r#"{"name":"acme-plugin","version":"1.2.0"}"#,
            ),
        ],
    );
    let bin = scripted_npm(directory.path(), &archive);
    let fixture = fixture();
    let marketplace = load_marketplace(&npm_marketplace(
        directory.path(),
        serde_json::json!({"source": "npm", "package": "@acme/plugin"}),
    ))
    .unwrap();

    let error = with_npm_on_path(&bin, || {
        fixture
            .store
            .install_from_marketplace(&marketplace, "acme-plugin")
            .unwrap_err()
    });

    assert!(
        error
            .to_string()
            .contains("the npm package is named `someone-else` where `@acme/plugin` was asked for"),
        "{error}"
    );
    fixture.assert_no_process_left();
}
