use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::model::{ElicitRequestParams, ElicitationAction};

use super::*;

#[derive(Default)]
struct RecordingEvents(Mutex<Vec<McpEventNotice>>);

impl McpEventSink for RecordingEvents {
    fn emit(&self, notice: McpEventNotice) {
        self.0.lock().unwrap().push(notice);
    }
}

fn url_request() -> ElicitRequestParams {
    ElicitRequestParams::UrlElicitationParams {
        meta: None,
        message: "Open the remote form".into(),
        url: "https://example.test/form".into(),
        elicitation_id: "remote-1".into(),
    }
}

async fn wait_for_pending(hub: &McpInteractionHub) -> McpElicitationRequest {
    for _ in 0..20 {
        if let Some(request) = hub.pending().into_iter().next() {
            return request;
        }
        tokio::task::yield_now().await;
    }
    panic!("elicitation did not enter the pending registry");
}

async fn wait_for_pending_count(hub: &McpInteractionHub, expected: usize) {
    for _ in 0..40 {
        if hub.pending().len() == expected {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("elicitation registry did not reach {expected} pending requests");
}

fn call_context(call_id: &str) -> McpCallContext {
    McpCallContext {
        session_id: "session-1".to_string(),
        turn_id: 1,
        call_id: call_id.to_string(),
        tool_generation: "11".repeat(32),
    }
}

#[test]
fn client_handler_advertises_the_configured_protocol_mode() {
    let server = McpServerId::parse("remote").unwrap();
    let legacy = McpClientHandler::new(
        server.clone(),
        McpConnectionGeneration([1; 32]),
        Duration::from_secs(1),
        McpInteractionHub::default(),
        crate::McpProtocolMode::Legacy,
    );
    let modern = McpClientHandler::new(
        server,
        McpConnectionGeneration([2; 32]),
        Duration::from_secs(1),
        McpInteractionHub::default(),
        crate::McpProtocolMode::V20260728,
    );
    assert_eq!(
        rmcp::ClientHandler::get_info(&legacy).protocol_version,
        rmcp::model::ProtocolVersion::V_2025_06_18
    );
    assert_eq!(
        rmcp::ClientHandler::get_info(&modern).protocol_version,
        rmcp::model::ProtocolVersion::V_2026_07_28
    );
}

#[tokio::test]
async fn elicitation_resolution_is_generation_bound_and_emits_projection_updates() {
    let events = Arc::new(RecordingEvents::default());
    let hub = McpInteractionHub::new(events.clone());
    let server_id = McpServerId::parse("remote").unwrap();
    let generation = McpConnectionGeneration([7; 32]);
    let _active = hub.enter_call(server_id.clone(), generation, call_context("call-1"));
    let request_task = tokio::spawn({
        let hub = hub.clone();
        let server_id = server_id.clone();
        async move {
            hub.request(server_id, generation, url_request(), Duration::from_secs(1))
                .await
        }
    });

    let request = wait_for_pending(&hub).await;
    assert_eq!(request.server_id, server_id);
    assert_eq!(request.connection_generation, generation);
    assert_eq!(request.session_id, "session-1");
    assert_eq!(request.turn_id, 1);
    assert_eq!(request.call_id, "call-1");
    hub.resolve(
        request.id,
        McpElicitationDecision::Accept {
            content: Value::Null,
        },
    )
    .unwrap();

    let result = request_task.await.unwrap();
    assert_eq!(result.action, ElicitationAction::Accept);
    assert!(result.content.is_none());
    assert!(hub.pending().is_empty());
    assert_eq!(events.0.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn elicitation_timeout_declines_and_connection_cancel_cancels() {
    let hub = McpInteractionHub::default();
    let server_id = McpServerId::parse("remote").unwrap();
    let generation = McpConnectionGeneration([9; 32]);
    let _active = hub.enter_call(server_id.clone(), generation, call_context("call-2"));
    let timed_out = hub
        .request(
            server_id.clone(),
            generation,
            url_request(),
            Duration::from_millis(1),
        )
        .await;
    assert_eq!(timed_out.action, ElicitationAction::Decline);
    assert!(hub.pending().is_empty());

    let request_task = tokio::spawn({
        let hub = hub.clone();
        let server_id = server_id.clone();
        async move {
            hub.request(server_id, generation, url_request(), Duration::from_secs(1))
                .await
        }
    });
    wait_for_pending(&hub).await;
    hub.cancel_connection(&server_id, generation);
    assert_eq!(
        request_task.await.unwrap().action,
        ElicitationAction::Cancel
    );
    assert!(hub.pending().is_empty());
}

#[tokio::test]
async fn ambiguous_calls_are_declined_and_shutdown_declines_pending_elicitation() {
    let hub = McpInteractionHub::default();
    let server_id = McpServerId::parse("remote").unwrap();
    let generation = McpConnectionGeneration([10; 32]);
    let _first = hub.enter_call(server_id.clone(), generation, call_context("call-a"));
    let _second = hub.enter_call(server_id.clone(), generation, call_context("call-b"));
    assert_eq!(
        hub.request(
            server_id.clone(),
            generation,
            url_request(),
            Duration::from_secs(1),
        )
        .await
        .action,
        ElicitationAction::Decline
    );
    drop(_second);

    let request_task = tokio::spawn({
        let hub = hub.clone();
        let server_id = server_id.clone();
        async move {
            hub.request(server_id, generation, url_request(), Duration::from_secs(1))
                .await
        }
    });
    wait_for_pending(&hub).await;
    hub.cancel_all();
    assert_eq!(
        request_task.await.unwrap().action,
        ElicitationAction::Decline
    );
    assert!(hub.pending().is_empty());
}

#[tokio::test]
async fn concurrent_elicitations_share_one_reference_counted_call_pause() {
    let hub = McpInteractionHub::default();
    let server_id = McpServerId::parse("remote").unwrap();
    let generation = McpConnectionGeneration([12; 32]);
    let active = hub.enter_call(server_id.clone(), generation, call_context("call-nested"));
    let paused = active.pause_state().subscribe();
    let first = tokio::spawn({
        let hub = hub.clone();
        let server_id = server_id.clone();
        async move {
            hub.request(server_id, generation, url_request(), Duration::from_secs(1))
                .await
        }
    });
    let second = tokio::spawn({
        let hub = hub.clone();
        let server_id = server_id.clone();
        async move {
            hub.request(server_id, generation, url_request(), Duration::from_secs(1))
                .await
        }
    });
    wait_for_pending_count(&hub, 2).await;
    assert!(*paused.borrow());
    let mut pending = hub.pending();
    hub.resolve(pending.remove(0).id, McpElicitationDecision::Decline)
        .unwrap();
    tokio::task::yield_now().await;
    assert!(*paused.borrow());
    hub.resolve(pending.remove(0).id, McpElicitationDecision::Decline)
        .unwrap();
    assert_eq!(first.await.unwrap().action, ElicitationAction::Decline);
    assert_eq!(second.await.unwrap().action, ElicitationAction::Decline);
    assert!(!*paused.borrow());
}

#[tokio::test]
async fn exact_call_cancellation_does_not_touch_another_server_call() {
    let hub = McpInteractionHub::default();
    let first_server = McpServerId::parse("first").unwrap();
    let second_server = McpServerId::parse("second").unwrap();
    let generation = McpConnectionGeneration([13; 32]);
    let first_context = call_context("call-first");
    let second_context = call_context("call-second");
    let _first_active = hub.enter_call(first_server.clone(), generation, first_context.clone());
    let _second_active = hub.enter_call(second_server.clone(), generation, second_context.clone());
    let first = tokio::spawn({
        let hub = hub.clone();
        let server = first_server.clone();
        async move {
            hub.request(server, generation, url_request(), Duration::from_secs(1))
                .await
        }
    });
    let second = tokio::spawn({
        let hub = hub.clone();
        let server = second_server.clone();
        async move {
            hub.request(server, generation, url_request(), Duration::from_secs(1))
                .await
        }
    });
    wait_for_pending_count(&hub, 2).await;
    hub.cancel_call(&first_server, generation, &first_context);
    assert_eq!(first.await.unwrap().action, ElicitationAction::Cancel);
    let remaining = hub.pending();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].server_id, second_server);
    hub.resolve(remaining[0].id, McpElicitationDecision::Decline)
        .unwrap();
    assert_eq!(second.await.unwrap().action, ElicitationAction::Decline);
}
