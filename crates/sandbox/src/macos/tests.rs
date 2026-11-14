use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::ResolvedPath;

use super::{command_line, policy, resolve_roots, WritableRoot, SANDBOX_EXEC};

fn root(path: &str, read_only: &[&str]) -> WritableRoot {
    WritableRoot {
        root: path.to_string(),
        read_only: read_only.iter().map(|path| path.to_string()).collect(),
    }
}

fn resolved(path: &Path) -> String {
    ResolvedPath::resolve(path)
        .unwrap()
        .native_text()
        .to_string()
}

fn definitions(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[test]
fn read_only_denies_by_default_reads_everything_and_writes_nothing() {
    let (text, params) = policy(&[], None, false);
    assert!(text.contains("(version 1)"));
    assert!(text.contains("(deny default)"));
    assert!(text.contains("\n(allow file-read*)"));
    assert!(text.contains("(literal \"/dev/tty\")"));
    assert!(!text.contains("(allow file-write*"), "{text}");
    assert!(!text.contains("(allow network"), "{text}");
    assert!(text.ends_with("(deny system-fcntl (fcntl-command 80 110))"));
    assert!(params.is_empty());
}

#[test]
fn workspace_write_names_each_root_by_a_parameter_and_keeps_it_from_being_replaced() {
    let (text, params) = policy(
        &[root("/work", &[]), root("/private/tmp", &[])],
        None,
        false,
    );
    assert!(
        text.contains(
            "(allow file-write*\n\
             (subpath (param \"WRITABLE_ROOT_0\")) (subpath (param \"WRITABLE_ROOT_1\"))\n)"
        ),
        "{text}"
    );
    for key in ["WRITABLE_ROOT_0", "WRITABLE_ROOT_1"] {
        assert!(text.contains(&format!(
            "(deny file-write-unlink (require-all (literal (param \"{key}\")) (vnode-type DIRECTORY)))"
        )));
    }
    assert!(!text.contains("/work") && !text.contains("/private/tmp"));
    assert_eq!(
        params,
        definitions(&[
            ("WRITABLE_ROOT_0", "/work"),
            ("WRITABLE_ROOT_1", "/private/tmp")
        ])
    );
}

#[test]
fn what_stays_read_only_is_excluded_from_its_root_with_what_is_beneath_it() {
    let (text, params) = policy(
        &[root("/work", &["/work/.git", "/work/.agents"])],
        None,
        false,
    );
    assert!(
        text.contains(
            "(allow file-write*\n(require-all (subpath (param \"WRITABLE_ROOT_0\")) \
             (require-not (literal (param \"WRITABLE_ROOT_0_EXCLUDED_0\"))) \
             (require-not (subpath (param \"WRITABLE_ROOT_0_EXCLUDED_0\"))) \
             (require-not (literal (param \"WRITABLE_ROOT_0_EXCLUDED_1\"))) \
             (require-not (subpath (param \"WRITABLE_ROOT_0_EXCLUDED_1\"))) )\n)"
        ),
        "{text}"
    );
    assert_eq!(
        params,
        definitions(&[
            ("WRITABLE_ROOT_0_EXCLUDED_0", "/work/.git"),
            ("WRITABLE_ROOT_0_EXCLUDED_1", "/work/.agents"),
            ("WRITABLE_ROOT_0", "/work"),
        ])
    );
}

#[test]
fn the_network_is_allowed_only_when_asked_and_the_denies_come_last() {
    let roots = [root("/work", &[])];
    let (without, _) = policy(&roots, None, false);
    assert!(!without.contains("(allow network"), "{without}");
    assert!(!without.contains("com.apple.TrustEvaluationAgent"));

    let (with, _) = policy(&roots, None, true);
    let network = with
        .find("(allow network-outbound)\n(allow network-inbound)\n")
        .expect("the network rules");
    assert!(with.contains("(socket-domain AF_SYSTEM)"));
    assert!(with.contains("com.apple.TrustEvaluationAgent"));
    let writes = with.find("(allow file-write*").unwrap();
    let preferences = with.find("(allow user-preference-read)").unwrap();
    let xpc = with
        .find("(deny mach-lookup (xpc-service-name-prefix \"\"))")
        .unwrap();
    let fcntl = with.find("(deny system-fcntl").unwrap();
    assert!(writes < network && network < preferences && preferences < xpc && xpc < fcntl);
}

#[test]
fn the_command_line_runs_the_fixed_sandbox_exec_with_the_roots_as_definitions() {
    let workspace = tempfile::tempdir().unwrap();
    let command = vec!["/bin/sh".to_string(), "-c".to_string(), "true".to_string()];
    let line = command_line(
        &[workspace.path().to_path_buf()],
        None,
        true,
        command.clone(),
    );

    assert_eq!(line[0], SANDBOX_EXEC);
    assert_eq!(SANDBOX_EXEC, "/usr/bin/sandbox-exec");
    assert_eq!(line[1], "-p");
    let (expected_policy, _) = policy(
        &resolve_roots(&[workspace.path().to_path_buf()]),
        None,
        true,
    );
    assert_eq!(line[2], expected_policy);
    assert_eq!(
        line[3..],
        [
            format!("-DWRITABLE_ROOT_0={}", resolved(workspace.path())),
            "--".to_string(),
            "/bin/sh".to_string(),
            "-c".to_string(),
            "true".to_string(),
        ]
    );
}

#[test]
fn read_only_has_no_definitions() {
    let command = vec!["/bin/true".to_string()];
    let line = command_line(&[], None, false, command);
    assert_eq!(line[0], SANDBOX_EXEC);
    assert_eq!(line[3..], ["--".to_string(), "/bin/true".to_string()]);
}

#[test]
fn roots_are_resolved_once_and_a_missing_one_is_left_out() {
    let workspace = tempfile::tempdir().unwrap();
    let nested = workspace.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    let roots = resolve_roots(&[
        workspace.path().to_path_buf(),
        nested.join(".."),
        workspace.path().join("missing"),
        nested.clone(),
    ]);
    assert_eq!(
        roots,
        [
            root(&resolved(workspace.path()), &[]),
            root(&resolved(&nested), &[]),
        ]
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_root_is_its_target() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target");
    let link = directory.path().join("link");
    std::fs::create_dir(&target).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(
        resolve_roots(&[link, target.clone()]),
        [root(&resolved(&target), &[])]
    );
}

#[test]
fn the_repository_its_skills_and_the_project_configuration_stay_read_only_when_they_exist() {
    let workspace = tempfile::tempdir().unwrap();
    for name in [".git", ".agents", ".hanafubuki-agent"] {
        std::fs::create_dir(workspace.path().join(name)).unwrap();
    }
    let other = tempfile::tempdir().unwrap();
    std::fs::create_dir(other.path().join(".agents")).unwrap();

    let roots = resolve_roots(&[workspace.path().to_path_buf(), other.path().to_path_buf()]);
    let base = PathBuf::from(resolved(workspace.path()));
    let other_base = PathBuf::from(resolved(other.path()));
    let under = |base: &Path, name: &str| base.join(name).to_str().unwrap().to_string();
    assert_eq!(
        roots[0].read_only,
        [
            under(&base, ".git"),
            under(&base, ".agents"),
            under(&base, ".hanafubuki-agent")
        ]
    );
    assert_eq!(roots[1].read_only, [under(&other_base, ".agents")]);
}

#[test]
fn a_git_file_keeps_the_repository_it_points_to_read_only_too() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir(workspace.path().join("repository")).unwrap();
    std::fs::write(workspace.path().join(".git"), "gitdir: repository\n").unwrap();
    let roots = resolve_roots(&[workspace.path().to_path_buf()]);
    let base = PathBuf::from(resolved(workspace.path()));
    assert_eq!(
        roots[0].read_only,
        [
            resolved(&workspace.path().join("repository")),
            base.join(".git").to_str().unwrap().to_string()
        ]
    );

    std::fs::write(workspace.path().join(".git"), "gitdir: gone\n").unwrap();
    let roots = resolve_roots(&[workspace.path().to_path_buf()]);
    assert_eq!(
        roots[0].read_only,
        [base.join(".git").to_str().unwrap().to_string()]
    );
}

#[test]
fn a_plugin_reads_only_the_platform_defaults_its_readable_roots_and_its_writable_ones() {
    let readable = ["/package".to_string()];
    let (text, params) = policy(&[root("/data", &[])], Some(&readable), false);
    assert!(!text.contains("\n(allow file-read*)"), "{text}");
    assert!(
        text.contains(
            "(allow file-read*\n\
             (subpath (param \"READABLE_ROOT_0\")) (subpath (param \"WRITABLE_ROOT_0\"))\n)"
        ),
        "{text}"
    );
    assert!(text.contains("(subpath \"/usr/lib\")"), "{text}");
    assert!(!text.contains("(allow user-preference-read)"), "{text}");
    assert!(params.contains(&("READABLE_ROOT_0".to_string(), "/package".to_string())));
}

#[test]
fn overlapping_writable_roots_cannot_grant_protected_metadata_back() {
    let (text, _) = policy(
        &[root("/work", &["/work/.git"]), root("/work/.git", &[])],
        None,
        false,
    );
    let deny = text
        .find("(deny file-write* (literal (param \"WRITABLE_ROOT_0_EXCLUDED_0\"))")
        .unwrap();
    let allow = text.find("(allow file-write*").unwrap();
    assert!(deny > allow, "{text}");
}
