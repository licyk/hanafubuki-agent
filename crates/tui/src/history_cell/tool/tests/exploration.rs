//! Consecutive read, list and search calls in one entry.

use hanafubuki_agent::{AgentListingEntry, AgentListingEntryKind, AgentToolDisplay};

use super::{both, render};
use crate::history_cell::tool::{explores, new_tool_call, ToolCallCell};
use crate::history_cell::{ActivityDisclosure, HistoryCell};

fn read(text: &str) -> AgentToolDisplay {
    AgentToolDisplay::FileRead {
        path: "/w/file".to_string(),
        start_line: Some(1),
        text: text.to_string(),
        truncated: false,
    }
}

/// `run`, then a call `id` of `name` on `subject` joined to it.
fn joined(run: &mut ToolCallCell, id: &str, name: &str, subject: &str) {
    run.join(new_tool_call(
        id.to_string(),
        name.to_string(),
        Some(subject.to_string()),
    ));
}

#[test]
fn reads_lists_and_searches_explore() {
    assert!(explores("read_file") && explores("list_directory") && explores("search_text"));
    assert!(!explores("exec_command") && !explores("apply_patch"));
}

#[test]
fn a_run_explores_while_a_call_runs_then_shows_a_row_per_call_snapshot() {
    let mut run = new_tool_call(
        "read-1".to_string(),
        "read_file".to_string(),
        Some("src/lib.rs".to_string()),
    );
    joined(&mut run, "read-2", "read_file", "src/main.rs");
    joined(&mut run, "list-1", "list_directory", "src");
    joined(&mut run, "read-3", "read_file", "src/lib.rs");
    joined(&mut run, "search-1", "search_text", "\"todo\" in src");
    let running = render(&run, 60);
    assert!(running.contains("Exploring"), "{running}");
    assert_eq!(
        run.activity_disclosure(60),
        Some(ActivityDisclosure::Generic)
    );
    assert_eq!(
        run.activity_ids(),
        ["read-1", "read-2", "list-1", "read-3", "search-1"].map(|id| format!("tool:{id}"))
    );

    run.finish("read-1", read("fn a() {}\n"));
    run.finish("read-2", read("fn main() {}\n"));
    run.finish(
        "list-1",
        AgentToolDisplay::Listing {
            path: "/w/src".to_string(),
            entries: vec![AgentListingEntry {
                path: "src/lib.rs".to_string(),
                kind: AgentListingEntryKind::File,
                size: Some(9),
            }],
            truncated: false,
        },
    );
    run.finish("read-3", read("fn a() {}\n"));
    run.complete("search-1", true);
    assert_eq!(run.transcript_animation_tick(), None);
    insta::assert_snapshot!(both(&run, 60));
}
