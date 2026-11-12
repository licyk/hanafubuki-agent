//! Each display's view, collapsed then expanded.

use hanafubuki_agent::{
    AgentListingEntry, AgentListingEntryKind, AgentMutationKind, AgentPatchPreview,
    AgentSearchMatch, AgentToolDisplay, AgentWebResult,
};

use super::{both, command, numbered, render, shown};
use crate::history_cell::{ActivityDisclosure, HistoryCell};

#[test]
fn a_failed_command_snapshot() {
    let cell = shown(
        "exec_command",
        Some("cargo test"),
        command(
            "cargo test -p foo\n  --all-features",
            &numbered(14),
            Some(101),
        ),
    );
    insta::assert_snapshot!(both(&cell, 60));
}

#[test]
fn a_command_that_went_on_in_the_background_snapshot() {
    let cell = shown(
        "exec_command",
        Some("npm run dev"),
        AgentToolDisplay::Command {
            command: "npm run dev".to_string(),
            cwd: Some("web".to_string()),
            output: "ready on :3000\n".to_string(),
            exit_code: None,
            duration_millis: Some(10_000),
            truncated: false,
        },
    );
    insta::assert_snapshot!(both(&cell, 60));
}

fn patch(files: serde_json::Value) -> AgentToolDisplay {
    AgentToolDisplay::Patch {
        preview: serde_json::from_value::<AgentPatchPreview>(serde_json::json!({
            "files": files,
            "error": null
        }))
        .unwrap(),
    }
}

#[test]
fn a_patch_snapshot() {
    let cell = shown(
        "apply_patch",
        Some("src/lib.rs"),
        patch(serde_json::json!([{
            "operation": "update",
            "path": "src/lib.rs",
            "movedTo": null,
            "lines": [
                {"kind": "hunk", "text": "fn main"},
                {"kind": "context", "text": "fn main() {"},
                {"kind": "removed", "text": "    println!(\"old\");"},
                {"kind": "added", "text": "    println!(\"new\");"},
                {"kind": "hunk", "text": ""},
                {"kind": "added", "text": "// a line long enough to wrap at the narrow width of this test"}
            ],
            "added": 2,
            "removed": 1,
            "omittedLines": 0
        }])),
    );
    insta::assert_snapshot!(both(&cell, 40));
}

#[test]
fn a_patch_of_several_files_snapshot() {
    let mut cell = super::new_tool_call("call-1".to_string(), "apply_patch".to_string(), None);
    cell.complete("call-1", true);
    cell.show(
        "call-1",
        hanafubuki_agent::AgentToolCallDisplay {
            subject: None,
            display: Some(patch(serde_json::json!([
                {"operation": "add", "path": "notes.txt", "movedTo": null,
                 "lines": [{"kind": "added", "text": "first"}, {"kind": "added", "text": "second"}],
                 "added": 2, "removed": 0, "omittedLines": 0},
                {"operation": "update", "path": "old.txt", "movedTo": "new.txt",
                 "lines": [{"kind": "removed", "text": "a"}, {"kind": "added", "text": "b"}],
                 "added": 1, "removed": 1, "omittedLines": 0},
                {"operation": "delete", "path": "gone.txt", "movedTo": null,
                 "lines": [], "added": 0, "removed": 0, "omittedLines": 0}
            ]))),
        },
    );
    insta::assert_snapshot!(both(&cell, 60));
}

#[test]
fn a_file_read_snapshot() {
    let cell = shown(
        "read_file",
        Some("notes.txt"),
        AgentToolDisplay::FileRead {
            path: "/w/notes.txt".to_string(),
            start_line: Some(98),
            text: "ninety-eight\nninety-nine\none hundred, which is a line long enough to wrap\n"
                .to_string(),
            truncated: true,
        },
    );
    insta::assert_snapshot!(both(&cell, 40));
    assert_eq!(
        cell.activity_disclosure(40),
        Some(ActivityDisclosure::OutputLines(3))
    );
}

