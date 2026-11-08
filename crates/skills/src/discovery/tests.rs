use super::*;
use crate::model::SkillIssueSeverity;

fn write(path: &std::path::Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

const DOCUMENT: &str = "---\nname: demo\ndescription: Does a thing.\n---\nBody.\n";

#[test]
fn a_directory_holding_skill_md_is_a_skill() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    write(&root.join("demo/SKILL.md"), DOCUMENT);
    write(&root.join("demo/references/api.md"), "reference");
    write(&root.join("demo/scripts/run.py"), "print()");
    write(&root.join("demo/LICENSE.txt"), "MIT");
    write(&root.join("demo/logo.png"), "png");

    let scan = scan_root(root, SkillScope::User);
    assert!(scan.issues.is_empty());
    assert_eq!(scan.skills.len(), 1);
    let skill = &scan.skills[0];
    assert_eq!(skill.entry.id.as_str(), "user:demo");
    assert_eq!(skill.entry.name, "demo");
    // A licence, a readme, and a loose image are ordinary package contents. The
    // previous allowlist refused the whole Skill for any of them.
    let paths = skill
        .entry
        .resources
        .iter()
        .map(|resource| (resource.path.as_str(), resource.kind))
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![
            ("LICENSE.txt", SkillResourceKind::Asset),
            ("logo.png", SkillResourceKind::Asset),
            ("references/api.md", SkillResourceKind::Reference),
            ("scripts/run.py", SkillResourceKind::Script),
        ]
    );
}

#[test]
fn skills_are_found_at_any_depth_but_never_inside_one_another() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    write(&root.join("group/nested/SKILL.md"), DOCUMENT);
    // A Skill that ships an example package must not register that example as a
    // second Skill.
    write(
        &root.join("group/nested/examples/sample/SKILL.md"),
        DOCUMENT,
    );

    let scan = scan_root(root, SkillScope::Repo);
    assert_eq!(
        scan.skills
            .iter()
            .map(|skill| skill.entry.id.as_str().to_string())
            .collect::<Vec<_>>(),
        vec!["repo:group/nested".to_string()]
    );
}

#[test]
fn a_broken_skill_is_reported_and_the_rest_still_load() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    write(&root.join("good/SKILL.md"), DOCUMENT);
    write(&root.join("broken/SKILL.md"), "no frontmatter here\n");

    let scan = scan_root(root, SkillScope::User);
    assert_eq!(scan.skills.len(), 1);
    assert_eq!(scan.skills[0].entry.id.as_str(), "user:good");
    assert_eq!(scan.issues.len(), 1);
    assert!(scan.issues[0].message.contains("frontmatter"));
    assert_eq!(scan.issues[0].severity, SkillIssueSeverity::Error);
}

#[test]
fn a_missing_root_is_simply_empty() {
    let temporary = tempfile::tempdir().unwrap();
    let scan = scan_root(&temporary.path().join("absent"), SkillScope::System);
    assert!(scan.skills.is_empty());
    assert!(scan.issues.is_empty());
}

#[test]
fn only_user_skills_are_editable() {
    let temporary = tempfile::tempdir().unwrap();
    write(&temporary.path().join("demo/SKILL.md"), DOCUMENT);
    for (scope, editable) in [
        (SkillScope::User, true),
        (SkillScope::System, false),
        (SkillScope::Repo, false),
    ] {
        let scan = scan_root(temporary.path(), scope);
        assert_eq!(scan.skills[0].entry.editable, editable);
    }
}

