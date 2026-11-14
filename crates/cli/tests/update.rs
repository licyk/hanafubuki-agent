//! `update` through the real binary, against the fake release on loopback
//! (`HANAFUBUKI_AGENT_TEST_RELEASE_URL`). The binary is hard-linked out of
//! `target/` (copied where it cannot be), where it would be a build from a
//! checkout; on Unix a released binary's update installs a shell script
//! that prints the new version, and a cargo installation runs a fake
//! `cargo` on `PATH`. Debug builds only: a release build asks GitHub
//! whatever its environment says.
#![cfg(debug_assertions)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use hanafubuki_agent::AGENT_HOME_ENV;
use hanafubuki_agent_update::test_support::{FakeArchive, FakeAsset, FakeRelease};
use hanafubuki_agent_update::{current_version, release_notes_url, Version};

const TEST_RELEASE_URL: &str = "HANAFUBUKI_AGENT_TEST_RELEASE_URL";
const BINARY: &str = env!("CARGO_BIN_EXE_hanafubuki-agent");

/// The binary at `bin/hanafubuki-agent` in a directory of its own, a home
/// for it and a fake release.
struct Installed {
    root: tempfile::TempDir,
    executable: PathBuf,
    server: FakeRelease,
}

impl Installed {
    fn new() -> Self {
        // Beside `target/debug`, so a hard link is possible.
        let root = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
        let bin = root.path().join("bin dir").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(root.path().join("home")).unwrap();
        let executable = bin.join(format!("hanafubuki-agent{}", std::env::consts::EXE_SUFFIX));
        if std::fs::hard_link(BINARY, &executable).is_err() {
            std::fs::copy(BINARY, &executable).unwrap();
        }
        Self {
            root,
            executable,
            server: FakeRelease::start(),
        }
    }

    fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command
            .env(AGENT_HOME_ENV, self.root.path().join("home"))
            .env(TEST_RELEASE_URL, self.server.api_url())
            .env_remove("GITHUB_TOKEN")
            .current_dir(self.root.path())
            .stdin(Stdio::null());
        command
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.command(&self.executable)
            .args(arguments)
            .output()
            .unwrap()
    }

    /// Publishes `version`, its archive holding `executable`.
    fn publish(&self, version: &Version, executable: impl Into<Vec<u8>>) {
        let archive = FakeArchive::release(version, executable).bytes();
        self.server.publish(
            &format!("v{version}"),
            &[FakeAsset::new(&FakeArchive::name(version), archive)],
        );
    }

    fn native(&self) -> String {
        hanafubuki_agent_update::Installation::at(&self.executable)
            .unwrap()
            .executable()
            .native_text()
            .to_string()
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn expect(output: Output, code: i32) -> Output {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );
    output
}

/// A shell script that prints `hanafubuki-agent <version>`.
fn reporting(version: &Version) -> Vec<u8> {
    format!("#!/bin/sh\necho 'hanafubuki-agent {version}'\n").into_bytes()
}

