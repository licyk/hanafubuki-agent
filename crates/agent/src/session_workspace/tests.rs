use super::*;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "hanafubuki-agent-session-workspace-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn the_branch_is_the_one_head_names_from_any_directory_of_the_work_tree() {
    let root = scratch("branch");
    write(&root.join(".git/HEAD"), "ref: refs/heads/feature/side\n");
    let nested = root.join("crates/agent");
    fs::create_dir_all(&nested).unwrap();

    let workspace = session_workspace(&nested);

    assert_eq!(workspace.branch.as_deref(), Some("feature/side"));
    assert_eq!(
        workspace.cwd.as_deref(),
        Some(nested.to_string_lossy().as_ref())
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_linked_work_tree_reads_the_head_its_gitdir_names() {
    let root = scratch("linked");
    let repository = root.join("main/.git/worktrees/side");
    write(&repository.join("HEAD"), "ref: refs/heads/side\n");
    write(
        &root.join("side/.git"),
        &format!("gitdir: {}\n", repository.display()),
    );

    assert_eq!(
        checked_out_branch(&root.join("side")).as_deref(),
        Some("side")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_detached_head_or_no_repository_has_no_branch() {
    let root = scratch("detached");
    assert_eq!(checked_out_branch(&root), None);
    write(
        &root.join(".git/HEAD"),
        "0123456789abcdef0123456789abcdef01234567\n",
    );
    assert_eq!(checked_out_branch(&root), None);
    let _ = fs::remove_dir_all(root);
}
