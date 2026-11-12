use hanafubuki_agent_contracts::ToolResult;
use serde_json::json;

use super::*;

const PATCH: &str = "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** Add File: src/new.rs\n+fn main() {}\n*** End Patch";

fn output(value: Value) -> ToolOutput {
    ToolResult::success(value).output
}

#[test]
fn a_read_shows_its_text_numbered_from_the_line_it_starts_on() {
    let arguments = json!({ "path": "src/lib.rs", "offset": 0, "maxBytes": null });
    let read = |offset: u64, start_line: Option<u64>| {
        let mut read = json!({
            "type": "file_read", "path": "/w/src/lib.rs", "content": "fn a() {}\n",
            "sha256": "00", "offset": offset, "nextOffset": offset + 10, "bytes": 10,
            "totalBytes": 30, "truncated": true
        });
        if let Some(start_line) = start_line {
            read["startLine"] = json!(start_line);
        }
        output(read)
    };
    let start_line = |output: &ToolOutput| match FsPresenter.display(READ_FILE, &arguments, output)
    {
        Some(AgentToolDisplay::FileRead { start_line, .. }) => start_line,
        other => panic!("a read shows as one: {other:?}"),
    };

    assert_eq!(
        FsPresenter.subject(READ_FILE, &arguments).as_deref(),
        Some("src/lib.rs")
    );
    assert_eq!(
        FsPresenter.display(READ_FILE, &arguments, &read(0, Some(1))),
        Some(AgentToolDisplay::FileRead {
            path: "/w/src/lib.rs".to_string(),
            start_line: Some(1),
            text: "fn a() {}\n".to_string(),
            truncated: true,
        })
    );
    assert_eq!(start_line(&read(10, Some(4))), Some(4));
    // A read recorded before the reader counted lines: numbered only from
    // the start.
    assert_eq!(start_line(&read(0, None)), Some(1));
    assert_eq!(start_line(&read(10, None)), None);
}

#[test]
fn a_listing_and_a_search_show_their_entries_and_matches() {
    let listing = output(json!({
        "type": "directory_listing", "root": "/w/src", "truncated": false,
        "entries": [
            { "path": "src/bin", "kind": "directory", "size": 0, "depth": 1 },
            { "path": "src/lib.rs", "kind": "file", "size": 42, "depth": 1 }
        ]
    }));
    assert_eq!(
        FsPresenter.display(LIST_DIRECTORY, &json!({ "path": "src" }), &listing),
        Some(AgentToolDisplay::Listing {
            path: "/w/src".to_string(),
            entries: vec![
                AgentListingEntry {
                    path: "src/bin".to_string(),
                    kind: AgentListingEntryKind::Directory,
                    size: None,
                },
                AgentListingEntry {
                    path: "src/lib.rs".to_string(),
                    kind: AgentListingEntryKind::File,
                    size: Some(42),
                },
            ],
            truncated: false,
        })
    );

    let search = json!({ "path": "src", "query": "fn main" });
    let found = output(json!({
        "type": "search_results", "root": "/w/src", "scannedFiles": 3, "scannedBytes": 90,
        "truncated": false,
        "matches": [{ "path": "src/main.rs", "line": 7, "column": 1, "preview": "fn main() {" }]
    }));
    assert_eq!(
        FsPresenter.subject(SEARCH_TEXT, &search).as_deref(),
        Some("\"fn main\" in src")
    );
    assert_eq!(
        FsPresenter.display(SEARCH_TEXT, &search, &found),
        Some(AgentToolDisplay::Search {
            query: "fn main".to_string(),
            path: Some("/w/src".to_string()),
            matches: vec![AgentSearchMatch {
                path: "src/main.rs".to_string(),
                line: 7,
                text: "fn main() {".to_string(),
            }],
            truncated: false,
        })
    );
}

#[test]
fn a_patch_shows_its_diff_from_a_function_or_a_freeform_call() {
    let applied =
        output(json!({ "type": "patch_applied", "added": [], "modified": [], "deleted": [] }));
    for arguments in [json!({ "patch": PATCH }), Value::String(PATCH.to_string())] {
        assert_eq!(
            FsPresenter.subject(APPLY_PATCH, &arguments).as_deref(),
            Some("src/lib.rs and 1 more")
        );
        let Some(AgentToolDisplay::Patch { preview }) =
            FsPresenter.display(APPLY_PATCH, &arguments, &applied)
        else {
            panic!("a patch shows its diff");
        };
        assert_eq!(preview.error, None);
        assert_eq!(preview.files.len(), 2);
        assert_eq!(preview.files[0].operation, AgentPatchOperation::Update);
        assert_eq!(
            preview.files[0].lines.last(),
            Some(&AgentPatchLine {
                kind: AgentPatchLineKind::Added,
                text: "new".to_string(),
            })
        );
        assert_eq!(preview.files[1].added, 1);
    }
}

#[test]
fn a_move_and_a_delete_name_their_paths_and_an_error_takes_the_fallback() {
    let moving = json!({ "source": "a.txt", "destination": "b.txt" });
    let moved = output(json!({
        "type": "path_moved", "source": "/w/a.txt", "destination": "/w/b.txt", "kind": "file"
    }));
    assert_eq!(
        FsPresenter.subject(MOVE_PATH, &moving).as_deref(),
        Some("a.txt → b.txt")
    );
    assert_eq!(
        FsPresenter.display(MOVE_PATH, &moving, &moved),
        Some(AgentToolDisplay::Mutation {
            kind: AgentMutationKind::Moved,
            paths: vec!["/w/a.txt".to_string(), "/w/b.txt".to_string()],
        })
    );
    let deleting = json!({ "path": "old", "recursive": true });
    let deleted = output(json!({
        "type": "path_deleted", "path": "/w/old", "kind": "directory", "bytes": null, "sha256": null
    }));
    assert_eq!(
        FsPresenter.display(DELETE_PATH, &deleting, &deleted),
        Some(AgentToolDisplay::Mutation {
            kind: AgentMutationKind::Deleted,
            paths: vec!["/w/old".to_string()],
        })
    );
    let failed = ToolResult::model_visible_error(json!({ "error": "path missing" })).output;
    assert_eq!(FsPresenter.display(DELETE_PATH, &deleting, &failed), None);
}
