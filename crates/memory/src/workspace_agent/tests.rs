use std::collections::VecDeque;
use std::sync::Mutex;

use hanafubuki_agent_model_api::{
    ModelApiProtocol, ModelEventStream, ModelFuture, ModelResponseMetadata, ModelResult,
    ModelStream, ModelToolCall, ToolProviderCapabilities,
};
use tempfile::TempDir;

use super::*;

struct FixtureSession {
    turns: Mutex<VecDeque<VecDeque<ModelResult<ModelEvent>>>>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl ModelSession for FixtureSession {
    fn tool_capabilities(&self) -> ToolProviderCapabilities {
        ModelApiProtocol::Responses.tool_capabilities()
    }

    fn stream(
        &self,
        request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        self.requests.lock().unwrap().push(request);
        let events = self.turns.lock().unwrap().pop_front().unwrap();
        Box::pin(async move { Ok(Box::new(FixtureStream { events }) as ModelStream) })
    }
}

struct FixtureStream {
    events: VecDeque<ModelResult<ModelEvent>>,
}

impl ModelEventStream for FixtureStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        let event = self.events.pop_front();
        Box::pin(async move { event })
    }
}

fn tool_turn(calls: Vec<(&str, &str, Value)>) -> VecDeque<ModelResult<ModelEvent>> {
    let mut events = VecDeque::from([Ok(ModelEvent::ResponseStarted {
        response_id: "response".into(),
        metadata: ModelResponseMetadata::default(),
    })]);
    for (item_id, name, arguments) in calls {
        events.push_back(Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: item_id.into(),
                    call_id: format!("call-{item_id}"),
                    namespace: None,
                    name: name.into(),
                    arguments,
                    argument_error: None,
                },
            },
        }));
    }
    events.push_back(Ok(ModelEvent::ResponseCompleted {
        response_id: "response".into(),
        usage: None,
        end_turn: Some(false),
    }));
    events
}

#[tokio::test]
async fn restricted_agent_incrementally_updates_only_published_memory_files() {
    let temporary = TempDir::new().unwrap();
    let workspace = temporary.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(workspace.join("rollout_summaries")).unwrap();
    fs::write(
        workspace.join("raw_memories.md"),
        "durable fact from session 7",
    )
    .unwrap();
    fs::write(
        workspace.join("semantic_diff.md"),
        "# Memory semantic diff\n\nsemanticChanges: 1\n",
    )
    .unwrap();
    fs::write(workspace.join("MEMORY.md"), "# Memory\n").unwrap();
    fs::write(workspace.join("memory_summary.md"), "v1\n").unwrap();
    let final_turn = VecDeque::from([
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage {
                item_id: "final".into(),
                phase: None,
                text: "done".into(),
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "response-final".into(),
            usage: None,
            end_turn: Some(true),
        }),
    ]);
    let session = Arc::new(FixtureSession {
        turns: Mutex::new(VecDeque::from([
            tool_turn(vec![("1", "memory_list_files", json!({}))]),
            tool_turn(vec![
                (
                    "2",
                    "memory_write_file",
                    json!({"path":"MEMORY.md","content":"# Memory\n\nDurable fact. Source: 7.\n"}),
                ),
                (
                    "3",
                    "memory_write_file",
                    json!({"path":"memory_summary.md","content":"v1\n\n- Durable fact: MEMORY.md\n"}),
                ),
            ]),
            final_turn,
        ])),
        requests: Mutex::new(Vec::new()),
    });

    run_memory_workspace_agent(
        session.clone(),
        "fixture".into(),
        workspace.clone(),
        &[],
        CancellationToken::default(),
    )
    .await
    .unwrap();
    let requests = session.requests.lock().unwrap();
    assert!(matches!(
        requests.first().and_then(|request| request.input.first()),
        Some(ModelInputItem::UserMessage { content })
            if content.iter().any(|content| matches!(content, hanafubuki_agent_model_api::ModelUserContent::InputText { text } if text.contains("semanticChanges: 1")))
    ));
    drop(requests);
    assert!(fs::read_to_string(workspace.join("MEMORY.md"))
        .unwrap()
        .contains("Source: 7"));
    assert!(fs::read_to_string(workspace.join("memory_summary.md"))
        .unwrap()
        .starts_with("v1\n"));
    assert_eq!(
        fs::read_to_string(workspace.join("raw_memories.md")).unwrap(),
        "durable fact from session 7"
    );
}

#[test]
fn restricted_agent_rejects_writes_to_raw_evidence() {
    let temporary = TempDir::new().unwrap();
    let mut written = BTreeSet::new();
    assert!(execute_workspace_tool(
        temporary.path(),
        "memory_write_file",
        json!({"path":"raw_memories.md","content":"overwrite"}),
        &[],
        &mut written,
    )
    .is_err());
}

#[test]
fn writes_are_bounded_by_what_publish_accepts() {
    let temporary = TempDir::new().unwrap();
    let mut written = BTreeSet::new();
    let mut write = |path: &str, content: String| {
        execute_workspace_tool(
            temporary.path(),
            "memory_write_file",
            json!({"path": path, "content": content}),
            &[],
            &mut written,
        )
    };
    let summary_at_limit = format!("v1\n{}", "s".repeat(MAX_MEMORY_SUMMARY_BYTES - 3));
    assert!(write("memory_summary.md", summary_at_limit.clone()).is_ok());
    let error = write("memory_summary.md", format!("{summary_at_limit}s")).unwrap_err();
    assert!(
        error.contains(&MAX_MEMORY_SUMMARY_BYTES.to_string()),
        "{error}"
    );
    assert!(write("memory_summary.md", "no version line\n".into()).is_err());

    let memory_at_limit = "m".repeat(MAX_MEMORY_RESOURCE_BYTES);
    assert!(write("MEMORY.md", memory_at_limit.clone()).is_ok());
    assert!(write("MEMORY.md", format!("{memory_at_limit}m")).is_err());
    assert_eq!(
        fs::read_to_string(temporary.path().join("MEMORY.md"))
            .unwrap()
            .len(),
        MAX_MEMORY_RESOURCE_BYTES
    );
}
