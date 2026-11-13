// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Which editor is chosen and how its command is split (Codex's
//! `resolve_editor_prefers_visual`, `resolve_editor_errors_when_unset`),
//! and a round trip through an editor that is a script
//! (`run_editor_returns_updated_content`), on every system, with each way
//! it can fail.

use super::{run_editor, EditorCommand, EditorError, EditorVariables};
use crate::test_support::FakeEditor;

fn variables(visual: Option<&str>, editor: Option<&str>) -> EditorVariables {
    EditorVariables {
        visual: visual.map(str::to_string),
        editor: editor.map(str::to_string),
    }
}

fn command(program: &str, args: &[&str]) -> EditorCommand {
    EditorCommand {
        program: program.to_string(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
    }
}

fn fake_command(editor: &FakeEditor) -> EditorCommand {
    variables(Some(&editor.command_line()), None)
        .command()
        .unwrap()
}

#[test]
fn visual_is_chosen_over_editor_and_a_blank_one_is_unset() {
    assert_eq!(
        variables(Some("vis"), Some("ed")).command().unwrap(),
        command("vis", &[])
    );
    assert_eq!(
        variables(None, Some("ed --wait")).command().unwrap(),
        command("ed", &["--wait"])
    );
    assert_eq!(
        variables(Some("  "), Some("ed")).command().unwrap(),
        command("ed", &[])
    );
    for unset in [variables(None, None), variables(Some(""), Some(" \t"))] {
        let error = unset.command().unwrap_err();
        assert!(matches!(error, EditorError::Unset), "{error:?}");
        assert_eq!(
            error.to_string(),
            "Set VISUAL or EDITOR to edit the message in an editor."
        );
    }
}

#[cfg(not(windows))]
#[test]
fn the_command_is_split_into_a_shells_words() {
    assert_eq!(
        variables(Some("'/opt/my editor/bin/ed' -f --title \"a b\""), None)
            .command()
            .unwrap(),
        command("/opt/my editor/bin/ed", &["-f", "--title", "a b"])
    );
    let error = variables(None, Some("vim \"-f")).command().unwrap_err();
    assert!(
        matches!(error, EditorError::Unsplittable("EDITOR")),
        "{error:?}"
    );
    assert_eq!(
        error.to_string(),
        "The editor in EDITOR is not a command line (a quote is not closed)."
    );
}

#[cfg(windows)]
#[test]
fn the_command_is_split_by_the_windows_command_lines_rules() {
    assert_eq!(
        variables(Some(r#""C:\Program Files\Vim\vim.exe" -f"#), None)
            .command()
            .unwrap(),
        command(r"C:\Program Files\Vim\vim.exe", &["-f"])
    );
    assert_eq!(
        variables(Some(r"C:\tools\ed.exe --wait"), None)
            .command()
            .unwrap(),
        command(r"C:\tools\ed.exe", &["--wait"])
    );
}

/// `code` is `code.cmd` on Windows, which a spawn by name does not find.
#[cfg(windows)]
#[test]
fn a_cmd_shim_on_the_path_is_found() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("edit.cmd"), "@exit /b 0\r\n").unwrap();
    let found = super::program_in(
        "edit",
        Some(dir.path().as_os_str().to_owned()),
        dir.path().to_path_buf(),
    );
    assert_eq!(found, dir.path().join("edit.cmd"));
    let missing = super::program_in(
        "missing",
        Some(dir.path().as_os_str().to_owned()),
        dir.path().to_path_buf(),
    );
    assert_eq!(missing, std::path::PathBuf::from("missing"));
}

#[test]
fn the_editor_edits_the_draft_in_a_markdown_file() {
    let editor = FakeEditor::new("edited\nby hand\n".as_bytes(), 0);
    let edited = run_editor(&fake_command(&editor), "the draft").unwrap();
    assert_eq!(edited, "edited\nby hand\n");
    assert_eq!(editor.seen().as_deref(), Some("the draft"));
}

#[test]
fn a_failed_editor_says_why() {
    let failed = FakeEditor::new(b"ignored", 3);
    let error = run_editor(&fake_command(&failed), "the draft").unwrap_err();
    assert!(
        matches!(&error, EditorError::Exited(status) if status.code() == Some(3)),
        "{error:?}"
    );
    assert!(
        error.to_string().starts_with("The editor exited with "),
        "{error}"
    );
    assert!(
        error.to_string().ends_with("; the draft is unchanged."),
        "{error}"
    );

    let error = run_editor(&command("hanafubuki-agent-no-such-editor", &[]), "x").unwrap_err();
    assert!(matches!(error, EditorError::NotFound(_)), "{error:?}");
    assert_eq!(
        error.to_string(),
        "The editor `hanafubuki-agent-no-such-editor` was not found; set VISUAL or EDITOR to an installed one."
    );

    let binary = FakeEditor::new(&[0xff, 0xfe, 0x00], 0);
    let error = run_editor(&fake_command(&binary), "the draft").unwrap_err();
    assert!(matches!(error, EditorError::Unreadable(_)), "{error:?}");
    assert!(
        error
            .to_string()
            .starts_with("The edited message could not be read ("),
        "{error}"
    );
}