#[test]
fn help_and_conflicting_flags_have_no_side_effects() {
    let installed = Installed::new();
    let help = expect(installed.run(&["update", "--help"]), 0);
    assert!(stdout(&help).contains("--check"));
    let conflict = expect(installed.run(&["update", "--check", "--force"]), 2);
    assert!(
        stderr(&conflict).contains("cannot be used with"),
        "{}",
        stderr(&conflict)
    );
    assert!(installed.server.requests().is_empty());
    assert_eq!(
        std::fs::read_dir(installed.root.path().join("home"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn a_debug_build_without_the_test_url_looks_nothing_up() {
    let installed = Installed::new();
    let output = expect(
        installed
            .command(&installed.executable)
            .env_remove(TEST_RELEASE_URL)
            .args(["update", "--check"])
            .output()
            .unwrap(),
        1,
    );
    assert_eq!(
        stderr(&output).lines().last(),
        Some(
            "error: a debug build looks up releases only at HANAFUBUKI_AGENT_TEST_RELEASE_URL, \
             which is not set; nothing was changed."
        )
    );
    assert!(installed.server.requests().is_empty());
}

#[test]
fn check_reports_a_newer_release_without_downloading() {
    let installed = Installed::new();
    let latest = Version::new(99, 0, 0);
    installed.publish(&latest, reporting(&latest));
    let output = expect(installed.run(&["update", "--check"]), 0);
    assert_eq!(
        stdout(&output),
        format!(
            "Current version  {}\nLatest release   99.0.0\nExecutable       {}\n\
             Installed as     a released binary\nRelease notes    {}\n\n\
             An update is available: run `hanafubuki-agent update` to install 99.0.0.\n",
            current_version(),
            installed.native(),
            release_notes_url(&latest)
        )
    );
    assert!(installed
        .server
        .requests()
        .iter()
        .all(|request| !request.path.starts_with("/download/")));
}

#[test]
fn check_and_update_with_nothing_to_install() {
    let installed = Installed::new();
    let current = current_version();
    installed.publish(&current, "unused");
    let check = expect(installed.run(&["update", "--check"]), 0);
    assert_eq!(
        stdout(&check).lines().last(),
        Some(format!("hanafubuki-agent {current} is the latest release.").as_str())
    );
    let update = expect(installed.run(&["update"]), 0);
    assert_eq!(
        stdout(&update),
        format!("hanafubuki-agent {current} is the latest release; nothing to do.\n")
    );

    let older = Version::new(0, 0, 1);
    installed.publish(&older, "unused");
    let update = expect(installed.run(&["update"]), 0);
    assert_eq!(
        stdout(&update),
        format!("This build ({current}) is newer than the latest release (0.0.1); nothing to install.\n")
    );
    let forced = expect(installed.run(&["update", "--force"]), 1);
    assert_eq!(
        stderr(&forced).lines().last(),
        Some(
            format!(
                "error: refusing to replace {current} with the older release 0.0.1; --force \
                 only reinstalls the same version."
            )
            .as_str()
        )
    );
}

#[test]
fn a_build_from_a_checkout_is_told_to_rebuild() {
    let installed = Installed::new();
    let latest = Version::new(99, 0, 0);
    installed.publish(&latest, "unused");
    let in_target = Path::new(BINARY);
    let check = expect(
        installed
            .command(in_target)
            .args(["update", "--check"])
            .output()
            .unwrap(),
        0,
    );
    assert!(stdout(&check).contains("Installed as     a build from a checkout\n"));
    let directory = hanafubuki_agent_update::Installation::at(in_target)
        .unwrap()
        .executable()
        .native()
        .parent()
        .unwrap()
        .display()
        .to_string();
    let sentence = format!(
        "This executable was built from a checkout ({directory}): update the checkout and \
         build it again.\n"
    );
    assert!(stdout(&check).ends_with(&sentence), "{}", stdout(&check));
    let update = expect(
        installed.command(in_target).arg("update").output().unwrap(),
        1,
    );
    assert_eq!(stdout(&update), sentence);
}

#[test]
fn a_release_without_a_digest_changes_nothing() {
    let installed = Installed::new();
    let latest = Version::new(99, 0, 0);
    let name = FakeArchive::name(&latest);
    installed.server.publish(
        "v99.0.0",
        &[FakeAsset::new(&name, "unused").without_digest()],
    );
    let before = std::fs::metadata(&installed.executable).unwrap().len();
    let output = expect(installed.run(&["update"]), 1);
    assert_eq!(
        stderr(&output).lines().last(),
        Some(
            format!(
                "error: release 99.0.0 gives no SHA-256 digest for {name}; nothing was changed."
            )
            .as_str()
        )
    );
    assert_eq!(
        std::fs::metadata(&installed.executable).unwrap().len(),
        before
    );
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn update_installs_the_newer_release_and_sends_the_token_only_to_the_api() {
        let installed = Installed::new();
        let latest = Version::new(99, 0, 0);
        installed.publish(&latest, reporting(&latest));
        let output = expect(
            installed
                .command(&installed.executable)
                .env("GITHUB_TOKEN", "ghp_secret")
                .arg("update")
                .output()
                .unwrap(),
            0,
        );
        let name = FakeArchive::name(&latest);
        let current = current_version();
        let lines: Vec<String> = stderr(&output).lines().map(str::to_string).collect();
        assert_eq!(
            lines,
            [
                format!(
                    "Updating hanafubuki-agent {current} → 99.0.0 at {}",
                    installed.native()
                ),
                format!("Downloading {name}: 0.0 of 0.0 MiB (0%)"),
                format!("Downloading {name}: 0.0 MiB: done"),
                "Verifying the SHA-256 digest: ok".to_string(),
                "Checking the new executable: hanafubuki-agent 99.0.0".to_string(),
                "Installing: done".to_string(),
            ]
        );
        assert_eq!(
            stdout(&output),
            format!(
                "Updated to 99.0.0. The next start of hanafubuki-agent runs it; running ones \
                 keep {current} until they restart.\n"
            )
        );
        let next = Command::new(&installed.executable)
            .arg("--version")
            .output()
            .unwrap();
        assert_eq!(stdout(&next), "hanafubuki-agent 99.0.0\n");
        for request in installed.server.requests() {
            let authorization = request.headers.get("authorization").map(String::as_str);
            if request.path.starts_with("/download/") {
                assert_eq!(authorization, None);
            } else {
                assert_eq!(authorization, Some("Bearer ghp_secret"));
            }
        }
    }

    #[test]
    fn a_digest_mismatch_ends_the_line_and_changes_nothing() {
        let installed = Installed::new();
        let latest = Version::new(99, 0, 0);
        let name = FakeArchive::name(&latest);
        let archive = FakeArchive::release(&latest, reporting(&latest)).bytes();
        installed.server.publish(
            "v99.0.0",
            &[FakeAsset::new(&name, archive).with_digest(&format!("sha256:{}", "0".repeat(64)))],
        );
        let output = expect(installed.run(&["update"]), 1);
        let stderr = stderr(&output);
        let lines: Vec<&str> = stderr.lines().collect();
        assert_eq!(
            lines[lines.len() - 2..],
            [
                "Verifying the SHA-256 digest: failed".to_string(),
                format!(
                    "error: the download of {name} does not match its SHA-256 digest; nothing \
                     was changed. Try again."
                ),
            ]
        );
        let version = Command::new(&installed.executable)
            .arg("--version")
            .output()
            .unwrap();
        assert_eq!(
            stdout(&version),
            format!("hanafubuki-agent {}\n", current_version())
        );
    }

    #[test]
    fn a_cargo_installation_is_updated_by_cargo() {
        let installed = Installed::new();
        let latest = Version::new(99, 0, 0);
        installed.publish(&latest, "unused");
        let root = installed
            .executable
            .parent()
            .and_then(Path::parent)
            .unwrap();
        std::fs::write(
            root.join(".crates2.json"),
            r#"{"installs":{"hanafubuki-agent-cli 0.2.0 (registry+https://github.com/rust-lang/crates.io-index)":{"bins":["hanafubuki-agent"]}}}"#,
        )
        .unwrap();
        let tools = installed.root.path().join("tools");
        std::fs::create_dir(&tools).unwrap();
        let record = installed.root.path().join("cargo-arguments");
        let target = installed.executable.display().to_string();
        let cargo = tools.join("cargo");
        std::fs::write(
            &cargo,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n\
                 printf '#!/bin/sh\\necho \"hanafubuki-agent 99.0.0\"\\n' > '{target}.new'\n\
                 chmod 755 '{target}.new'\nmv '{target}.new' '{target}'\n",
                record.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths(
            std::iter::once(tools).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();

        let check = expect(installed.run(&["update", "--check"]), 0);
        let root_text = hanafubuki_agent_update::Installation::at(&installed.executable)
            .unwrap()
            .executable()
            .native()
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .display()
            .to_string();
        assert!(stdout(&check).contains(&format!(
            "Installed as     cargo install (in {root_text})\n"
        )));
        let command = format!("cargo install --locked hanafubuki-agent-cli --root {root_text}");
        assert!(stdout(&check).ends_with(&format!(
            "An update is available: run `{command}` to install 99.0.0.\n"
        )));

        let output = expect(
            installed
                .command(&installed.executable)
                .env("PATH", path)
                .arg("update")
                .output()
                .unwrap(),
            0,
        );
        assert_eq!(stderr(&output), format!("Running {command}\n"));
        assert!(stdout(&output).starts_with("Updated to 99.0.0. "));
        assert_eq!(
            std::fs::read_to_string(record).unwrap(),
            format!("install\n--locked\nhanafubuki-agent-cli\n--root\n{root_text}\n")
        );
    }
}
