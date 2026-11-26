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
fn collecting_files_lists_the_package_and_leaves_out_hidden_entries() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    std::fs::create_dir_all(root.join("references")).unwrap();
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::create_dir_all(root.join("ars/shared")).unwrap();
    std::fs::create_dir_all(root.join("ars/~test/evals")).unwrap();
    std::fs::write(root.join("SKILL.md"), "body").unwrap();
    std::fs::write(root.join("references/api.md"), "reference").unwrap();
    std::fs::write(root.join("scripts/run.py"), "print()").unwrap();
    std::fs::write(root.join("agents/openai.yaml"), "interface: {}").unwrap();
    std::fs::write(root.join("ars/shared/WORKFLOW.md"), "workflow").unwrap();
    std::fs::write(root.join("ars/~test/evals/run.json"), "{}").unwrap();
    std::fs::write(root.join(".DS_Store"), "junk").unwrap();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::write(root.join(".git/config"), "junk").unwrap();

    let files = collect_files(root).unwrap();
    assert_eq!(
        files
            .iter()
            .map(|file| (file.relative_path.as_str(), file.kind))
            .collect::<Vec<_>>(),
        vec![
            ("SKILL.md", crate::model::SkillResourceKind::Reference),
            ("agents/openai.yaml", crate::model::SkillResourceKind::Asset),
            (
                "ars/shared/WORKFLOW.md",
                crate::model::SkillResourceKind::Reference
            ),
            (
                "ars/~test/evals/run.json",
                crate::model::SkillResourceKind::Asset
            ),
            (
                "references/api.md",
                crate::model::SkillResourceKind::Reference
            ),
            ("scripts/run.py", crate::model::SkillResourceKind::Script),
        ]
    );
}

/// A package far past the catalog's bound is refused where it is *written* —
/// installing or duplicating it — and never silently cut down to fit. Discovery
/// itself applies no such bound: what a package already ships is its business.
#[test]
fn a_package_past_the_bound_is_refused_rather_than_truncated() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    std::fs::create_dir_all(root.join("ars/scripts")).unwrap();
    for index in 0..(crate::model::MAX_SKILL_FILES + 10) {
        std::fs::write(root.join(format!("ars/scripts/check-{index:05}.py")), "x").unwrap();
    }

    match collect_files(root) {
        Err(crate::model::SkillError::LimitExceeded(message)) => println!("{message}"),
        Err(other) => panic!("unexpected error: {other}"),
        Ok(files) => panic!("listed {} files", files.len()),
    }
    assert!(copy_tree(root, &temporary.path().join("copy")).is_err());
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
