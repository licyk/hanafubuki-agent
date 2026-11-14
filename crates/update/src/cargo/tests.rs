use std::path::Path;

use super::*;

#[test]
fn cargo_is_only_looked_for_in_absolute_directories() {
    let directory = tempfile::tempdir().unwrap();
    let name = format!("cargo{}", std::env::consts::EXE_SUFFIX);
    std::fs::write(directory.path().join(&name), b"").unwrap();
    let relative = std::ffi::OsString::from(".");
    assert_eq!(find_cargo(&relative), None);
    let path = std::env::join_paths([Path::new("relative"), directory.path()]).unwrap();
    assert_eq!(find_cargo(&path), Some(directory.path().join(name)));
}

#[test]
fn without_cargo_on_path_the_command_is_named() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("hanafubuki-agent");
    std::fs::write(&executable, b"installed").unwrap();
    let installation = Installation::at(&executable).unwrap();
    let command = ["cargo", "install", "--locked", "hanafubuki-agent-cli"].map(String::from);
    let error = run_cargo_on(
        &installation,
        &command,
        false,
        Some(directory.path().as_os_str()),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "cannot find cargo on PATH; run `cargo install --locked hanafubuki-agent-cli` yourself. \
         Nothing was changed."
    );
}

#[cfg(unix)]
mod fake_cargo {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use super::*;

    struct Fixture {
        _root: tempfile::TempDir,
        bin: PathBuf,
        tools: PathBuf,
        record: PathBuf,
        installation: Installation,
    }

    fn executable(path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    impl Fixture {
        /// An installed executable reporting `installed`, and a `cargo`
        /// that records its arguments and directory, then exits with
        /// `status`, having replaced the executable with one reporting
        /// `installs` when given.
        fn new(installed: &str, installs: Option<&str>, status: u8) -> Self {
            let root = tempfile::tempdir().unwrap();
            let bin = root.path().join("bin");
            let tools = root.path().join("tools");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::create_dir_all(&tools).unwrap();
            let target = bin.join("hanafubuki-agent");
            executable(&target, &format!("echo 'hanafubuki-agent {installed}'"));
            let record = root.path().join("record");
            let install = installs
                .map(|version| {
                    format!(
                        "printf '#!/bin/sh\\necho \"hanafubuki-agent {version}\"\\n' > '{0}.new'; \
                         chmod 755 '{0}.new'; mv '{0}.new' '{0}'",
                        target.display()
                    )
                })
                .unwrap_or_default();
            executable(
                &tools.join("cargo"),
                &format!(
                    "printf '%s\\n' \"$@\" > '{record}'; pwd >> '{record}'\n{install}\nexit {status}",
                    record = record.display()
                ),
            );
            let installation = Installation::at(&target).unwrap();
            Self {
                _root: root,
                bin,
                tools,
                record,
                installation,
            }
        }

        fn run(&self, force: bool) -> Result<Version, UpdateError> {
            let command =
                ["cargo", "install", "--locked", "hanafubuki-agent-cli"].map(String::from);
            run_cargo_on(
                &self.installation,
                &command,
                force,
                Some(self.tools.as_os_str()),
            )
        }

        fn recorded(&self) -> Vec<String> {
            std::fs::read_to_string(&self.record)
                .unwrap()
                .lines()
                .map(str::to_string)
                .collect()
        }
    }

    #[test]
    fn cargo_runs_outside_any_project_and_the_new_version_is_read_back() {
        let fixture = Fixture::new("0.0.1", Some("99.0.0"), 0);
        assert_eq!(fixture.run(false).unwrap(), Version::new(99, 0, 0));
        let recorded = fixture.recorded();
        assert_eq!(
            recorded[..3],
            ["install", "--locked", "hanafubuki-agent-cli"]
        );
        let directory = PathBuf::from(&recorded[3]);
        assert!(!directory.exists(), "its directory is removed afterwards");
        assert!(!directory.starts_with(&fixture.bin));
        assert!(!directory.starts_with(env!("CARGO_MANIFEST_DIR")));
    }

    #[test]
    fn force_reinstalls_the_same_version() {
        let current = crate::current_version().to_string();
        let fixture = Fixture::new(&current, Some(&current), 0);
        assert_eq!(fixture.run(true).unwrap(), crate::current_version());
        assert_eq!(fixture.recorded()[3], "--force");
    }

    #[test]
    fn a_failing_cargo_leaves_the_version_unchanged() {
        let fixture = Fixture::new("0.0.1", None, 101);
        let error = fixture.run(false).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "cargo install --locked hanafubuki-agent-cli failed (exit status: 101); \
                 hanafubuki-agent {} is unchanged.",
                crate::current_version()
            )
        );
    }

    #[test]
    fn cargo_installing_elsewhere_is_noticed() {
        let fixture = Fixture::new("0.0.1", None, 0);
        let error = fixture.run(false).unwrap_err();
        assert_eq!(
            error,
            UpdateError::CargoElsewhere {
                executable: fixture.installation.executable().native_text().to_string(),
                reported: Some("hanafubuki-agent 0.0.1".to_string()),
                current: crate::current_version(),
            }
        );
        assert!(error.to_string().contains("CARGO_INSTALL_ROOT"));
    }
}
