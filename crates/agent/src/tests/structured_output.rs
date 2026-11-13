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
