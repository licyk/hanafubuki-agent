use super::*;

#[test]
fn a_relative_path_is_normalized_or_refused() {
    assert_eq!(
        normalize_relative("references/api.md").unwrap(),
        "references/api.md"
    );
    assert_eq!(
        normalize_relative("./scripts/run.py").unwrap(),
        "scripts/run.py"
    );
    assert_eq!(
        normalize_relative("assets\\logo.png").unwrap(),
        "assets/logo.png"
    );
    for path in ["", "..", "a/../b", "/etc/passwd", ".git/config", "a:b"] {
        assert!(
            normalize_relative(path).is_err(),
            "{path} should be refused"
        );
    }
}

#[test]
fn collecting_files_skips_hidden_entries_and_reports_them_nowhere() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    std::fs::create_dir_all(root.join("references")).unwrap();
    std::fs::write(root.join("SKILL.md"), "body").unwrap();
    std::fs::write(root.join("references/api.md"), "reference").unwrap();
    std::fs::write(root.join(".DS_Store"), "junk").unwrap();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::write(root.join(".git/config"), "junk").unwrap();

    let files = collect_files(root).unwrap();
    assert_eq!(
        files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect::<Vec<_>>(),
        vec!["SKILL.md", "references/api.md"]
    );
}

#[test]
fn copying_a_tree_reproduces_exactly_what_would_be_loaded() {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("source");
    let destination = temporary.path().join("destination");
    std::fs::create_dir_all(source.join("scripts")).unwrap();
    std::fs::write(source.join("SKILL.md"), "body").unwrap();
    std::fs::write(source.join("scripts/run.py"), "print()").unwrap();
    std::fs::write(source.join(".hidden"), "junk").unwrap();

    copy_tree(&source, &destination).unwrap();
    assert_eq!(
        std::fs::read_to_string(destination.join("scripts/run.py")).unwrap(),
        "print()"
    );
    assert!(!destination.join(".hidden").exists());
}

#[test]
fn replacing_a_directory_keeps_the_old_one_until_the_new_one_lands() {
    let temporary = tempfile::tempdir().unwrap();
    let staging = temporary.path().join("staging");
    let destination = temporary.path().join("live");
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::create_dir_all(&destination).unwrap();
    std::fs::write(staging.join("SKILL.md"), "new").unwrap();
    std::fs::write(destination.join("SKILL.md"), "old").unwrap();
    std::fs::write(destination.join("stale.md"), "gone").unwrap();

    replace_directory(&staging, &destination).unwrap();
    assert_eq!(
        std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
        "new"
    );
    assert!(!destination.join("stale.md").exists());
}

#[test]
fn a_path_outside_its_root_is_refused() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    assert!(ensure_below(&root, &root.join("inside")).is_ok());
    assert!(ensure_below(&root, temporary.path().join("outside").as_path()).is_err());
}
