use super::*;
use serde_json::json;

fn json_output(value: Value) -> ToolOutput {
    hanafubuki_agent_contracts::ToolResult::success(value).output
}

#[test]
fn memory_presentations_keep_resources_matches_and_readable_text() {
    let output = json_output(json!({"resources": [
        {"id": "MEMORY.md", "bytes": 256, "future": true}, null,
        {"id": "notes/build.md", "bytes": 64}
    ]}));
    let Some(AgentToolDisplay::Listing { entries, .. }) =
        MemoryPresenter.display("list_agent_memories", &json!({}), &output)
    else {
        panic!("listing")
    };
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].path, "MEMORY.md");
    assert_eq!(entries[1].size, Some(64));

    let output = json_output(json!({"matches": [
        {"resourceId": "notes/build.md", "line": 8, "excerpt": "Use cargo xtask"}, false
    ]}));
    let Some(AgentToolDisplay::Search { matches, query, .. }) =
        MemoryPresenter.display("search_agent_memories", &json!({"query": "cargo"}), &output)
    else {
        panic!("search")
    };
    assert_eq!(query, "cargo");
    assert_eq!(
        matches,
        vec![AgentSearchMatch {
            path: "notes/build.md".into(),
            line: 8,
            text: "Use cargo xtask".into()
        }]
    );

    let output = json_output(
        json!({"generation": 2, "resourceId": "MEMORY.md", "startLine": 4, "endLine": 5, "content": "first\nsecond", "truncated": true}),
    );
    assert_eq!(
        MemoryPresenter.display("read_agent_memory", &json!({}), &output),
        Some(AgentToolDisplay::FileRead {
            path: "MEMORY.md".into(),
            start_line: Some(4),
            text: "first\nsecond".into(),
            truncated: true
        })
    );
}

#[test]
fn memory_note_reports_queueing_and_errors_use_the_fallback() {
    let mut output = json_output(
        json!({"operation": "forget", "noteId": "note-1", "status": "queued_for_consolidation"}),
    );
    let Some(AgentToolDisplay::Text { text }) =
        MemoryPresenter.display("append_agent_memory_note", &json!({}), &output)
    else {
        panic!("note")
    };
    assert!(
        text.contains("queued for consolidation")
            && text.contains("forget")
            && text.contains("note-1")
    );
    output.is_error = true;
    assert!(MemoryPresenter
        .display("append_agent_memory_note", &json!({}), &output)
        .is_none());
    assert!(MemoryPresenter
        .display(
            "read_agent_memory",
            &json!({}),
            &json_output(json!({"unrecognized": true}))
        )
        .is_none());
}
