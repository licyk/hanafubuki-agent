// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A patch (`apply_patch`), whether or not it applied. Collapsed: `Edited`
//! and the file with its line counts, or `Edited N files` with a row per
//! file. Expanded: Codex's diff (`diff_render`).

use hanafubuki_agent::AgentPatchPreview;

use super::{Extent, View};
use crate::diff_render::{patch_diff_lines, patch_has_lines, patch_summary_rows, patch_title};
use crate::history_cell::ActivityDisclosure;

pub(super) fn view(preview: &AgentPatchPreview, failed: bool, extent: Extent) -> View {
    let summary = patch_summary_rows(preview);
    let has_diff = preview.error.is_none() && patch_has_lines(preview);
    View {
        failed: failed || preview.error.is_some(),
        title: patch_title(preview, failed),
        details: match (extent.expanded, has_diff) {
            (false, _) => Vec::new(),
            (true, true) => patch_diff_lines(preview, extent.columns),
            (true, false) => summary.clone(),
        },
        preview: summary,
        hidden: has_diff.then_some(ActivityDisclosure::Generic),
    }
}
