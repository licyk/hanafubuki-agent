use hanafubuki_agent_contracts::ToolCallValidationError;
use hanafubuki_agent_workspace_fs::{
    ApplyPatchRequest, ListDirectoryRequest, ReadFileRequest, SearchTextRequest, WorkspaceFsRequest,
};
use serde_json::json;

use super::parse;
use crate::tool::FsToolKind;

#[test]
fn filesystem_tools_are_typed_bounded_contracts_without_raw_escape_hatches() {
    let cases = [
        (
            FsToolKind::ListDirectory,
            json!({"path": ".", "maxDepth": 2, "maxEntries": 100}),
        ),
        (
            FsToolKind::SearchText,
            json!({"path": ".", "query": "needle", "maxDepth": 3, "maxResults": 20, "maxBytes": 4096}),
        ),
        (
            FsToolKind::ReadFile,
            json!({"path": "README.md", "offset": 0, "maxBytes": 4096}),
        ),
        (
            FsToolKind::ApplyPatch,
            json!({"patch": "*** Begin Patch\n*** Update File: notes.txt\n@@\n-a\n+b\n*** End Patch"}),
        ),
        (
            FsToolKind::MovePath,
            json!({"source": "a", "destination": "b"}),
        ),
        (
            FsToolKind::DeletePath,
            json!({"path": "b", "recursive": false}),
        ),
    ];
    for (kind, arguments) in cases {
        let (request, notes) = parse(kind, &arguments).unwrap();
        assert!(notes.is_empty());
        // Path resolution and approval are the domain's: an extra key is
        // ignored and changes nothing, so the model cannot hand in a
        // pre-approved or pre-resolved target.
        for forbidden in ["approved", "canonicalPath", "actionDigest", "recursiveGlob"] {
            let mut forged = arguments.clone();
            forged[forbidden] = json!(true);
            assert_eq!(parse(kind, &forged).unwrap().0, request);
        }
    }

    assert_eq!(
        parse(FsToolKind::DeletePath, &json!({"path": "x"})),
        Err(ToolCallValidationError::MissingArgument("recursive"))
    );
}

#[test]
fn limits_out_of_range_are_clamped_to_their_bounds_with_a_note() {
    let (request, notes) = parse(
        FsToolKind::ReadFile,
        &json!({"path": "x", "offset": 134217729_u64, "maxBytes": 262145}),
    )
    .unwrap();
    assert_eq!(
        request,
        WorkspaceFsRequest::ReadFile(ReadFileRequest {
            path: "x".to_string(),
            offset: 134217728,
            max_bytes: 262144,
        })
    );
    assert_eq!(
        notes.notes(),
        [
            "offset 134217729 is above its maximum; 134217728 was used",
            "maxBytes 262145 is above its maximum; 262144 was used",
        ]
    );
    let (request, notes) = parse(
        FsToolKind::ListDirectory,
        &json!({"path": ".", "maxDepth": 0, "maxEntries": -3}),
    )
    .unwrap();
    assert_eq!(
        request,
        WorkspaceFsRequest::ListDirectory(ListDirectoryRequest {
            path: ".".to_string(),
            max_depth: 1,
            max_entries: 1,
        })
    );
    assert_eq!(notes.notes().len(), 2);
    // A limit that is not a number is an error naming the field.
    assert_eq!(
        parse(
            FsToolKind::ReadFile,
            &json!({"path": "x", "offset": 0, "maxBytes": "lots"})
        )
        .map(|(request, _)| request)
        .unwrap_err()
        .to_string(),
        "invalid tool argument maxBytes: must be an integer"
    );
}

#[test]
fn omitted_or_null_limits_take_their_defaults_without_widening_containment() {
    let (request, notes) = parse(
        FsToolKind::SearchText,
        &json!({"path": "../outside", "query": "needle", "maxDepth": null}),
    )
    .unwrap();
    assert!(notes.is_empty());
    // The path is passed through as given: the domain resolves and contains
    // it, so defaults never change which path is checked.
    assert_eq!(
        request,
        WorkspaceFsRequest::SearchText(SearchTextRequest {
            path: "../outside".to_string(),
            query: "needle".to_string(),
            max_depth: 8,
            max_results: 50,
            max_bytes: 1024 * 1024,
        })
    );
    assert!(parse(FsToolKind::ReadFile, &json!({"path": "x"})).is_ok());
    assert_eq!(
        parse(FsToolKind::ReadFile, &json!({"maxBytes": 4})).map(|(request, _)| request),
        Err(ToolCallValidationError::MissingArgument("path"))
    );
}

#[test]
fn apply_patch_accepts_freeform_and_function_wrapped_codex_patches() {
    let patch = "*** Begin Patch\n*** Add File: notes.txt\n+hello\n*** End Patch";
    let expected = WorkspaceFsRequest::ApplyPatch(ApplyPatchRequest {
        patch: patch.to_string(),
    });
    assert_eq!(
        parse(FsToolKind::ApplyPatch, &json!(patch)).unwrap().0,
        expected
    );
    assert_eq!(
        parse(FsToolKind::ApplyPatch, &json!({"patch": patch}))
            .unwrap()
            .0,
        expected
    );
    // An extra key is ignored; the patch alone says what is written.
    assert_eq!(
        parse(
            FsToolKind::ApplyPatch,
            &json!({"patch": patch, "path": "x"})
        )
        .unwrap()
        .0,
        expected
    );
}

/// An `Add File` hunk with no `+` lines creates an empty file, so the one
/// writer covers that case too.
#[test]
fn apply_patch_can_create_an_empty_file() {
    let empty = "*** Begin Patch\n*** Add File: pkg/__init__.py\n*** End Patch";
    assert!(parse(FsToolKind::ApplyPatch, &json!(empty)).is_ok());
}
