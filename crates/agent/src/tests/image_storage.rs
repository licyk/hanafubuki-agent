use super::*;
use base64::Engine as _;
use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_history::{
    AgentHistoryPaths, FileAgentHistoryStore, NoopAgentHistoryRefusalSink,
};
use hanafubuki_agent_kernel::test_support::{FakeRegisteredExchange, FakeTools};
use hanafubuki_agent_kernel::ModelApiProtocol;
use hanafubuki_agent_kernel::{
    AgentHistoryStore, AgentService, AgentSessionConfig, AgentSessionHeadForkRequest,
    ConversationItem,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_tool_images_use_one_blob_and_survive_fork_and_index_rebuild() {
    let bytes = fixtures::image_bytes();
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let output = hanafubuki_agent_mcp::convert_tool_output(
        serde_json::from_value(json!({
            "content":[
                {"type":"text","text":"before image"},
                {"type":"image","mimeType":"image/png","data":encoded},
                {"type":"text","text":"after image"}
            ],"isError":false
        }))
        .unwrap(),
    )
    .unwrap();
    let server = FakeServer::start(vec![
        (tool_events("list_instances", "{}"), Duration::ZERO),
        (assistant_events("First image."), Duration::ZERO),
        (tool_events("list_instances", "{}"), Duration::ZERO),
        (assistant_events("Repeated image."), Duration::ZERO),
        (assistant_events("Restored image."), Duration::ZERO),
    ])
    .await;
    let tools = FakeTools::new([
        FakeRegisteredExchange::success("list_instances", json!({}), output.clone()),
        FakeRegisteredExchange::success("list_instances", json!({}), output),
    ])
    .await;
    let temporary = tempfile::tempdir().unwrap();
    let paths = AgentHistoryPaths::under(temporary.path().to_path_buf());
    let workers = BackgroundWorkerGroup::default();
    let store = Arc::new(FileAgentHistoryStore::open(paths.clone(), workers.clone()).unwrap());
    let core =
        AgentService::without_default_model_with_history_store(tools.registries(), store.clone());
    let session = core
        .create_model_session(
            fixtures::config(),
            ModelApiProtocol::Responses,
            &server.base_url,
            None,
            Vec::new(),
            Default::default(),
            ModelCompactionCapability::LocalSummary,
            Default::default(),
        )
        .unwrap();
    for _ in 0..2 {
        Box::pin(core.start_turn(session.id, "Show the image."))
            .await
            .unwrap();
    }
    let history = store.load_latest_model_context(session.id).unwrap();
    let images: Vec<_> = history
        .iter()
        .filter_map(|item| match &item.content {
            ConversationItem::ToolOutput { images, .. } => images.first(),
            _ => None,
        })
        .cloned()
        .collect();
    assert_eq!(images.len(), 2);
    assert_eq!(images[0], images[1]);
    assert_eq!(store.load_image_attachment(&images[0]).unwrap(), bytes);
    let display = core.tool_display(session.id, "call-tool").unwrap().unwrap();
    let Some(hanafubuki_agent_contracts::AgentToolDisplay::Content { blocks }) = display.display
    else {
        panic!("expected content display")
    };
    assert!(
        matches!(blocks[1], hanafubuki_agent_contracts::AgentOutputBlock::Image { bytes: count, .. } if count == bytes.len() as u64)
    );
    let snapshot = core.snapshot(session.id).unwrap();
    let child = core
        .fork_session_at_head(AgentSessionHeadForkRequest {
            source_session_id: session.id,
            source_generation: snapshot.generation,
            source_revision: snapshot.revision,
        })
        .unwrap()
        .session;
    fixtures::stop(core, workers).await;
    drop(store);
    let files = fixtures::files(&paths.root);
    let blobs: Vec<_> = files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "blob"))
        .collect();
    assert_eq!(blobs.len(), 1);
    assert_eq!(std::fs::read(blobs[0]).unwrap(), bytes);
    for file in files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
    {
        let contents = std::fs::read(file).unwrap();
        assert!(
            !contents
                .windows(encoded.len())
                .any(|part| part == encoded.as_bytes()),
            "inline payload in {}",
            file.display()
        );
    }
    {
        let connection = rusqlite::Connection::open(&paths.sqlite_database).unwrap();
        let mut statement = connection
            .prepare("SELECT item_json FROM agent_history_items")
            .unwrap();
        for row in statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
        {
            assert!(
                !row.unwrap().contains(&encoded),
                "inline payload in SQLite projection"
            );
        }
    }
    let rollout_bytes: u64 = files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .map(|p| std::fs::metadata(p).unwrap().len())
        .sum();
    println!(
        "IMAGE_STORAGE image_bytes={} image_occurrences={} blob_files={} rollout_bytes={}",
        bytes.len(),
        images.len(),
        blobs.len(),
        rollout_bytes
    );
    let workers = BackgroundWorkerGroup::default();
    let store = Arc::new(
        FileAgentHistoryStore::open_rebuilding_index(
            paths.clone(),
            workers.clone(),
            Arc::new(NoopAgentHistoryRefusalSink),
        )
        .unwrap(),
    );
    store
        .cleanup_unreferenced_image_attachments(session.id)
        .unwrap();
    assert_eq!(
        store
            .load_visible_image_attachment(child.id, &images[0])
            .unwrap(),
        bytes
    );
    let core =
        AgentService::without_default_model_with_history_store(tools.registries(), store.clone());
    assert!(core.recover_sessions().unwrap().quarantined.is_empty());
    core.activate_model_session(
        ModelApiProtocol::Responses,
        &server.base_url,
        None,
        Vec::new(),
        Default::default(),
        ModelCompactionCapability::LocalSummary,
        Default::default(),
        child.id,
    )
    .unwrap();
    Box::pin(core.start_turn(child.id, "Continue with the inherited images."))
        .await
        .unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 5);
    for (index, image_count) in [(1, 1), (3, 2), (4, 2)] {
        assert_eq!(
            String::from_utf8_lossy(&requests[index].body)
                .matches(&encoded)
                .count(),
            image_count
        );
    }
    store
        .delete_sessions_confirmed(&[child.id, session.id])
        .unwrap();
    assert!(!fixtures::files(&paths.attachments)
        .iter()
        .any(|p| p.extension().is_some_and(|e| e == "blob")));
    fixtures::stop(core, workers).await;
}

