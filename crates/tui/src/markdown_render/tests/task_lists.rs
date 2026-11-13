// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Task-list boxes replace unordered bullets and keep ordered numbers; the
//! hanging indent follows the task's text.

use crate::markdown_render::render_markdown_text_with_width;

/// Codex's samples, nested and quoted tasks, and the markers that are no
/// tasks (escaped, in code, in a fence).
pub(super) const TASK_LIST_SAMPLES: [&str; 13] = [
    "- [ ] Unchecked\n- [x] Checked\n- [X] Uppercase\n- Ordinary\n",
    "- [ ] Parent task with enough words to wrap\n  - [x] Nested task\n  - Ordinary child\n- [x] Done\n",
    "9. [ ] Numbered task with enough words to wrap\n10. [x] Done\n11. Ordinary\n",
    "> - [ ] Quoted task with enough words to wrap\n> - [x] Done\n",
    "- [ ] First paragraph\n\n  Second paragraph\n\n- [x] Next task\n",
    "- [ ]\n- [x]\n",
    "- [x] **Bold**, *italic*, and `code`\n\n`- [x] inline`\n\n```text\n- [x] fenced\n```\n\n[ ] Plain text\n\n- \\[x] Escaped\n",
    "- [ ] \n  - child\n\n- [x] # title\n\n- [x] > quote\n\n- [x] ---\n",
    "-     [x] code, not a task\n",
    "- [x] <b>done</b>\n\n- Parent\n  - [x] \n\n  Continued paragraph\n",
    "- [x] <div>done</div>\n",
    "- [x] \n  <div>done</div>\n",
    "> - Parent\n>   - [x] \n>\n>   Continued paragraph\n",
];

#[test]
fn lists_preserve_layout() {
    let stages = TASK_LIST_SAMPLES
        .map(|source| render_markdown_text_with_width(source, Some(24)).to_string())
        .join("\n---\n");
    insta::assert_snapshot!(stages);
}
