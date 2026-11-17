use clap::{error::ErrorKind, CommandFactory};

use super::*;

#[test]
fn every_command_has_generated_help_without_running_it() {
    let command = Cli::command();
    command.clone().debug_assert();
    for name in command.get_subcommands().map(|command| command.get_name()) {
        let error = Cli::try_parse_from(["xtask", name, "--help"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DisplayHelp, "{name}");
        assert_eq!(error.exit_code(), 0);
        assert!(error.to_string().contains(&format!("cargo xtask {name}")));
    }
    assert_eq!(
        Cli::try_parse_from(["xtask", "help", "tag"])
            .unwrap_err()
            .kind(),
        ErrorKind::DisplayHelp
    );
}

#[test]
fn parses_release_arguments_into_typed_options() {
    let Task::Version(options) = Cli::try_parse_from(["xtask", "version", "--dry-run", "minor"])
        .unwrap()
        .task
    else {
        panic!("expected version")
    };
    assert_eq!(options.level, version::Level::Minor);
    assert!(options.dry_run);

    let Task::Tag(options) = Cli::try_parse_from([
        "xtask",
        "tag",
        "--version",
        "v0.7.3",
        "-m",
        "Release notes",
        "--push",
        "--remote",
        "upstream",
        "--dry-run",
    ])
    .unwrap()
    .task
    else {
        panic!("expected tag")
    };
    assert_eq!(options.version.as_deref(), Some("v0.7.3"));
    assert_eq!(options.message.as_deref(), Some("Release notes"));
    assert_eq!(options.remote.as_deref(), Some("upstream"));
    assert!(options.push && options.dry_run);
}

#[test]
fn report_and_asset_defaults_remain_optional() {
    for (args, expected) in [
        (vec!["xtask", "module-size-report"], 20),
        (vec!["xtask", "module-size-report", "--top", "3"], 3),
    ] {
        let Task::ModuleSizeReport { top } = Cli::try_parse_from(args).unwrap().task else {
            panic!("expected report")
        };
        assert_eq!(top.get(), expected);
    }
    let Task::WelcomeAssets { source, output } =
        Cli::try_parse_from(["xtask", "welcome-assets", "--source", "audit directory"])
            .unwrap()
            .task
    else {
        panic!("expected assets")
    };
    assert_eq!(source, PathBuf::from("audit directory"));
    assert!(output.is_none());
}

#[test]
fn rejects_missing_conflicting_and_invalid_arguments() {
    for args in [
        vec![],
        vec!["unknown"],
        vec!["check", "extra"],
        vec!["check-locks", "--push"],
        vec!["version"],
        vec!["version", "invalid"],
        vec!["version", "patch", "minor"],
        vec!["tag", "--version", "0.7.3", "--name", "other"],
        vec!["tag", "--message"],
        vec!["module-size-report", "--top", "0"],
        vec!["module-size-report", "--top", "not-a-number"],
        vec!["welcome-assets"],
        vec!["welcome-preview", "--input", "cells.json"],
        vec!["welcome-preview", "--input", "--output", "preview.svg"],
    ] {
        let error =
            Cli::try_parse_from(std::iter::once("xtask").chain(args.iter().copied())).unwrap_err();
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }
}

#[test]
fn parsed_preview_paths_reach_the_real_command_handler() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("terminal cells.json");
    let output = directory.path().join("terminal preview.svg");
    std::fs::write(
        &input,
        r#"{"width":1,"height":1,"cells":[{"symbol":"X","color":[1,2,3]}]}"#,
    )
    .unwrap();
    let task = Cli::try_parse_from([
        std::ffi::OsStr::new("xtask"),
        std::ffi::OsStr::new("welcome-preview"),
        std::ffi::OsStr::new("--input"),
        input.as_os_str(),
        std::ffi::OsStr::new("--output"),
        output.as_os_str(),
    ])
    .unwrap()
    .task;
    crate::run(directory.path(), task).unwrap();
    let svg = std::fs::read_to_string(output).unwrap();
    assert!(svg.contains("#010203") && svg.contains(">X</text>"));
}
