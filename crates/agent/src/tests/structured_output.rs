//! A recap on a gateway that refuses structured output: the chain falls to
//! the forced tool, then to plain text, and the route keeps the rung it
//! learned, so the next request starts there.

use hanafubuki_agent_kernel::AgentSessionMode;

use super::*;

const RECAP: &str =
    r#"{"summary":"Fixed the login timeout; not deployed yet.","next_action":"Deploy the fix?"}"#;

fn expected() -> AgentRecap {
    AgentRecap {
        summary: "Fixed the login timeout; not deployed yet.".to_string(),
        next_action: Some("Deploy the fix?".to_string()),
    }
}

/// A started service on `server` whose session has one finished,
/// programmatic turn (so no automatic title takes the server's answers).
async fn service_with_a_turn(
    name: &str,
    server: &FakeServer,
) -> (AgentApplicationService, TestPaths, AgentSessionId) {
    let (service, paths) = configured_service(name, &server.base_url);
    service.start_plugins().await;
    let session_id = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    service
        .start_turn(session_id, "Fix the login timeout".to_string().into())
        .await
        .unwrap();
    (service, paths, session_id)
}

fn finish(service: AgentApplicationService, paths: TestPaths) {
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

fn body(request: &CapturedRequest) -> serde_json::Value {
    serde_json::from_slice(&request.body).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn permission_review_on_a_stateless_responses_gateway_keeps_the_text_schema() {
    let arguments = json!({ "cmd": approved_command(), "yield_time_ms": 30_000 }).to_string();
    let assessment = json!({
        "riskLevel": "low",
        "userAuthorization": "explicit",
        "outcome": "allow",
        "rationale": "The user requested this command."
    })
    .to_string();
    let mut responses = Vec::new();
    for _ in 0..2 {
        responses.extend([
            (tool_events("exec_command", &arguments), Duration::ZERO),
            (assistant_events(&assessment), Duration::ZERO),
            (assistant_events("Ran it."), Duration::ZERO),
        ]);
    }
    let server = FakeServer::refusing(
        responses,
        GatewayRefusals {
            structured_output: true,
            tool_choice: false,
        },
    )
    .await;
    let (service, paths, _) = service_with_config("review-stateless-gateway", |_, config| {
        configure_test_provider(config, &server.base_url, None);
        config
            .write_settings(crate::AgentSettingsPatch {
                sandbox_mode: Some(Some(AgentSandboxMode::FullAccess)),
                ..Default::default()
            })
            .unwrap();
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .set_session_permission_mode(session.id, AgentPermissionMode::AutoReview)
        .unwrap();
    for _ in 0..2 {
        service
            .start_turn(session.id, "Run the command.".to_string().into())
            .await
            .unwrap();
        let request = body(server.requests().last().unwrap());
        let output = request["input"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|item| item["type"] == "function_call_output")
            .unwrap();
        assert!(output["output"]
            .as_str()
            .unwrap()
            .contains("approved-agent-command"));
    }
    let requests = server.requests();
    assert_eq!(requests.len(), 7, "only the first native review is refused");
    assert!(body(&requests[1])["text"]["format"].is_object());
    for index in [2, 5] {
        let request = body(&requests[index]);
        assert!(request.get("text").is_none());
        assert!(request["tools"].as_array().unwrap().is_empty());
        assert_eq!(request["tool_choice"], "none");
        let instructions = request["instructions"].as_str().unwrap();
        let schema: serde_json::Value =
            serde_json::from_str(&instructions[instructions.find('{').unwrap()..]).unwrap();
        assert_eq!(
            schema["properties"]["userAuthorization"]["enum"],
            json!(["explicit", "implicit", "absent", "conflicting"])
        );
        assert_eq!(
            schema["properties"]["outcome"]["enum"],
            json!(["allow", "deny"])
        );
        assert_eq!(schema["additionalProperties"], false);
    }
    finish(service, paths);
}

/// The reported gateway: `text.format` refused with a 400. The recap comes
/// through the forced tool, and the next one asks for it at once.
// The managed turn's task must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_recap_on_a_gateway_without_structured_output_comes_through_the_forced_tool() {
    let server = FakeServer::refusing(
        vec![
            (assistant_events("Fixed the timeout."), Duration::ZERO),
            (
                tool_events("submit_agent_conversation_recap", RECAP),
                Duration::ZERO,
            ),
            (
                tool_events("submit_agent_conversation_recap", RECAP),
                Duration::ZERO,
            ),
        ],
        GatewayRefusals {
            structured_output: true,
            tool_choice: false,
        },
    )
    .await;
    let (service, paths, session_id) = service_with_a_turn("gateway-tool", &server).await;

    assert_eq!(service.recap_session(session_id).await.unwrap(), expected());

    let requests = server.requests();
    assert_eq!(requests.len(), 3, "the turn, the refused one, the tool");
    assert!(body(&requests[1])["text"]["format"].is_object());
    let tool = body(&requests[2]);
    assert!(tool.get("text").is_none());
    assert_eq!(
        tool["tool_choice"]["name"],
        "submit_agent_conversation_recap"
    );

    assert_eq!(service.recap_session(session_id).await.unwrap(), expected());
    let requests = server.requests();
    assert_eq!(requests.len(), 4, "the learned rung first, nothing refused");
    assert_eq!(
        body(&requests[3])["tool_choice"]["name"],
        "submit_agent_conversation_recap"
    );
    finish(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_streamed_rejection_of_named_tool_choice_falls_back_to_a_text_title() {
    let error = json!({"error": {
        "code": "upstream_error",
        "message": "buddy: cannot unmarshal object into Go struct field Request.tool_choice of type string"
    }});
    let server = FakeServer::refusing(
        vec![
            (assistant_events("Fixed the timeout."), Duration::ZERO),
            (format!("data: {error}\n\n").into_bytes(), Duration::ZERO),
            (
                assistant_events(r#"{"title":"Fix login timeout"}"#),
                Duration::ZERO,
            ),
        ],
        GatewayRefusals {
            structured_output: true,
            tool_choice: false,
        },
    )
    .await;
    let (service, paths, session_id) = service_with_a_turn("title-streamed-refusal", &server).await;

    assert_eq!(
        service.suggest_session_title(session_id).await.unwrap(),
        "Fix login timeout"
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(
        body(&requests[2])["tool_choice"]["name"],
        "submit_agent_conversation_title"
    );
    assert_eq!(body(&requests[3])["tool_choice"], "none");
    finish(service, paths);
}

/// A gateway that refuses the forced tool too: the recap comes as text,
/// read tolerantly, and the next one is asked as text at once.
// The managed turn's task must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_recap_on_a_gateway_without_a_forced_tool_comes_through_text() {
    let fenced = format!("Here is the recap:\n```json\n{RECAP}\n```");
    let server = FakeServer::refusing(
        vec![
            (assistant_events("Fixed the timeout."), Duration::ZERO),
            (assistant_events(&fenced), Duration::ZERO),
            (assistant_events(RECAP), Duration::ZERO),
            (assistant_events(RECAP), Duration::ZERO),
        ],
        GatewayRefusals {
            structured_output: true,
            tool_choice: true,
        },
    )
    .await;
    let (service, paths, session_id) = service_with_a_turn("gateway-text", &server).await;

    assert_eq!(service.recap_session(session_id).await.unwrap(), expected());

    let requests = server.requests();
    assert_eq!(requests.len(), 4, "the turn, two refused, the text");
    assert!(body(&requests[2])["tool_choice"].is_object());
    let text = body(&requests[3]);
    assert!(text.get("text").is_none());
    assert_eq!(text["tool_choice"], "none");
    assert!(text["tools"].as_array().unwrap().is_empty());
    assert!(text["instructions"]
        .as_str()
        .unwrap()
        .contains("Answer with only a JSON object that matches this JSON schema"));

    assert_eq!(service.recap_session(session_id).await.unwrap(), expected());
    assert_eq!(server.requests().len(), 5);
    assert!(body(&server.requests()[4]).get("text").is_none());

    // Forgetting what the Provider taught tries native output again.
    let provider_id = service.snapshot(session_id).unwrap().provider_id;
    assert_eq!(
        service
            .forget_learned_model_facts(&provider_id, None)
            .await
            .unwrap(),
        1
    );
    assert_eq!(service.recap_session(session_id).await.unwrap(), expected());
    let requests = server.requests();
    assert_eq!(requests.len(), 8);
    assert!(body(&requests[5])["text"]["format"].is_object());
    finish(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fenced_recap_preserves_code_fences_inside_its_summary() {
    let summary = "Fixed parsing of ```json blocks; not deployed yet.";
    let encoded = json!({"summary": summary, "next_action": null, "extra": true});
    let fenced = format!("Here is the recap:\n```json\n{encoded}\n```");
    let server = FakeServer::start(vec![
        (assistant_events("Fixed the parser."), Duration::ZERO),
        (assistant_events(&fenced), Duration::ZERO),
    ])
    .await;
    let (service, paths, session_id) = service_with_a_turn("recap-nested-fence", &server).await;
    assert_eq!(
        service.recap_session(session_id).await.unwrap(),
        AgentRecap {
            summary: summary.to_string(),
            next_action: None,
        }
    );
    assert_eq!(
        server.requests().len(),
        2,
        "the recap is parsed without retrying"
    );
    finish(service, paths);
}
