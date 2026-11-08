use std::fs;

use super::*;

/// A user instruction directory that holds no instructions.
fn no_user_instructions() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn parent_child_override_and_lossy_content_are_discovered_in_order() {
    let root = tempfile::tempdir().unwrap();
    let nested = root.path().join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(root.path().join("AGENTS.md"), "parent").unwrap();
    fs::write(nested.join("AGENTS.md"), "ignored").unwrap();
    fs::write(nested.join("AGENTS.override.md"), b"child\xffrule").unwrap();
    let roots = super::super::agent_workspace_roots_from([root.path().to_path_buf()]).unwrap();

    let discovered =
        discover_workspace_instructions(no_user_instructions().path(), &roots, Some(&nested));
    let content = discovered.content.unwrap();
    assert!(content.find("parent").unwrap() < content.find("child").unwrap());
    assert!(content.contains("child�rule"));
    assert!(!content.contains("ignored"));
    assert!(discovered
        .warnings
        .iter()
        .any(|warning| warning.contains("lossy UTF-8")));
}

#[test]
fn multiple_roots_receive_a_fair_share_and_utf8_head_tail_truncation() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    fs::write(
        first.path().join("AGENTS.md"),
        format!("HEAD{}TAIL", "界".repeat(512)),
    )
    .unwrap();
    fs::write(second.path().join("AGENTS.md"), "SECOND").unwrap();
    let roots = super::super::agent_workspace_roots_from([
        first.path().to_path_buf(),
        second.path().to_path_buf(),
    ])
    .unwrap();

    // Each header names its root twice, so the budget leaves room for a long
    // temporary path (macOS's is over 60 bytes) and still truncates.
    let discovered = discover_workspace_instructions_with_budget(
        no_user_instructions().path(),
        &roots,
        None,
        1024,
        1024,
    );
    let content = discovered.content.unwrap();
    assert!(content.contains("HEAD"));
    assert!(content.contains("workspace instructions omitted"));
    assert!(content.contains("TAIL"));
    assert!(content.contains("SECOND"));
    assert!(content.is_char_boundary(content.len()));
}

#[test]
fn a_budget_too_small_for_the_marker_keeps_a_utf8_head_within_it() {
    assert_eq!(truncate_head_tail(&"界".repeat(64), 8), "界界");
}

#[test]
fn an_unavailable_root_warns_without_blocking_another_root() {
    let available = tempfile::tempdir().unwrap();
    fs::write(available.path().join("AGENTS.md"), "available rule").unwrap();
    let missing = available.path().join("missing-volume");
    let roots = super::super::agent_workspace_roots_from([missing, available.path().to_path_buf()])
        .unwrap();

    let discovered = discover_workspace_instructions(no_user_instructions().path(), &roots, None);
    assert!(discovered.content.unwrap().contains("available rule"));
    assert!(discovered
        .warnings
        .iter()
        .any(|warning| warning.contains("workspace root unavailable")));
}

#[test]
fn user_instructions_come_before_the_project_within_the_same_budget() {
    let user = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    fs::write(user.path().join("AGENTS.md"), "USER".repeat(256)).unwrap();
    fs::write(root.path().join("AGENTS.md"), "PROJECT").unwrap();
    let roots = super::super::agent_workspace_roots_from([root.path().to_path_buf()]).unwrap();

    let discovered =
        discover_workspace_instructions_with_budget(user.path(), &roots, None, 1024, 1024);
    let content = discovered.content.unwrap();
    assert!(content.len() <= 1024);
    assert!(content.find("USER").unwrap() < content.find("PROJECT").unwrap());
    assert!(content.contains("workspace instructions omitted"));

    let missing = user.path().join("missing");
    let discovered = discover_workspace_instructions(&missing, &roots, None);
    let content = discovered.content.unwrap();
    assert!(!content.contains("USER"));
    assert!(content.contains("PROJECT"));
    assert!(discovered.warnings.is_empty());
}

#[cfg(windows)]
#[test]
fn windows_verbatim_case_and_cross_volume_roots_keep_one_path_authority() {
    use std::path::{Component, Prefix};

    fn drive(path: &Path) -> Option<u8> {
        match path.components().next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                    Some(drive.to_ascii_lowercase())
                }
                _ => None,
            },
            _ => None,
        }
    }

    let mut candidates = vec![std::env::temp_dir(), std::env::current_dir().unwrap()];
    for variable in [
        "RUNNER_TEMP",
        "GITHUB_WORKSPACE",
        "USERPROFILE",
        "PUBLIC",
        "ProgramData",
    ] {
        if let Some(path) = std::env::var_os(variable) {
            candidates.push(PathBuf::from(path));
        }
    }
    let mut volumes = Vec::<(u8, tempfile::TempDir)>::new();
    for candidate in candidates {
        let Ok(base) = ResolvedPath::resolve(&candidate) else {
            continue;
        };
        let Some(volume) = drive(base.native()) else {
            continue;
        };
        if volumes.iter().any(|(existing, _)| *existing == volume) {
            continue;
        }
        if let Ok(directory) = tempfile::Builder::new()
            .prefix("hanafubuki-agent-windows-path-contract-")
            .tempdir_in(base.system())
        {
            volumes.push((volume, directory));
        }
        if volumes.len() == 2 {
            break;
        }
    }
    assert_eq!(
        volumes.len(),
        2,
        "the Windows path contract runner must expose two writable drive volumes"
    );
    let (_, first) = volumes.remove(0);
    let (_, second) = volumes.remove(0);

    let nested = first.path().join("NestedDirectory");
    fs::create_dir(&nested).unwrap();
    fs::write(first.path().join("AGENTS.md"), "first-volume-root").unwrap();
    fs::write(nested.join("AGENTS.md"), "first-volume-nested").unwrap();
    fs::write(second.path().join("AGENTS.md"), "second-volume-root").unwrap();

    let first_resolved = ResolvedPath::resolve(first.path()).unwrap();
    let second_resolved = ResolvedPath::resolve(second.path()).unwrap();
    assert_ne!(
        drive(first_resolved.native()),
        drive(second_resolved.native())
    );
    assert!(first_resolved
        .system()
        .to_string_lossy()
        .starts_with(r"\\?\"));
    assert!(!first_resolved.native_text().starts_with(r"\\?\"));

    let case_variant = PathBuf::from(nested.to_string_lossy().to_uppercase());
    assert_eq!(
        PathKey::of(&case_variant),
        PathKey::of(ResolvedPath::resolve(&nested).unwrap().native())
    );
    let roots = super::super::agent_workspace_roots_from([
        first_resolved.system().to_path_buf(),
        second_resolved.system().to_path_buf(),
    ])
    .unwrap();
    let discovered =
        discover_workspace_instructions(no_user_instructions().path(), &roots, Some(&case_variant));
    assert!(discovered.warnings.is_empty(), "{:?}", discovered.warnings);
    let content = discovered.content.unwrap();
    assert!(
        content.find("first-volume-root").unwrap() < content.find("first-volume-nested").unwrap()
    );
    assert!(content.contains("second-volume-root"));
    assert!(!content.contains(r"\\?\"));
}
