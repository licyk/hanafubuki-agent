//! The example plugins (`examples/plugins/`) built as their READMEs say and
//! checked with `plugins validate`, then started with `--run` on Linux,
//! where the sandbox's plugin profile was checked (skipped when the machine
//! has no sandbox), and the WASM one wherever its target is installed. The Rust one is built with the workspace's cargo; the
//! Python one needs an interpreter with a pykardia that speaks
//! kardia-guest/2 (PyPI's 0.1.0 does not), named by
//! `HANAFUBUKI_AGENT_TEST_PYTHON`, and is skipped without one. Unix only:
//! the manifests name Unix programs.

#![cfg(unix)]

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use support::Home;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A copy of the example `name` (its sources and its package, not a build).
fn copy_example(name: &str, into: &Path) -> PathBuf {
    let source = repository().join("examples/plugins").join(name);
    let target = into.join(name);
    copy(&source, &target);
    target
}

fn copy(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if name != "target" {
                copy(&path, &target.join(name));
            }
        } else {
            std::fs::copy(&path, target.join(name)).unwrap();
        }
    }
}

/// Validates `package`, then starts it on Linux, where the sandbox's
/// plugin profile was checked; guests never start without the sandbox, so
/// a machine without one skips the start.
fn validate(home: &Home, package: &Path) {
    let run = home
        .run(&["plugins", "validate", package.to_str().unwrap()])
        .expect(0);
    assert!(run.stdout().contains("No issues."), "{}", run.stdout());
    if !cfg!(target_os = "linux") {
        return;
    }
    let run = home.run(&["plugins", "validate", package.to_str().unwrap(), "--run"]);
    if run.code() != 0 && run.stdout().contains("its sandbox is unavailable") {
        eprintln!("not started: {}", run.stdout());
        return;
    }
    let run = run.expect(0);
    assert!(
        run.stdout().contains("guest:    running"),
        "{}",
        run.stdout()
    );
    let id = installed(home, package);
    home.run(&["plugins", "grant", &id]).expect(0);
    shown_running(home, &id, "running");
}

/// Installs `package` into `home`, returning its id.
fn installed(home: &Home, package: &Path) -> String {
    let run = home
        .run(&["plugins", "add", package.to_str().unwrap()])
        .expect(0);
    let added = run.stdout();
    let id = added
        .split('`')
        .nth(1)
        .unwrap_or_else(|| panic!("no id in {added}"));
    id.to_string()
}

/// `plugins info` of the installed `id` shows its code `state` once its
/// guest started in that command's own run of the Agent.
fn shown_running(home: &Home, id: &str, state: &str) {
    let run = home.run(&["plugins", "info", id]).expect(0);
    assert!(
        run.stdout().contains(&format!("code:        {state}")),
        "{}",
        run.stdout()
    );
}

#[test]
fn the_rust_example_builds_validates_and_runs() {
    let home = Home::new();
    let example = copy_example("hello-rust", &home.project);
    let target = repository().join("target/plugin-examples");
    let built = Command::new(env!("CARGO"))
        .args(["build", "--release", "--locked"])
        .current_dir(&example)
        .env("CARGO_TARGET_DIR", &target)
        .status()
        .unwrap();
    assert!(built.success(), "the example did not build");
    std::fs::copy(
        target.join("release/hello-rust"),
        example.join("package/hello-rust"),
    )
    .unwrap();
    validate(&home, &example.join("package"));
}

/// The WASM example needs no sandbox, so it starts on every machine that
/// has the target to build it (CI installs it).
#[test]
fn the_wasm_example_builds_validates_and_runs() {
    let sysroot = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .unwrap();
    let sysroot = PathBuf::from(String::from_utf8(sysroot.stdout).unwrap().trim());
    if !sysroot.join("lib/rustlib/wasm32-unknown-unknown").is_dir() {
        eprintln!("skipped: the wasm32-unknown-unknown target is not installed");
        return;
    }
    let home = Home::new();
    let example = copy_example("hello-wasm", &home.project);
    let target = repository().join("target/plugin-examples");
    let built = Command::new(env!("CARGO"))
        .args([
            "build",
            "--release",
            "--locked",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .current_dir(&example)
        .env("CARGO_TARGET_DIR", &target)
        .status()
        .unwrap();
    assert!(built.success(), "the example did not build");
    std::fs::copy(
        target.join("wasm32-unknown-unknown/release/hello_wasm.wasm"),
        example.join("package/hello-wasm.wasm"),
    )
    .unwrap();
    let package = example.join("package");
    let run = home
        .run(&["plugins", "validate", package.to_str().unwrap(), "--run"])
        .expect(0);
    assert!(
        run.stdout().contains("guest:    running (WebAssembly)"),
        "{}",
        run.stdout()
    );
    let id = installed(&home, &package);
    shown_running(&home, &id, "running (WebAssembly)");
}

#[test]
fn the_python_example_exports_validates_and_runs() {
    let Some(python) = std::env::var_os("HANAFUBUKI_AGENT_TEST_PYTHON") else {
        eprintln!("skipped: HANAFUBUKI_AGENT_TEST_PYTHON names no interpreter with pykardia");
        return;
    };
    let home = Home::new();
    let example = copy_example("hello-python", &home.project);
    let exported = Command::new(python)
        .args([
            "-m",
            "pykardia.guest.export",
            "process",
            "plugin.py",
            "Plugin",
            "package/hello-python.pyz",
        ])
        .current_dir(&example)
        .status()
        .unwrap();
    assert!(exported.success(), "the example was not exported");
    // The guest's metadata is exactly the manifest's name providing
    // `plugin`: a start whose guest declares anything else fails.
    validate(&home, &example.join("package"));
}
