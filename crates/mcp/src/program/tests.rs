use std::ffi::OsString;
use std::fs;
use std::path::Path;

use hanafubuki_agent_foundation::paths::ResolvedPath;
use tempfile::tempdir;

use super::*;

fn native(path: &Path) -> std::path::PathBuf {
    ResolvedPath::resolve(path).unwrap().native().to_path_buf()
}

#[test]
fn accepts_absolute_and_path_programs_but_rejects_shell_text_and_relative_paths() {
    let root = tempdir().unwrap();
    let executable = root.path().join("runner");
    fs::write(&executable, "fixture").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let absolute = resolve_stdio_program(executable.to_str().unwrap(), None).unwrap();
    assert_eq!(absolute.resolved_path, native(&executable));
    let path = OsString::from(root.path());
    let named = resolve_stdio_program("runner", Some(&path)).unwrap();
    assert_eq!(named.resolved_path, absolute.resolved_path);
    for invalid in ["./runner", "runner --stdio", "runner|other", "runner;other"] {
        assert!(validate_stdio_program_shape(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn path_resolution_uses_the_first_executable_and_classifies_missing_or_non_executable() {
    let first = tempdir().unwrap();
    let second = tempdir().unwrap();
    for root in [first.path(), second.path()] {
        let executable = root.join("same-name");
        fs::write(&executable, root.to_string_lossy().as_bytes()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let path = std::env::join_paths([first.path(), second.path()]).unwrap();
    let resolved = resolve_stdio_program("same-name", Some(&path)).unwrap();
    assert!(resolved.resolved_path.starts_with(native(first.path())));
    assert_eq!(resolved.searched_path_entries, 2);
    assert!(matches!(
        resolve_stdio_program("missing", Some(&path)),
        Err(McpError::ResolveProgram(_))
    ));

    let blocked = first.path().join("blocked");
    fs::write(&blocked, "fixture").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            resolve_stdio_program("blocked", Some(&path)),
            Err(McpError::ResolveProgram(_))
        ));
    }
}
