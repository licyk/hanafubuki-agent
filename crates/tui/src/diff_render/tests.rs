use hanafubuki_agent::PatchPreview;

use super::*;

fn render(preview: &PatchPreview, width: usize) -> String {
    create_diff_summary(preview, width)
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn several_files_each_get_a_header_snapshot() {
    let preview: PatchPreview = serde_json::from_value(serde_json::json!({
        "files": [
            {"operation": "add", "path": "new.txt", "movedTo": null,
             "lines": [{"kind": "added", "text": "hello"}], "added": 1, "removed": 0, "omittedLines": 0},
            {"operation": "update", "path": "old.txt", "movedTo": "renamed.txt",
             "lines": [{"kind": "removed", "text": "a"}, {"kind": "added", "text": "b"}],
             "added": 1, "removed": 1, "omittedLines": 3}
        ],
        "error": null
    }))
    .unwrap();
    insta::assert_snapshot!(render(&preview, 60));
}

#[test]
fn an_unreadable_patch_says_why() {
    let preview = PatchPreview {
        files: Vec::new(),
        error: Some("missing *** Begin Patch".to_string()),
    };
    assert_eq!(
        render(&preview, 60),
        "  └ the patch could not be read: \n    missing *** Begin Patch"
    );
}