#[test]
fn a_read_whose_first_line_is_unknown_is_not_numbered() {
    let cell = shown(
        "read_file",
        Some("notes.txt"),
        AgentToolDisplay::FileRead {
            path: "/w/notes.txt".to_string(),
            start_line: None,
            text: "a\nb\n".to_string(),
            truncated: false,
        },
    );
    assert_eq!(render(&cell, 40), "• Read notes.txt (2 lines)");
    let expanded = super::text(&cell.transcript_lines(40));
    assert_eq!(expanded, "• Read notes.txt (2 lines)\n  └ a\n    b");
}

#[test]
fn a_listing_snapshot() {
    let entry = |path: &str, kind, size| AgentListingEntry {
        path: path.to_string(),
        kind,
        size,
    };
    let cell = shown(
        "list_directory",
        Some("src"),
        AgentToolDisplay::Listing {
            path: "/w/src".to_string(),
            entries: vec![
                entry("src/lib.rs", AgentListingEntryKind::File, Some(42)),
                entry("src/bin", AgentListingEntryKind::Directory, None),
                entry("src/big.rs", AgentListingEntryKind::File, Some(3_500)),
            ],
            truncated: false,
        },
    );
    insta::assert_snapshot!(both(&cell, 60));
}

#[test]
fn a_search_snapshot() {
    let found = |path: &str, line, text: &str| AgentSearchMatch {
        path: path.to_string(),
        line,
        text: text.to_string(),
    };
    let cell = shown(
        "search_text",
        Some("\"todo\" in src"),
        AgentToolDisplay::Search {
            query: "todo".to_string(),
            path: Some("/w/src".to_string()),
            matches: vec![
                found("src/lib.rs", 3, "// todo: one todo"),
                found("src/main.rs", 10, "let todo = 1;"),
            ],
            truncated: true,
        },
    );
    insta::assert_snapshot!(both(&cell, 60));
}

#[test]
fn the_web_tools_snapshot() {
    let search = shown(
        "web_search",
        Some("ratatui"),
        AgentToolDisplay::WebSearch {
            query: "ratatui".to_string(),
            results: vec![AgentWebResult {
                title: "Ratatui".to_string(),
                url: "https://ratatui.rs".to_string(),
                snippet: "A Rust crate".to_string(),
            }],
        },
    );
    let page = shown(
        "fetch_web_page",
        Some("https://ratatui.rs"),
        AgentToolDisplay::WebPage {
            url: "https://ratatui.rs".to_string(),
            title: Some("Ratatui".to_string()),
            text: (1..=45).map(|n| format!("paragraph {n}\n")).collect(),
            truncated: false,
        },
    );
    insta::assert_snapshot!(format!("{}\n\n{}", both(&search, 60), both(&page, 60)));
}

#[test]
fn a_move_and_a_delete_have_nothing_more_to_show() {
    let moved = shown(
        "move_path",
        Some("a.txt → b.txt"),
        AgentToolDisplay::Mutation {
            kind: AgentMutationKind::Moved,
            paths: vec!["/w/a.txt".to_string(), "/w/b.txt".to_string()],
        },
    );
    assert_eq!(render(&moved, 60), "• Moved a.txt → b.txt");
    assert_eq!(moved.activity_disclosure(60), None);
    let deleted = shown(
        "delete_path",
        None,
        AgentToolDisplay::Mutation {
            kind: AgentMutationKind::Deleted,
            paths: vec!["/w/x".to_string()],
        },
    );
    assert_eq!(render(&deleted, 60), "• Deleted /w/x");
}

#[test]
fn text_and_fields_snapshot() {
    let mcp = shown(
        "mcp__docs__lookup",
        Some("ratatui"),
        AgentToolDisplay::Text {
            text: (1..=5).map(|n| format!("result {n}\n")).collect(),
        },
    );
    let fields = shown(
        "goal_status",
        None,
        AgentToolDisplay::Fields {
            rows: vec![
                ("status".to_string(), "active".to_string()),
                ("objective".to_string(), "ship".to_string()),
            ],
        },
    );
    insta::assert_snapshot!(format!("{}\n\n{}", both(&mcp, 60), both(&fields, 60)));
}
