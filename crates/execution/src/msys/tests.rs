use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::ResolvedPath;

use super::{add_posix_shell_environment, command_line_script, PosixShellSearch, MAX_SCRIPT_UNITS};

/// An executable file at `directory/name`, and its resolved form.
fn fake(directory: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(directory).unwrap();
    let file = directory.join(name);
    std::fs::write(&file, b"").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    resolved(&file)
}

fn resolved(path: &Path) -> PathBuf {
    ResolvedPath::resolve(path).unwrap().native().to_path_buf()
}

fn path_of(directories: &[&Path]) -> OsString {
    std::env::join_paths(directories).unwrap()
}

fn search(root: &Path, path: &[&Path]) -> PosixShellSearch {
    let system32 = root.join("Windows").join("System32");
    std::fs::create_dir_all(&system32).unwrap();
    PosixShellSearch {
        msys_path: None,
        path: path_of(path),
        wsl_directories: vec![resolved(&system32)],
        roots: vec![root.join("Git"), root.join("msys64")],
    }
}

#[test]
fn wsl_launchers_on_the_path_are_skipped() {
    let root = tempfile::tempdir().unwrap();
    let system32 = root.path().join("Windows").join("System32");
    fake(&system32, "bash.exe");
    let apps = root.path().join("Microsoft").join("WindowsApps");
    fake(&apps, "bash.exe");
    let tools = root.path().join("tools");
    let shell = fake(&tools, "bash.exe");
    let found = search(root.path(), &[&system32, &apps, &tools])
        .find()
        .unwrap();
    assert_eq!(found, shell);
}

#[test]
fn bash_comes_before_sh_at_each_step_and_the_steps_go_in_order() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");
    fake(&first, "sh.exe");
    let bash = fake(&second, "bash.exe");
    let mut lookup = search(root.path(), &[&first, &second]);
    assert_eq!(lookup.find().unwrap(), bash);

    let msys = root.path().join("msys");
    let msys_sh = fake(&msys, "sh.exe");
    lookup.msys_path = Some(path_of(&[&msys]));
    assert_eq!(lookup.find().unwrap(), msys_sh);

    let installed = fake(
        &root.path().join("msys64").join("usr").join("bin"),
        "bash.exe",
    );
    let lookup = search(root.path(), &[&root.path().join("empty")]);
    assert_eq!(lookup.find().unwrap(), installed);
    let git = fake(&root.path().join("Git").join("usr").join("bin"), "sh.exe");
    assert_eq!(lookup.find().unwrap(), installed, "bash before sh");
    std::fs::remove_file(&installed).unwrap();
    assert_eq!(lookup.find().unwrap(), git);
}

#[test]
fn git_for_windows_launcher_is_replaced_by_its_shell() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("Git").join("bin");
    let launcher = fake(&bin, "bash.exe");
    let shell = fake(&root.path().join("Git").join("usr").join("bin"), "bash.exe");
    let lookup = search(root.path(), &[&bin]);
    assert_eq!(lookup.find().unwrap(), shell);

    std::fs::remove_file(&shell).unwrap();
    assert_eq!(lookup.find().unwrap(), launcher, "a bin without usr\\bin");
}

#[test]
fn a_missing_shell_says_what_was_searched_and_what_to_use_instead() {
    let root = tempfile::tempdir().unwrap();
    let mut lookup = search(root.path(), &[]);
    let message = lookup.find().unwrap_err().to_string();
    for expected in [
        "bash.exe",
        "sh.exe",
        "PATH",
        "System32",
        "WindowsApps",
        "\"power_shell\"",
        &root
            .path()
            .join("Git")
            .join("usr")
            .join("bin")
            .display()
            .to_string(),
        &root
            .path()
            .join("msys64")
            .join("usr")
            .join("bin")
            .display()
            .to_string(),
    ] {
        assert!(message.contains(expected), "{expected:?} in {message}");
    }
    assert!(!message.contains("Git Bash shell"), "{message}");
    lookup.msys_path = Some(OsString::new());
    let message = lookup.find().unwrap_err().to_string();
    assert!(
        message.contains("Git Bash shell the Agent runs in"),
        "{message}"
    );
}

#[test]
fn the_shell_directory_leads_the_path_and_the_request_wins() {
    let root = tempfile::tempdir().unwrap();
    let shell = fake(&root.path().join("usr").join("bin"), "bash.exe");
    let directory = shell.parent().unwrap().to_str().unwrap().to_string();
    let mut environment = BTreeMap::from([
        ("Path".to_string(), r"C:\tools".to_string()),
        ("CHERE_INVOKING".to_string(), "".to_string()),
    ]);
    add_posix_shell_environment(&mut environment, &shell, Some("UCRT64".to_string()));
    assert_eq!(
        environment,
        BTreeMap::from([
            ("Path".to_string(), format!(r"{directory};C:\tools")),
            ("CHERE_INVOKING".to_string(), "".to_string()),
            ("MSYS2_PATH_TYPE".to_string(), "inherit".to_string()),
            ("MSYSTEM".to_string(), "UCRT64".to_string()),
        ])
    );

    let mut environment = BTreeMap::new();
    add_posix_shell_environment(&mut environment, &shell, None);
    assert_eq!(environment["PATH"], directory);
    assert_eq!(environment["CHERE_INVOKING"], "1");
    assert!(!environment.contains_key("MSYSTEM"));
}

#[test]
fn a_script_is_spelled_for_the_msys_command_line() {
    for (script, spelled) in [
        // What a review lets run is spelled as it is.
        ("git --no-pager log -n 3", "git --no-pager log -n 3"),
        ("pwd", " pwd"),
        ("ls\npwd", " ls\npwd"),
        (r"printf 'a\\b' x", r"printf 'a\\\\b' x"),
        (r"echo a\b", r"echo a\\b"),
        (r#"echo "a\"b" x"#, r#"echo "a\"b" x"#),
        (r#"echo \\"x" y"#, r#"echo \\"x" y"#),
        (r"echo end\\", r"echo end\\"),
        ("", " "),
    ] {
        assert_eq!(command_line_script(script).unwrap(), spelled, "{script:?}");
    }
}

#[test]
fn a_script_the_msys_runtime_would_cut_short_is_refused() {
    let longest = format!("echo {}", "中".repeat(MAX_SCRIPT_UNITS - 5));
    assert!(command_line_script(&longest).is_ok());
    let message = command_line_script(&format!("{longest}x"))
        .unwrap_err()
        .to_string();
    assert!(message.contains("\"power_shell\""), "{message}");
    // A script that gets a space in front has one unit less.
    assert!(command_line_script(&"x".repeat(MAX_SCRIPT_UNITS - 1)).is_ok());
    assert!(command_line_script(&"x".repeat(MAX_SCRIPT_UNITS)).is_err());
}