#[test]
fn what_a_loaded_skill_ignored_is_reported_against_its_file() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    write(
        &root.join("demo/SKILL.md"),
        "---\nname: demo\ndescription: Does a thing.\nmetadata:\n  version: 1.0\n---\nBody.\n",
    );
    write(
        &root.join("demo/agents/openai.yaml"),
        "interface:\n  display_name: 42\n  short_description: Short\n",
    );

    let scan = scan_root(root, SkillScope::User);
    assert_eq!(scan.skills.len(), 1);
    let skill = &scan.skills[0];
    assert_eq!(skill.entry.version, None);
    assert_eq!(
        skill.entry.interface.short_description.as_deref(),
        Some("Short")
    );
    assert!(skill.issues.is_empty(), "the issues moved to the scan");
    let issues = scan
        .issues
        .iter()
        .map(|issue| (issue.severity, issue.path.clone(), issue.message.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        issues,
        vec![
            (
                SkillIssueSeverity::Warning,
                native_text(&root.join("demo/SKILL.md")),
                "ignored metadata.version: expected a string, found a number"
            ),
            (
                SkillIssueSeverity::Warning,
                native_text(&root.join("demo/agents/openai.yaml")),
                "ignored interface.display_name: expected a string, found a number"
            ),
        ]
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_directory_below_the_root_is_skipped_with_an_issue() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    write(&root.join("good/SKILL.md"), DOCUMENT);
    write(&root.join("locked/hidden/SKILL.md"), DOCUMENT);
    let locked = root.join("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // A process that ignores permissions (root, CAP_DAC_READ_SEARCH) can still
    // list the directory, so there is nothing to skip and nothing to test.
    let still_readable = std::fs::read_dir(&locked).is_ok();
    let scan = (!still_readable).then(|| scan_root(root, SkillScope::User));
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    let Some(scan) = scan else {
        return;
    };

    assert_eq!(
        scan.skills
            .iter()
            .map(|skill| skill.entry.id.as_str())
            .collect::<Vec<_>>(),
        vec!["user:good"]
    );
    assert_eq!(scan.issues.len(), 1);
    assert_eq!(scan.issues[0].path, native_text(&locked));
    assert!(scan.issues[0].message.contains("could not be read"));
    assert_eq!(scan.issues[0].severity, SkillIssueSeverity::Error);
}

#[cfg(unix)]
#[test]
fn an_unreadable_root_still_fails_as_a_whole() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("root");
    write(&root.join("good/SKILL.md"), DOCUMENT);
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o000)).unwrap();
    let still_readable = std::fs::read_dir(&root).is_ok();
    let scan = (!still_readable).then(|| scan_root(&root, SkillScope::User));
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    let Some(scan) = scan else {
        return;
    };

    assert!(scan.skills.is_empty());
    assert_eq!(scan.issues.len(), 1);
    assert_eq!(scan.issues[0].path, native_text(&root));
    assert_eq!(scan.issues[0].severity, SkillIssueSeverity::Error);
}

#[cfg(unix)]
#[test]
fn a_symlink_inside_a_skill_is_never_one_of_its_resources() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("root");
    write(&temporary.path().join("outside/secret.md"), "secret");
    write(&root.join("demo/SKILL.md"), DOCUMENT);
    std::fs::create_dir_all(root.join("demo/references")).unwrap();
    std::os::unix::fs::symlink(
        temporary.path().join("outside/secret.md"),
        root.join("demo/references/secret.md"),
    )
    .unwrap();
    std::os::unix::fs::symlink(temporary.path().join("outside"), root.join("demo/linked")).unwrap();

    let scan = scan_root(&root, SkillScope::User);
    assert!(scan.issues.is_empty());
    assert!(scan.skills[0].entry.resources.is_empty());
}

#[test]
fn repository_roots_climb_to_the_git_root_and_no_further() {
    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path().to_path_buf();
    // The outer `.git` bounds the walk whatever lies above the temporary
    // directory.
    std::fs::create_dir_all(base.join(".git")).unwrap();
    std::fs::create_dir_all(base.join("repo/.git")).unwrap();
    std::fs::create_dir_all(base.join("repo/apps/web")).unwrap();
    std::fs::create_dir_all(base.join("loose")).unwrap();

    let roots = repository_skill_roots(&[
        base.join("repo/apps/web"),
        base.join("repo/apps"),
        base.join("loose"),
    ]);
    let skills = |directory: &str| base.join(directory).join(".agents").join("skills");
    // Nearest first, each once although two workspace roots share
    // ancestors, and each walk stops at its nearest repository root.
    assert_eq!(
        roots,
        vec![
            skills("repo/apps/web"),
            skills("repo/apps"),
            skills("repo"),
            skills("loose"),
            base.join(".agents").join("skills"),
        ]
    );
}