mod fixtures {
    use super::*;

    pub(super) fn config() -> AgentSessionConfig {
        AgentSessionConfig {
            provider_id: "test".into(),
            provider_protocol: ModelApiProtocol::Responses,
            model: "fixture-model".into(),
            instructions: "Inspect images.".into(),
            mode: AgentSessionMode::Default,
            reasoning_effort: None,
            context_window_tokens: None,
            input_capabilities: Default::default(),
            auto_compaction_enabled: true,
            auto_compact_threshold_percent: 80,
            compaction_capability: ModelCompactionCapability::LocalSummary,
            sampling_retry_policy: Default::default(),
            context_window_source: None,
            compaction_strategy: Default::default(),
            scope: Default::default(),
            configured_facts: Default::default(),
            role_instructions: None,
        }
    }

    pub(super) fn image_bytes() -> Vec<u8> {
        if let Ok(path) = std::env::var("HANAFUBUKI_TEST_TOOL_IMAGE") {
            return std::fs::read(path).unwrap();
        }
        let mut output = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(32, 32)
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        output.into_inner()
    }

    pub(super) fn files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut found = Vec::new();
        if !root.exists() {
            return found;
        }
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                found.extend(files(&path));
            } else {
                found.push(path);
            }
        }
        found
    }

    pub(super) async fn stop(core: AgentService, workers: BackgroundWorkerGroup) {
        tokio::task::spawn_blocking(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            assert!(core.shutdown(deadline).is_empty());
            assert!(workers.shutdown(deadline).is_empty());
        })
        .await
        .unwrap();
    }
}
