// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use hanafubuki_agent::AgentPatchPreview;
use ratatui::text::Line;

use super::*;
use crate::render::highlight::{resolve_theme_by_name, set_syntax_theme};

/// The patch's title, then its diff.
fn render(preview: &AgentPatchPreview, width: usize) -> String {
    let mut lines = vec![RtLine::from(patch_title(preview, false))];
    lines.extend(patch_diff_lines(preview, width));
    lines
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
    let preview: AgentPatchPreview = serde_json::from_value(serde_json::json!({
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
    let preview = AgentPatchPreview {
        files: Vec::new(),
        error: Some("missing *** Begin Patch".to_string()),
    };
    assert_eq!(
        render(&preview, 60),
        "Could not read the patch\nmissing *** Begin Patch"
    );
    assert_eq!(patch_summary_rows(&preview).len(), 1);
    assert!(!patch_has_lines(&preview));
}

/// Each span as `text` with its foreground and its dimming.
fn styled(lines: &[Line<'static>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .filter(|span| !span.content.is_empty())
                .map(|span| {
                    let dim = if span.style.add_modifier.contains(Modifier::DIM) {
                        "~"
                    } else {
                        ""
                    };
                    match span.style.fg {
                        Some(color) => format!("[{}]{{{color}{dim}}}", span.content),
                        None => format!("[{}]{dim}", span.content),
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_patch_is_highlighted_by_its_files_language_snapshot() {
    set_syntax_theme(resolve_theme_by_name("base16-ocean-dark", None).unwrap());
    let preview: AgentPatchPreview = serde_json::from_value(serde_json::json!({
        "files": [
            {"operation": "update", "path": "src/lib.rs", "movedTo": null,
             "lines": [
                {"kind": "hunk", "text": "@@"},
                {"kind": "context", "text": "fn answer() -> u32 {"},
                {"kind": "removed", "text": "    41"},
                {"kind": "added", "text": "    42 // the answer"},
                {"kind": "context", "text": "}"}
             ],
             "added": 1, "removed": 1, "omittedLines": 0}
        ],
        "error": null
    }))
    .unwrap();
    insta::assert_snapshot!(styled(&patch_diff_lines(&preview, 80)));
}

#[test]
fn a_highlighted_row_wraps_with_the_sign_column_kept() {
    set_syntax_theme(resolve_theme_by_name("base16-ocean-dark", None).unwrap());
    let texts = vec!["let value = 1234567890;".to_string()];
    let highlighted = highlight_lines("a.rs", &texts).unwrap();
    let rows = push_wrapped_diff_line(
        DiffLineKind::Added,
        &texts[0],
        highlighted.into_iter().next(),
        None,
        12,
    );
    let text: Vec<String> = rows
        .iter()
        .map(|row| row.spans.iter().map(|span| span.content.as_ref()).collect())
        .collect();
    assert_eq!(text, ["+let value =", "  1234567890", " ;"]);
}

#[test]
fn an_added_files_rows_are_numbered_and_wrap_behind_the_gutter() {
    let preview: AgentPatchPreview = serde_json::from_value(serde_json::json!({
        "files": [{"operation": "add", "path": "notes.unknown-extension", "movedTo": null,
            "lines": (1..=10).map(|n| serde_json::json!({"kind": "added", "text": format!("line {n}")}))
                .chain([serde_json::json!({"kind": "added", "text": "a long last line"})])
                .collect::<Vec<_>>(),
            "added": 11, "removed": 0, "omittedLines": 0}],
        "error": null
    }))
    .unwrap();
    let rendered = render(&preview, 14);
    let rows: Vec<&str> = rendered.lines().collect();
    assert_eq!(rows[0], "Added notes.unknown-extension (+11 -0)");
    assert_eq!(rows[1], " 1 +line 1");
    assert_eq!(rows[10], "10 +line 10");
    assert_eq!(rows[11..], ["11 +a long las", "    t line"]);
}

#[test]
fn a_file_of_no_known_language_keeps_the_diff_colours() {
    let texts = vec!["plain words".to_string()];
    assert!(highlight_lines("notes.unknown-extension", &texts).is_none());
    let rows = push_wrapped_diff_line(DiffLineKind::Removed, &texts[0], None, None, 80);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].spans.iter().all(|span| span.style == style_del()));
}

#[test]
fn a_git_diff_is_highlighted_per_file_snapshot() {
    set_syntax_theme(resolve_theme_by_name("base16-ocean-dark", None).unwrap());
    let diff = [
        "diff --git a/src/main.py b/src/main.py",
        "index 1111111..2222222 100644",
        "--- a/src/main.py",
        "+++ b/src/main.py",
        "@@ -1,2 +1,2 @@",
        " def main():",
        "-    return 1",
        "+    return 2",
        "diff --git a/notes.txt b/notes.txt",
        "--- a/notes.txt",
        "+++ b/notes.txt",
        "@@ -1 +1 @@",
        "-old words",
        "+new words",
        "\\ No newline at end of file",
    ]
    .join("\n");
    insta::assert_snapshot!(styled(&git_diff_lines(&diff)));
    assert_eq!(
        git_diff_lines("  \n"),
        vec![Line::from("No changes detected.".italic())]
    );
}
