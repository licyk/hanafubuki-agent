use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::{PathKey, ResolvedPath};

use super::{acl_plan, capability_sid, command_line, offline_environment};
use crate::roots::resolve_roots;

fn argv(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn the_command_line_quotes_as_the_standard_library_does() {
    assert_eq!(
        command_line(&argv(&[
            r"C:\Windows\System32\cmd.exe",
            "/D",
            "/S",
            "/C",
            "echo hi > out.txt"
        ]))
        .unwrap(),
        r#""C:\Windows\System32\cmd.exe" /D /S /C "echo hi > out.txt""#
    );
    assert_eq!(
        command_line(&argv(&["prog", "", r#"say "hi""#, r"C:\my dir\", r"a\\b"])).unwrap(),
        r#""prog" "" "say \"hi\"" "C:\my dir\\" a\\b"#
    );
    assert_eq!(
        command_line(&argv(&["prog", r#"a\"b"#])).unwrap(),
        r#""prog" a\\\"b"#
    );
}

#[test]
fn a_command_line_cannot_carry_is_refused() {
    assert!(command_line(&[]).is_err());
    assert!(command_line(&argv(&["pro\"g"])).is_err());
    assert!(command_line(&argv(&["prog", "a\0b"])).is_err());
}

/// A root's SID is part of the ACEs left on its directory, so it must not
/// change from one release to the next.
#[test]
fn a_capability_sid_is_derived_from_the_path_key_alone() {
    let work = capability_sid(&PathKey::of(Path::new("/work")));
    assert_eq!(work, "S-1-5-21-3402435716-1864852127-429106255-4212131304");
    assert_eq!(work, capability_sid(&PathKey::of(Path::new("/work"))));
    assert_ne!(work, capability_sid(&PathKey::of(Path::new("/work/sub"))));
    if cfg!(windows) {
        assert_eq!(
            capability_sid(&PathKey::of(Path::new(r"C:\Work"))),
            capability_sid(&PathKey::of(Path::new(r"c:\work")))
        );
    }
}

#[test]
fn each_root_is_granted_to_its_own_sid_and_read_only_paths_are_denied_to_the_roots_holding_them() {
    let workspace = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    for name in [".git", ".agents/skills"] {
        std::fs::create_dir_all(workspace.path().join(name)).unwrap();
    }
    let nested = workspace.path().join(".agents/skills");
    let roots = resolve_roots(&[
        workspace.path().to_path_buf(),
        nested.clone(),
        other.path().to_path_buf(),
        workspace.path().to_path_buf(),
    ]);
    let plan = acl_plan(&roots);

    let granted: Vec<&Path> = plan
        .grants
        .iter()
        .map(|grant| grant.root.native())
        .collect();
    let resolved = |path: &Path| ResolvedPath::resolve(path).unwrap();
    assert_eq!(
        granted,
        [
            resolved(workspace.path()).native(),
            resolved(&nested).native(),
            resolved(other.path()).native()
        ]
    );
    for grant in &plan.grants {
        assert_eq!(grant.sid, capability_sid(grant.root.key()));
    }

    let denied: Vec<(PathBuf, Vec<usize>)> = plan
        .denies
        .iter()
        .map(|deny| (deny.path.native().to_path_buf(), deny.grants.clone()))
        .collect();
    let workspace = resolved(workspace.path());
    assert_eq!(
        denied,
        [
            (workspace.native().join(".git"), vec![0]),
            (workspace.native().join(".agents"), vec![0, 1]),
        ]
    );
}

#[test]
fn read_only_has_nothing_to_grant() {
    let plan = acl_plan(&[]);
    assert!(plan.grants.is_empty());
    assert!(plan.denies.is_empty());
}

#[test]
fn the_offline_environment_keeps_what_the_command_already_has() {
    let added = offline_environment(|key| key == "HTTP_PROXY");
    assert!(!added.iter().any(|(key, _)| *key == "HTTP_PROXY"));
    assert!(added.contains(&("HTTPS_PROXY", "http://127.0.0.1:9")));
    assert!(added.contains(&("GIT_SSH_COMMAND", "cmd /c exit 1")));
    assert!(added.contains(&("SBX_NONET_ACTIVE", "1")));
    assert!(offline_environment(|_| true).is_empty());
}
