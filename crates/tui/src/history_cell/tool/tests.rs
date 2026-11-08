use hanafubuki_agent::PatchPreview;

use super::*;

fn render(cell: &ToolCallCell, width: u16) -> String {
    cell.display_lines(width)
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

fn finished(result: ToolResultView, is_error: bool) -> ToolCallCell {
    let mut cell = new_tool_call(
        "call-1".to_string(),
        "exec_command".to_string(),
        Some("cargo test".to_string()),
    );
    cell.complete(is_error, result);
    cell
}

#[test]
fn a_running_call_shows_its_name_and_subject() {
    let cell = new_tool_call(
        "call-1".to_string(),
        "exec_command".to_string(),
        Some("cargo test\u{1b}[2J".to_string()),
    );
    let rendered = render(&cell, 60);
    assert!(
        rendered.ends_with("Running exec_command: cargo test"),
        "{rendered}"
    );
}

#[test]
fn a_long_output_keeps_its_ends_snapshot() {
    let text = (1..=14)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let cell = finished(
        ToolResultView {
            text: Some(text),
            exit_code: Some(1),
            patch: None,
        },
        true,
    );
    insta::assert_snapshot!(render(&cell, 60));
}

#[test]
fn a_patch_shows_its_diff_snapshot() {
    let preview: PatchPreview = serde_json::from_value(serde_json::json!({
        "files": [{
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
        }],
        "error": null
    }))
    .unwrap();
    let mut cell = new_tool_call("call-1".to_string(), "apply_patch".to_string(), None);
    cell.complete(
        false,
        ToolResultView {
            text: Some("Done".to_string()),
            exit_code: None,
            patch: Some(preview),
        },
    );
    insta::assert_snapshot!(render(&cell, 40));
}
