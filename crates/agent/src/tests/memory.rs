use super::*;
use serde_json::Value;

#[test]
fn memory_stream_filter_withholds_partial_and_complete_internal_citation_blocks() {
    let answer = "visible answer";
    assert_eq!(
        super::super::memory::memory_stream_visible_prefix_len("visible answer "),
        answer.len()
    );
    assert_eq!(
        super::super::memory::memory_stream_visible_prefix_len(
            "visible answer\n<hanafubuki-memory"
        ),
        answer.len()
    );
    assert_eq!(
        super::super::memory::memory_stream_visible_prefix_len(
            "visible answer\n<hanafubuki-memory-citations>[{}]</hanafubuki-memory-citations>"
        ),
        answer.len()
    );
    assert_eq!(
        super::super::memory::memory_stream_visible_prefix_len("visible answer continued"),
        "visible answer continued".len()
    );
}

/// The note tool writes on behalf of one exact turn, and it is offered whenever
/// the session may contribute at all. Which words the user chose is a judgement
/// for the model working from its instructions, not something Rust can decide by
/// matching prefixes.
#[tokio::test]
async fn the_memory_note_tool_is_bound_to_the_turn_that_planned_it() {
    use super::super::plugins::memory_tools;
    use hanafubuki_agent_contracts::{
        NoopAgentToolProgressSink, ToolCallContext, ToolEffect, ToolInvocation, ToolSourceStep,
    };

    let (service, paths) = service_with("explicit-memory-note-tool", |_, _| {});
    let memory = service.test_memory();
    let session_id: hanafubuki_agent_kernel::AgentSessionId =
        serde_json::from_value(json!(77)).unwrap();
    memory
        .repository()
        .sync_source_state(
            &session_id.to_string(),
            hanafubuki_agent_memory::AgentMemoryContributionMode::Enabled,
            1,
            9,
        )
        .unwrap();
    let turn_id: hanafubuki_agent_kernel::AgentTurnId = serde_json::from_value(json!(3)).unwrap();
    let generation: hanafubuki_agent_kernel::AgentGeneration =
        serde_json::from_value(json!(4)).unwrap();
    let session_revision: hanafubuki_agent_kernel::AgentRevision =
        serde_json::from_value(json!(12)).unwrap();
    let step = ToolSourceStep {
        session_id,
        turn_id,
        generation,
        session_revision,
        memory_contribution: true,
        cancellation: CancellationToken::default(),
    };
    // A session that cannot contribute is never offered the note tool.
    let silent_step = ToolSourceStep {
        memory_contribution: false,
        ..step.clone()
    };
    let silent = memory_tools(memory.clone(), true, true, &silent_step).unwrap();
    assert!(silent
        .iter()
        .all(|tool| tool.handler.descriptor().name.local_name() != "append_agent_memory_note"));

    let tools = memory_tools(memory.clone(), true, true, &step).unwrap();
    let tool = tools
        .iter()
        .find(|tool| tool.handler.descriptor().name.local_name() == "append_agent_memory_note")
        .unwrap();
    let descriptor = tool.handler.descriptor();
    assert_eq!(
        descriptor.execution_class,
        hanafubuki_agent_kernel::AgentToolExecutionClass::Exclusive
    );
    // It writes, so Plan mode never offers it; the read tools it does.
    assert_eq!(descriptor.effect, ToolEffect::Mutating);
    assert!(tools
        .iter()
        .filter(|tool| tool.handler.descriptor().name.local_name() != "append_agent_memory_note")
        .all(|tool| tool.handler.descriptor().effect == ToolEffect::ReadOnly));
    assert!(hanafubuki_agent_kernel::validate_dynamic_tool_arguments(
        &descriptor.input_schema,
        &json!({"operation":"remember","content":"Prefer narrow tests.","path":"MEMORY.md"})
    )
    .is_err());

    let invocation = |turn_id: hanafubuki_agent_kernel::AgentTurnId| ToolInvocation {
        context: ToolCallContext {
            session_id,
            turn_id,
            generation,
            mode: hanafubuki_agent_kernel::AgentSessionMode::Default,
            session_revision,
            call_id: "memory-note-call".to_string(),
            image_input: false,
        },
        arguments: json!({"operation":"remember","content":"Prefer narrow tests."}),
        cancellation: CancellationToken::default(),
        progress: Arc::new(NoopAgentToolProgressSink),
        plan_leases: Vec::new(),
        turn: Default::default(),
    };
    let handler = tool.handler.clone();
    // The tool was planned for one turn and cannot write for another.
    let other_turn = invocation(serde_json::from_value(json!(4)).unwrap());
    assert!(handler.permission(&other_turn).await.is_err());
    assert!(handler.invoke(other_turn, None).await.is_err());
    let output = handler.invoke(invocation(turn_id), None).await.unwrap();
    assert_eq!(
        output
            .output
            .structured_content
            .as_ref()
            .and_then(|value| value.get("status"))
            .and_then(Value::as_str),
        Some("queued_for_consolidation")
    );
    assert_eq!(
        std::fs::read_to_string(&memory.repository().paths().ad_hoc_notes)
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(handler.invoke(invocation(turn_id), None).await.is_err());

    assert!(service
        .shutdown(Instant::now() + Duration::from_secs(1))
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The regression this whole refactor exists for: a memory the user can see in
/// the artifacts folder must reach the model, together with the tools that read
/// the rest of it. Neither used to happen until a consolidation pass had
/// published a generation and no newer source was waiting for the next one.
#[cfg(unix)]
#[tokio::test]
async fn published_memory_reaches_the_model_with_the_tools_that_read_it() {
    let server = FakeServer::start(vec![(assistant_events("done"), Duration::ZERO)]).await;
    let (service, paths) = service_with("memory-reaches-the-model", |_, config| {
        configure_test_provider(config, &server.base_url, None);
        config
            .write_settings(AgentSettingsPatch {
                memory: Some(hanafubuki_agent_config::AgentMemorySettings {
                    enabled: true,
                    generate_memories: true,
                    ..Default::default()
                }),
                ..AgentSettingsPatch::default()
            })
            .unwrap();
    });

    // Memory as a user actually has it: published files, and a source waiting
    // for a consolidation pass that has not run.
    let memory = service.test_memory();
    let artifacts = &memory.repository().paths().artifacts;
    std::fs::write(
        artifacts.join("memory_summary.md"),
        "v1\n\n- Validation preferences: MEMORY.md\n",
    )
    .unwrap();
    std::fs::write(
        artifacts.join("MEMORY.md"),
        "# Memory\n\nThe user prefers narrow validation runs.\n",
    )
    .unwrap();
    assert_eq!(memory.repository().generation().unwrap(), 0);

    let session = service
        .create_session(hanafubuki_agent_kernel::AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .start_turn(session.id, "How should I run validation?".into())
        .await
        .unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let body = String::from_utf8(requests[0].body.clone()).unwrap();

    assert!(body.contains("Validation preferences"));
    assert!(body.contains("MEMORY_SUMMARY BEGINS"));
    for tool in [
        "list_agent_memories",
        "search_agent_memories",
        "read_agent_memory",
        "append_agent_memory_note",
    ] {
        assert!(body.contains(tool), "{tool} was not offered to the model");
    }
    // No generation handshake survives in the tool contract.
    assert!(!body.contains("\"generation\""));

    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Pins, byte for byte, the memory context the model receives: a developer
/// item right after the Skill catalog, which follows the world state,
/// carrying the memory instructions and the published summary. A change here
/// is a change of the model's input.
#[cfg(unix)]
#[tokio::test]
async fn the_memory_context_item_is_pinned_right_after_the_skill_catalog() {
    use sha2::{Digest, Sha256};

    let server = FakeServer::start(vec![(assistant_events("done"), Duration::ZERO)]).await;
    let (service, paths) = service_with("memory-pinned-fragment", |_, config| {
        configure_test_provider(config, &server.base_url, None);
        enable_memory(config, true);
    });
    enable_two_skills(&service, &paths);
    let memory = service.test_memory();
    std::fs::write(
        memory
            .repository()
            .paths()
            .artifacts
            .join("memory_summary.md"),
        "v1\n\n- Validation preferences: MEMORY.md\n",
    )
    .unwrap();
    let session = service
        .create_session(hanafubuki_agent_kernel::AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .start_turn(session.id, "How should I run validation?".into())
        .await
        .unwrap();

    let request: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    let input = request["input"].as_array().unwrap();
    let text = |item: &Value| item["content"][0]["text"].as_str().unwrap().to_string();
    let catalog = input
        .iter()
        .position(|item| text(item).starts_with("[Application-provided Skill capability catalog."))
        .unwrap();
    assert!(catalog > 0);
    assert!(input[..catalog]
        .iter()
        .all(|item| text(item).contains("<agent_world_state kind=")));
    let memory_item = &input[catalog + 1];
    assert_eq!(memory_item["role"], "developer");
    assert!(text(memory_item).ends_with(
        "========= MEMORY_SUMMARY BEGINS =========\nv1\n\n- Validation preferences: MEMORY.md\n========= MEMORY_SUMMARY ENDS ========="
    ));
    assert_eq!(
        hex::encode(Sha256::digest(memory_item.to_string().as_bytes())),
        "2fa46ae001946652e89ef8da6c7da386bf0ae0352fff546842669cae5769110f"
    );
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Publishes generation 1 of memory with one source, session `source`.
fn publish_memory_from(repository: &hanafubuki_agent_memory::AgentMemoryRepository, source: &str) {
    repository
        .sync_source_state(
            source,
            hanafubuki_agent_memory::AgentMemoryContributionMode::Enabled,
            1,
            8,
        )
        .unwrap();
    let lease = repository
        .claim_source(source, 8, 100, 1_000)
        .unwrap()
        .unwrap();
    repository
        .complete_source(
            &lease,
            Some(hanafubuki_agent_memory::StageOneResult {
                raw_memory: "user prefers narrow validation".into(),
                rollout_summary: "summary for narrow validation".into(),
                rollout_slug: "narrow-validation".into(),
            }),
            101,
        )
        .unwrap();
    let consolidation = repository.claim_consolidation(102, 1_000).unwrap().unwrap();
    let inputs = repository.consolidation_inputs(2).unwrap();
    let workspace = repository
        .prepare_consolidation_workspace(consolidation.source_revision + 1, &inputs)
        .unwrap();
    std::fs::write(
        workspace.path.join("MEMORY.md"),
        format!("# Memory\n\nUser prefers narrow validation. Source: {source}.\n"),
    )
    .unwrap();
    std::fs::write(
        workspace.path.join("memory_summary.md"),
        "v1\n\n- Validation preferences: MEMORY.md\n",
    )
    .unwrap();
    assert_eq!(
        repository
            .publish_consolidation_workspace(&consolidation, &workspace, 103)
            .unwrap(),
        1
    );
}

/// How often the memory of source session `source` was cited.
fn citation_usage(
    repository: &hanafubuki_agent_memory::AgentMemoryRepository,
    source: &str,
) -> i64 {
    rusqlite::Connection::open(&repository.paths().database)
        .unwrap()
        .query_row(
            "SELECT usage_count FROM memory_stage1 WHERE session_id=?1",
            [source],
            |row| row.get(0),
        )
        .unwrap()
}

/// An answer that cites memory it was served: the citation block never
/// reaches the visible answer, the committed message keeps the validated
/// citations, and the memory store records their use.
#[cfg(unix)]
#[tokio::test]
async fn a_cited_memory_answer_is_stripped_and_its_usage_recorded() {
    let answer = "Run the narrow suite.\n<hanafubuki-memory-citations>[{\"resourceId\":\"MEMORY.md\",\"note\":\"validation preference\",\"sourceSessionIds\":[\"9\"]}]</hanafubuki-memory-citations>";
    let server = FakeServer::start(vec![(assistant_events(answer), Duration::ZERO)]).await;
    let (service, paths) = service_with("memory-cited-answer", |_, config| {
        configure_test_provider(config, &server.base_url, None);
        enable_memory(config, false);
    });
    let repository = service.test_memory().repository().clone();
    publish_memory_from(&repository, "9");
    assert_eq!(citation_usage(&repository, "9"), 0);

    let session = service
        .create_session(hanafubuki_agent_kernel::AgentSessionMode::Default)
        .await
        .unwrap();
    let outcome = service
        .start_turn(session.id, "How should I run validation?".into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Run the narrow suite.");
    let body = String::from_utf8(server.requests()[0].body.clone()).unwrap();
    assert!(body.contains("MEMORY_SUMMARY BEGINS"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while citation_usage(&repository, "9") == 0 {
        assert!(
            Instant::now() < deadline,
            "the citation usage was never recorded"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(citation_usage(&repository, "9"), 1);

    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Turning the `memory` plugin off in the plugin manager stops everything it
/// does: the next request carries neither the memory context nor its tools,
/// and turning it back on restores both.
#[cfg(unix)]
#[tokio::test]
async fn turning_the_memory_plugin_off_removes_memory_from_the_next_request() {
    use hanafubuki_agent_contracts::RegistryService as _;

    let server = FakeServer::start(vec![
        (assistant_events("first"), Duration::ZERO),
        (assistant_events("second"), Duration::ZERO),
        (assistant_events("third"), Duration::ZERO),
    ])
    .await;
    let (service, paths) = service_with("memory-plugin-disabled", |_, config| {
        configure_test_provider(config, &server.base_url, None);
        enable_memory(config, false);
    });
    std::fs::write(
        service
            .test_memory()
            .repository()
            .paths()
            .artifacts
            .join("memory_summary.md"),
        "v1\n\n- Validation preferences: MEMORY.md\n",
    )
    .unwrap();
    service.start_plugins().await;
    let registries = service.test_plugin_registries();
    let memory_prompts = || {
        registries
            .prompt
            .registry()
            .snapshot()
            .entries()
            .iter()
            .filter(|entry| entry.owner() == "memory")
            .count()
    };
    let session = service
        .create_session(hanafubuki_agent_kernel::AgentSessionMode::Default)
        .await
        .unwrap();
    let request_body =
        |index: usize| String::from_utf8(server.requests()[index].body.clone()).unwrap();
    let carries_memory =
        |body: &str| body.contains("MEMORY_SUMMARY BEGINS") && body.contains("list_agent_memories");

    service
        .start_turn(session.id, "How should I run validation?".into())
        .await
        .unwrap();
    assert!(carries_memory(&request_body(0)));

    service
        .set_plugin_enabled("memory".into(), false)
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while memory_prompts() != 0 {
        assert!(Instant::now() < deadline, "the memory plugin never stopped");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    service
        .start_turn(session.id, "And now?".into())
        .await
        .unwrap();
    let body = request_body(1);
    assert!(!body.contains("MEMORY_SUMMARY BEGINS"));
    assert!(!body.contains("Validation preferences: MEMORY.md"));
    assert!(!body.contains("list_agent_memories"));

    service
        .set_plugin_enabled("memory".into(), true)
        .await
        .unwrap();
    while memory_prompts() != 1 {
        assert!(
            Instant::now() < deadline,
            "the memory plugin never restarted"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    service
        .start_turn(session.id, "Once more?".into())
        .await
        .unwrap();
    assert!(carries_memory(&request_body(2)));

    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Retrieval and generation are separate switches, and turning retrieval off
/// must remove both the context and the tools that reach it.
#[cfg(unix)]
#[tokio::test]
async fn disabling_retrieval_withholds_memory_and_its_read_tools() {
    let server = FakeServer::start(vec![(assistant_events("done"), Duration::ZERO)]).await;
    let (service, paths) = service_with("memory-retrieval-disabled", |_, config| {
        configure_test_provider(config, &server.base_url, None);
        config
            .write_settings(AgentSettingsPatch {
                memory: Some(hanafubuki_agent_config::AgentMemorySettings {
                    enabled: true,
                    use_memories: false,
                    generate_memories: false,
                    ..Default::default()
                }),
                ..AgentSettingsPatch::default()
            })
            .unwrap();
    });
    let memory = service.test_memory();
    std::fs::write(
        memory
            .repository()
            .paths()
            .artifacts
            .join("memory_summary.md"),
        "v1\n\n- Validation preferences: MEMORY.md\n",
    )
    .unwrap();

    let session = service
        .create_session(hanafubuki_agent_kernel::AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .start_turn(session.id, "How should I run validation?".into())
        .await
        .unwrap();
    let body = String::from_utf8(server.requests()[0].body.clone()).unwrap();
    assert!(!body.contains("Validation preferences"));
    assert!(!body.contains("list_agent_memories"));
    assert!(!body.contains("append_agent_memory_note"));

    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn the_memory_plugin_registers_its_prompt_tool_source_and_observer_until_it_stops() {
    use super::super::plugins::memory_plugin;
    use crate::test_support::mount;
    use hanafubuki_agent_contracts::RegistryService as _;

    let (service, paths) = service_with("memory-plugin-lifecycle", |_, _| {});
    let mounted = mount(memory_plugin(service.test_memory())).await;
    let sources = mounted.sources.registry().snapshot();
    assert_eq!(
        sources
            .items()
            .map(|source| source.id().to_string())
            .collect::<Vec<_>>(),
        ["memory"]
    );
    assert!(mounted.tools.registry().snapshot().is_empty());
    assert_eq!(
        mounted
            .prompt
            .registry()
            .snapshot()
            .items()
            .map(|contributor| contributor.id().to_string())
            .collect::<Vec<_>>(),
        ["memory"]
    );
    assert_eq!(
        mounted
            .observers
            .registry()
            .snapshot()
            .items()
            .map(|observer| observer.id().to_string())
            .collect::<Vec<_>>(),
        ["memory"]
    );
    assert_eq!(
        mounted
            .renderers
            .registry()
            .snapshot()
            .items()
            .map(|renderer| renderer.kind.as_str().to_string())
            .collect::<Vec<_>>(),
        ["agent_memory"]
    );

    mounted.fiber.dispose().await.unwrap();
    assert!(mounted.sources.registry().snapshot().is_empty());
    assert!(mounted.prompt.registry().snapshot().is_empty());
    assert!(mounted.renderers.registry().snapshot().is_empty());
    assert!(mounted.observers.registry().snapshot().is_empty());
    mounted.root.fiber().dispose().await.unwrap();
    assert!(service
        .shutdown(Instant::now() + Duration::from_secs(1))
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The kernel commits a session's memory contribution; the memory plugin's
/// observer records it in the memory store before the toggle returns.
#[tokio::test]
async fn a_committed_memory_contribution_reaches_the_memory_store() {
    let server = FakeServer::start(Vec::new()).await;
    let (service, paths) = configured_service("memory-contribution-observer", &server.base_url);
    let session = service
        .create_session(hanafubuki_agent_kernel::AgentSessionMode::Default)
        .await
        .unwrap();
    let repository = service.test_memory().repository().clone();
    let source = session.id.to_string();
    assert!(repository.contribution_mode(&source).is_err());

    let (mode, revision) = service
        .set_memory_session_contribution(session.id, session.memory_contribution.revision, true)
        .unwrap();
    assert_eq!(
        mode,
        hanafubuki_agent_memory::AgentMemoryContributionMode::Enabled
    );
    assert_eq!(
        repository.contribution_mode(&source).unwrap(),
        (mode, revision)
    );

    service
        .set_memory_session_contribution(session.id, revision, false)
        .unwrap();
    assert_eq!(
        repository.contribution_mode(&source).unwrap().0,
        hanafubuki_agent_memory::AgentMemoryContributionMode::Disabled
    );

    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Archives and restores a new session and turns its memory contribution on
/// and off, and returns how many memory scans each action signalled.
async fn scans_signalled_by_session_actions(name: &str, memory_plugin: bool) -> [usize; 4] {
    use hanafubuki_agent_contracts::RegistryService as _;

    let server = FakeServer::start(Vec::new()).await;
    let (service, paths) = configured_service(name, &server.base_url);
    service.start_plugins().await;
    if !memory_plugin {
        service
            .set_plugin_enabled("memory".into(), false)
            .await
            .unwrap();
        let observers = service.test_plugin_registries().observers;
        let deadline = Instant::now() + Duration::from_secs(5);
        while observers
            .registry()
            .snapshot()
            .entries()
            .iter()
            .any(|entry| entry.owner() == "memory")
        {
            assert!(Instant::now() < deadline, "the memory plugin never stopped");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    let session = service
        .create_session(hanafubuki_agent_kernel::AgentSessionMode::Default)
        .await
        .unwrap();
    let memory = service.test_memory();

    let before = memory.test_scan_signals();
    let archived = service
        .archive_session(session.id, session.generation, session.revision)
        .unwrap();
    let archive = memory.test_scan_signals() - before;
    let before = memory.test_scan_signals();
    service
        .restore_session(session.id, archived.generation, archived.revision)
        .unwrap();
    let restore = memory.test_scan_signals() - before;
    let before = memory.test_scan_signals();
    let (_, revision) = service
        .set_memory_session_contribution(session.id, session.memory_contribution.revision, true)
        .unwrap();
    let enable = memory.test_scan_signals() - before;
    let before = memory.test_scan_signals();
    service
        .set_memory_session_contribution(session.id, revision, false)
        .unwrap();
    let disable = memory.test_scan_signals() - before;

    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
    [archive, restore, enable, disable]
}

/// Archiving, restoring and the user's contribution toggle change what
/// memory may read, so each asks the memory plugin's observer for one scan.
#[tokio::test]
async fn session_actions_signal_a_memory_scan_through_the_memory_plugin() {
    assert_eq!(
        scans_signalled_by_session_actions("memory-scan-session-actions", true).await,
        [1, 1, 1, 1]
    );
}

/// A new or forked session starts contributing as the memory settings say,
/// recorded in the memory store, without a scan: it is picked up once idle,
/// and a scan now would compete with its first turn for the model.
#[tokio::test]
async fn creating_or_forking_a_contributing_session_signals_no_memory_scan() {
    let server = FakeServer::start(Vec::new()).await;
    let (service, paths) = service_with("memory-scan-session-creation", |_, config| {
        configure_test_provider(config, &server.base_url, None);
        enable_memory(config, true);
    });
    service.start_plugins().await;
    let memory = service.test_memory();
    let contributes = |session_id: hanafubuki_agent_kernel::AgentSessionId| {
        memory
            .repository()
            .contribution_mode(&session_id.to_string())
            .unwrap()
            .0
            == hanafubuki_agent_memory::AgentMemoryContributionMode::Enabled
    };

    let before = memory.test_scan_signals();
    let session = service
        .create_session(hanafubuki_agent_kernel::AgentSessionMode::Default)
        .await
        .unwrap();
    let fork = service
        .fork_session_at_head(hanafubuki_agent_kernel::AgentSessionHeadForkRequest {
            source_session_id: session.id,
            source_generation: session.generation,
            source_revision: session.revision,
        })
        .unwrap();

    assert!(contributes(session.id));
    assert!(contributes(fork.session.id));
    assert_eq!(memory.test_scan_signals(), before);
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn session_actions_signal_no_memory_scan_with_the_memory_plugin_off() {
    assert_eq!(
        scans_signalled_by_session_actions("memory-scan-plugin-off", false).await,
        [0, 0, 0, 0]
    );
}

/// Saving the settings brings the plugins up to date and asks for a memory
/// scan, so memory turned on from the settings page starts its first pass.
#[tokio::test]
async fn saving_the_settings_signals_a_memory_scan_and_persists_them() {
    let (service, paths) = service_with("memory-scan-settings-write", |_, _| {});
    let memory = service.test_memory();
    let before = memory.test_scan_signals();

    let view = service
        .write_settings(AgentSettingsPatch {
            memory: Some(hanafubuki_agent_config::AgentMemorySettings {
                enabled: true,
                ..Default::default()
            }),
            ..AgentSettingsPatch::default()
        })
        .await
        .unwrap();

    assert!(view.settings.memory.enabled);
    assert_eq!(memory.test_scan_signals() - before, 1);
    assert!(
        AgentConfig::new(&paths.config_dir)
            .settings()
            .unwrap()
            .memory
            .enabled
    );
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
