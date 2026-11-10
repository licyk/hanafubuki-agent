// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/command_migration_tests.rs` at commit
// `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the name-fallback test is not ported (the
// plugin migration requires a description); a migration of a package's
// declared and default command paths is added.

use std::fs;
use std::path::Path;

use super::*;

#[test]
fn command_skill_names_must_fit_the_skill_name_limit() {
    let source_name = "this-is-a-deeply-nested-command-with-a-very-long-name";
    let file = Path::new("commands/this/is/a/deeply/nested/command/with/a/very/long/name.md");
    let command = parse_command("---\ndescription: Review PR\n---\nReview\n");

    assert_eq!(skill_name_if_supported(source_name, file, &command), None);
}

#[test]
fn commands_with_overlong_descriptions_are_preserved() {
    let description = "x".repeat(1025);
    let command = parse_command(&format!("---\ndescription: {description}\n---\nReview\n"));

    assert_eq!(
        skill_name_if_supported("review", Path::new("commands/review.md"), &command),
        Some("source-command-review".to_string())
    );
    let rendered = render_skill(
        &command.body,
        "source-command-review",
        &description,
        "review",
    );
    assert_eq!(
        parse_command(&rendered).description.as_deref(),
        Some(description.as_str())
    );
}

#[test]
fn commands_with_provider_runtime_expansion_are_skipped() {
    let command =
        parse_command("---\ndescription: Deploy\n---\nDeploy $ARGUMENTS from @release.yaml\n");

    assert_eq!(
        skill_name_if_supported("deploy", Path::new("commands/deploy.md"), &command),
        None
    );
}

#[test]
fn commands_without_description_are_skipped() {
    let command = parse_command("Review the current change.\n");

    assert_eq!(
        skill_name_if_supported("review", Path::new("commands/review.md"), &command),
        None
    );
}

#[test]
fn command_slug_collisions_are_skipped() {
    let root = tempfile::tempdir().unwrap();
    let commands = root.path().join("commands");
    fs::create_dir_all(&commands).unwrap();
    fs::write(
        commands.join("foo-bar.md"),
        "---\ndescription: First\n---\nRun the first command.\n",
    )
    .unwrap();
    fs::write(
        commands.join("foo_bar.md"),
        "---\ndescription: Second\n---\nRun the second command.\n",
    )
    .unwrap();

    assert_eq!(
        unique_command_sources(supported_command_sources(&commands).unwrap()),
        Vec::<CommandSource>::new()
    );
}

#[test]
fn migration_writes_skills_from_the_default_or_declared_command_paths() {
    let root = tempfile::tempdir().unwrap();
    let package = root.path();
    fs::create_dir_all(package.join("commands/git")).unwrap();
    fs::write(
        package.join("commands/git/commit.md"),
        "---\ndescription: Commit the change\n---\nWrite a commit message.\n",
    )
    .unwrap();
    fs::write(
        package.join("commands/README.md"),
        "---\ndescription: x\n---\n",
    )
    .unwrap();
    fs::create_dir_all(package.join("extra")).unwrap();
    fs::write(
        package.join("extra/lint.md"),
        "---\ndescription: Lint\n---\nRun the linter.\n",
    )
    .unwrap();

    assert_eq!(
        migrate_commands(package, &[]).unwrap(),
        vec!["source-command-git-commit".to_string()]
    );
    let skill = fs::read_to_string(
        migrated_command_skills_root(package).join("source-command-git-commit/SKILL.md"),
    )
    .unwrap();
    assert!(skill.starts_with(
        "---\nname: \"source-command-git-commit\"\ndescription: \"Commit the change\"\n---\n"
    ));
    assert!(skill.contains("Write a commit message."));

    assert_eq!(
        migrate_commands(package, &[package.join("extra")]).unwrap(),
        vec!["source-command-lint".to_string()]
    );
    assert!(!migrated_command_skills_root(package)
        .join("source-command-git-commit")
        .exists());
}
