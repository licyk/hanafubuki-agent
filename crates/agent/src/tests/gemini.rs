//! Native Gemini over HTTP, including real file tools and durable session history.
use super::*;
use serde_json::Value;

const SIGNATURE: &str = "opaque-test-signature";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generate_content_runs_parallel_tools_and_resumes_local_history() {
    exercise(AgentModelProtocol::GeminiGenerateContent).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactions_runs_parallel_tools_and_resumes_local_history() {
    exercise(AgentModelProtocol::GeminiInteractions).await;
}

async fn exercise(protocol: AgentModelProtocol) {
    let generate = protocol == AgentModelProtocol::GeminiGenerateContent;
    let answer = |text: &str| {
        if generate {
            gemini_content_events(json!({"text":text}))
        } else {
            gemini_interaction_events(
                json!({"type":"model_output","content":[]}),
                json!({"type":"text","text":text}),
                "completed",
            )
        }
    };
    let server = FakeServer::start(
        [
            calls(generate),
            answer("Read."),
            answer("Restored."),
            answer("A file was read."),
            answer("Continued."),
        ]
        .into_iter()
        .map(|stream| (stream, Duration::ZERO))
        .collect(),
    )
    .await;
    let other = FakeServer::start(vec![(assistant_events("Switched."), Duration::ZERO)]).await;
    let (service, paths, config) = service_with_config(
        if generate {
            "gemini-generate-session"
        } else {
            "gemini-interactions-session"
        },
        |_, config| {
            configure_test_provider(config, &server.base_url, None);
            add_test_provider(config, "provider-2", "Other", &other.base_url);
            let revision = config.providers().unwrap().revision;
            config
                .mutate_providers(revision, |registry| {
                    registry.providers[0].protocol = protocol;
                    registry.providers[0].api_token = Some("local-fixture-key".into());
                    registry.model_policy.model = "gemini-3-test".into();
                    Ok(())
                })
                .unwrap();
        },
    );
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("first.txt"), "content-from-file").unwrap();
    let session = service
        .create_session_with_workspace(
            AgentSessionMode::Default,
            None,
            workspace.path().display().to_string(),
        )
        .await
        .unwrap();
    let input = AgentTurnInput {
        images: vec![super::model_vision::staged(&service, session.id)],
        ..AgentTurnInput::from("Inspect these files and image.")
    };
    assert_eq!(
        service
            .start_turn(session.id, input)
            .await
            .unwrap()
            .final_answer,
        "Read."
    );

    let first_requests = server.requests();
    assert_eq!(first_requests.len(), 2);
    for request in &first_requests {
        assert_eq!(request.headers["x-goog-api-key"], "local-fixture-key");
        assert!(request.path.ends_with(if generate {
            "/models/gemini-3-test:streamGenerateContent?alt=sse"
        } else {
            "/interactions"
        }));
    }
    let body = |r: &CapturedRequest| serde_json::from_slice::<Value>(&r.body).unwrap();
    assert!(body(&first_requests[0]).to_string().contains("image/png"));
    let follow_up = body(&first_requests[1]);
    assert!(follow_up.to_string().contains(SIGNATURE));
    if generate {
        let parts: Vec<&Value> = follow_up["contents"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|message| message["parts"].as_array().unwrap())
            .collect();
        let calls: Vec<_> = parts
            .iter()
            .filter(|p| p.get("functionCall").is_some())
            .collect();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["thoughtSignature"], SIGNATURE);
        assert!(calls[0]["functionCall"].get("id").is_none());
        let results: Vec<_> = parts
            .iter()
            .filter_map(|p| p.get("functionResponse"))
            .collect();
        assert_eq!(results.len(), 2);
        assert!(results[0]["response"]["output"]
            .to_string()
            .contains("content-from-file"));
        assert!(results[1]["response"].get("error").is_some());
        assert!(results.iter().all(|r| r.get("id").is_none()));
    } else {
        assert_eq!(follow_up["store"], false);
        assert!(follow_up.get("previous_interaction_id").is_none());
        let steps = follow_up["input"].as_array().unwrap();
        let results: Vec<_> = steps
            .iter()
            .filter(|s| s["type"] == "function_result")
            .collect();
        assert_eq!(results.len(), 2);
        let result = |id: &str| *results.iter().find(|r| r["call_id"] == id).unwrap();
        assert!(result("read-a")["result"]
            .to_string()
            .contains("content-from-file"));
        assert_eq!(result("read-b")["is_error"], true);
    }
    let snapshot = service.snapshot(session.id).unwrap();
    let fork = service
        .fork_session_at_head(AgentSessionHeadForkRequest {
            source_session_id: session.id,
            source_generation: snapshot.generation,
            source_revision: snapshot.revision,
        })
        .unwrap()
        .session;
    super::model_reasoning::stop(&service);
    let service = super::model_reasoning::reopen(config, &paths);
    assert_eq!(
        service
            .start_turn(fork.id, "Resume.".into())
            .await
            .unwrap()
            .final_answer,
        "Restored."
    );
    assert!(body(&server.requests()[2]).to_string().contains(SIGNATURE));
    let fork = service.snapshot(fork.id).unwrap();
    service
        .set_session_model(AgentSessionModelSelectionRequest {
            session_id: fork.id,
            generation: fork.generation,
            revision: fork.revision,
            provider_id: "provider-2".into(),
            model: "other".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        service
            .start_turn(fork.id, "Switch.".into())
            .await
            .unwrap()
            .final_answer,
        "Switched."
    );
    assert!(!body(&other.requests()[0]).to_string().contains(SIGNATURE));

    let original = service.open_session(session.id, 1).unwrap().runtime;
    service
        .compact_session(original.id, original.generation, original.revision)
        .await
        .unwrap();
    assert_eq!(
        service
            .start_turn(original.id, "Continue.".into())
            .await
            .unwrap()
            .final_answer,
        "Continued."
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 5);
    let compacted = body(&requests[4]).to_string();
    assert!(compacted.contains("A file was read."));
    assert!(!compacted.contains(SIGNATURE));
    super::model_reasoning::stop(&service);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

fn calls(generate: bool) -> Vec<u8> {
    let args = |path| json!({"path":path,"offset":0,"maxBytes":128});
    let a = args("first.txt");
    let b = args("missing.txt");
    let frames = if generate {
        vec![
            json!({"responseId":"calls","candidates":[{"index":0,"content":{"role":"model","parts":[
            {"thoughtSignature":SIGNATURE,"functionCall":{"name":"read_file","args":a}},{"functionCall":{"name":"read_file","args":b}}
        ]},"finishReason":"STOP"}]}),
        ]
    } else {
        let mut frames = vec![
            json!({"event_type":"interaction.created","interaction":{"id":"calls","status":"in_progress"}}),
            json!({"event_type":"step.start","index":0,"step":{"type":"thought","signature":SIGNATURE,"summary":[{"type":"text","text":"Read both."}]}}),
            json!({"event_type":"step.stop","index":0}),
        ];
        for (i, (id, args)) in [("read-a", a), ("read-b", b)].into_iter().enumerate() {
            let index = i + 1;
            frames.push(json!({"event_type":"step.start","index":index,"step":{"type":"function_call","id":id,"name":"read_file"}}));
            frames.push(json!({"event_type":"step.delta","index":index,"delta":{"type":"arguments_delta","arguments":args.to_string()}}));
        }
        frames.extend([json!({"event_type":"step.stop","index":2}),json!({"event_type":"step.stop","index":1}),
            json!({"event_type":"interaction.completed","interaction":{"id":"calls","status":"requires_action"}})]);
        frames
    };
    frames
        .into_iter()
        .map(|frame| format!("data: {frame}\n\n"))
        .collect::<String>()
        .into_bytes()
}
