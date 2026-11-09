use hanafubuki_agent::{AgentCompressRange, PatchPreview};

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
            compress: None,
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
            compress: None,
        },
    );
    insta::assert_snapshot!(render(&cell, 40));
}

#[test]
fn only_an_outputs_left_out_lines_are_details_to_disclose() {
    let long_output = || ToolResultView {
        text: Some((1..=30).map(|line| format!("line {line}\n")).collect()),
        exit_code: Some(0),
        patch: None,
        compress: None,
    };
    let mut call = new_tool_call("call-1".to_string(), "shell".to_string(), None);
    assert_eq!(call.activity_disclosure(80), None);
    call.complete(false, long_output());
    assert_eq!(
        call.activity_disclosure(80),
        Some(ActivityDisclosure::OutputLines(20))
    );
    assert_eq!(call.activity_ids(), vec!["tool:call-1".to_string()]);
    let compact = call.display_lines(80).len();
    assert!(call.transcript_lines(80).len() > compact);
    // A patch's diff is already whole, whatever output came with it.
    let mut patched = new_tool_call("call-2".to_string(), "apply_patch".to_string(), None);
    patched.complete(
        false,
        ToolResultView {
            patch: Some(PatchPreview {
                files: Vec::new(),
                error: None,
            }),
            ..long_output()
        },
    );
    assert_eq!(patched.activity_disclosure(80), None);
}

#[test]
fn a_compress_call_folds_its_summaries_and_receipt() {
    let mut cell = new_tool_call("fold-1".to_string(), "compress".to_string(), None);
    cell.complete(
        false,
        ToolResultView {
            text: Some(
                "[Compressed m00002–m00004 → 1 block(s), ~8000 tokens saved.]\n · b1 full summary 120ch · head \"The setup\" … tail \"done.\"\nFingerprints show head/tail only."
                    .to_string(),
            ),
            exit_code: None,
            patch: None,
            compress: Some(vec![
                AgentCompressRange {
                    start_ref: "m00002".to_string(),
                    end_ref: "m00004".to_string(),
                    topic: Some("Setup".to_string()),
                    summary: "The setup ran.\nEverything passed.".to_string(),
                },
                AgentCompressRange {
                    start_ref: "m00006".to_string(),
                    end_ref: "m00007".to_string(),
                    topic: None,
                    summary: "Read the logs.".to_string(),
                },
            ]),
        },
    );

    assert_eq!(
        render(&cell, 100).replace('\u{2022}', "*"),
        "* Called compress: m00002–m00004 Setup, m00006–m00007\n  └ [Compressed m00002–m00004 → 1 block(s), ~8000 tokens saved.]"
    );
    assert_eq!(
        cell.activity_disclosure(100),
        Some(ActivityDisclosure::Generic)
    );
    let detailed: Vec<String> = cell
        .transcript_lines(100)
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect();
    assert_eq!(
        detailed[detailed.len() - 5..],
        [
            "    m00002–m00004 Setup",
            "    The setup ran.",
            "    Everything passed.",
            "    m00006–m00007",
            "    Read the logs.",
        ]
    );
    assert!(detailed.iter().any(|line| line.contains("Fingerprints")));
}
