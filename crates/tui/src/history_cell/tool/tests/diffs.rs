// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A patch's entry expanded at Codex's widths, as Codex's `diff_render`
//! snapshots show the same patches (each snapshot here is named after
//! Codex's): its header after the bullet, each file's rows under it. The
//! first row stands behind the entry's `└`, where Codex's indent is four
//! spaces; a file of several has its row under the header's, not its own
//! `└`. Collapsed, the entry is its header (a row per file of several).

use hanafubuki_agent::{AgentPatchPreview, AgentToolDisplay};

use super::{both, shown, text};
use crate::history_cell::HistoryCell;
use crate::test_support::{diff_gallery, patch_file, patch_preview, update_block};
use crate::wrapping::line_width;

fn cell(preview: AgentPatchPreview) -> super::ToolCallCell {
    shown("apply_patch", None, AgentToolDisplay::Patch { preview })
}

/// The entry expanded at `width`, each row without its trailing blanks.
fn expanded(preview: AgentPatchPreview, width: u16) -> String {
    text(&cell(preview).transcript_lines(width))
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn apply_update_block() {
    insta::assert_snapshot!("apply_update_block", expanded(update_block(), 80));
}

#[test]
fn apply_update_with_rename_block() {
    let preview = patch_preview(vec![patch_file(
        "update",
        "old_name.rs",
        Some("new_name.rs"),
        "@@ 1 1\n A\n-B\n+B changed\n C",
    )]);
    insta::assert_snapshot!("apply_update_with_rename_block", expanded(preview, 80));
}

#[test]
fn apply_multiple_files_block() {
    let preview = patch_preview(vec![
        patch_file("update", "a.txt", None, "@@ 1 1\n-one\n+one changed"),
        patch_file("add", "b.txt", None, "+new"),
    ]);
    insta::assert_snapshot!("apply_multiple_files_block", expanded(preview, 80));
}

#[test]
fn apply_add_block() {
    let preview = patch_preview(vec![patch_file(
        "add",
        "new_file.txt",
        None,
        "+alpha\n+beta",
    )]);
    insta::assert_snapshot!("apply_add_block", expanded(preview, 80));
}

#[test]
fn apply_update_block_wraps_long_lines() {
    let preview = patch_preview(vec![patch_file(
        "update",
        "long_example.txt",
        None,
        "@@ 1 1\n line 1\n-short\n+short this_is_a_very_long_modified_line_that_should_wrap_across_multiple_terminal_columns_and_continue_even_further_beyond_eighty_columns_to_force_multiple_wraps\n line 3",
    )]);
    insta::assert_snapshot!("apply_update_block_wraps_long_lines", expanded(preview, 72));
}

#[test]
fn apply_update_block_wraps_long_lines_text() {
    let preview = patch_preview(vec![patch_file(
        "update",
        "wrap_demo.txt",
        None,
        "@@ 1 1\n 1\n-2\n+added long line which wraps and_if_there_is_a_long_token_it_will_be_broken\n 3\n-4\n+4 context line which also wraps across",
    )]);
    insta::assert_snapshot!(
        "apply_update_block_wraps_long_lines_text",
        expanded(preview, 28)
    );
}

#[test]
fn apply_update_block_line_numbers_three_digits_text() {
    let body = format!(
        "@@ 97 97\n{}-line 100\n+line 100 changed\n{}",
        (97..100)
            .map(|n| format!(" line {n}\n"))
            .collect::<String>(),
        (101..104)
            .map(|n| format!(" line {n}\n"))
            .collect::<String>(),
    );
    let preview = patch_preview(vec![patch_file("update", "hundreds.txt", None, &body)]);
    insta::assert_snapshot!(
        "apply_update_block_line_numbers_three_digits_text",
        expanded(preview, 80)
    );
}

#[test]
fn diff_gallery_80x24() {
    insta::assert_snapshot!("diff_gallery_80x24", expanded(diff_gallery(), 80));
}

#[test]
fn diff_gallery_94x35() {
    insta::assert_snapshot!("diff_gallery_94x35", expanded(diff_gallery(), 94));
}

#[test]
fn diff_gallery_120x40() {
    insta::assert_snapshot!("diff_gallery_120x40", expanded(diff_gallery(), 120));
}

/// The narrow widths: no row past the entry's width.
#[test]
fn diff_gallery_at_40_and_20_columns() {
    insta::assert_snapshot!("diff_gallery_40", expanded(diff_gallery(), 40));
    insta::assert_snapshot!("diff_gallery_20", expanded(diff_gallery(), 20));
    for width in [120, 80, 40, 20, 12] {
        let cell = cell(diff_gallery());
        for line in cell
            .transcript_lines(width)
            .iter()
            .chain(&cell.display_lines(width))
        {
            assert!(line_width(line) <= usize::from(width), "{width}: {line:?}");
        }
    }
}

#[test]
fn collapsed_a_patch_is_its_header_and_its_files() {
    insta::assert_snapshot!(both(&cell(diff_gallery()), 80));
}

/// `/raw` is the expanded entry's text, its line numbers included (Codex's
/// raw lines are its diff summary).
#[test]
fn raw_lines_keep_the_line_numbers() {
    let raw = text(&cell(update_block()).raw_lines());
    assert_eq!(
        raw,
        "• Edited example.txt (+1 -1)\n  └ 1  line one\n    2 -line two\n    2 +line two changed\n    3  line three"
    );
}
