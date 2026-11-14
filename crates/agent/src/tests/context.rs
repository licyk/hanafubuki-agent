//! `/context`'s read through the facade (`session_context`): before any
//! request an estimate of the next, while a response streams the request
//! its step sent, and once the response is back that request with what the
//! Provider reported of it and the session's cache figures.

use hanafubuki_agent_kernel::{AgentContextComposition, AgentContextRequest, AgentReportedPrompt};

use super::*;

/// The groups add up to the request's estimate.
fn assert_adds_up(composition: &AgentContextComposition) {
    let sum = composition.instructions_tokens
        + composition
            .tools
            .iter()
            .map(|tools| tools.tokens)
            .sum::<u64>()
        + composition
            .sections
            .iter()
            .map(|section| section.tokens)
            .sum::<u64>()
        + composition.user_messages_tokens
        + composition.assistant_messages_tokens
        + composition.reasoning_tokens
        + composition.tool_calls_tokens
        + composition.continuation_tokens
        + composition.compaction_summaries_tokens
        + composition.image_tokens
        + composition.other_tokens;
    assert_eq!(sum, composition.total_tokens, "{composition:#?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_context_is_the_next_request_then_the_one_sent_with_what_the_provider_reported() {
    // The answer holds before it completes, with 1,200 prompt tokens of
    // which 900 were read from the cache.
    let answer = String::from_utf8(assistant_events("Hi."))
        .unwrap()
        .replace(
            "data: {\"type\":\"response.completed\"",
            &format!("{HOLD}data: {{\"type\":\"response.completed\""),
        )
        .replace(
            "\"end_turn\":true}",
            "\"end_turn\":true,\"usage\":{\"input_tokens\":1200,\"output_tokens\":3,\"input_tokens_details\":{\"cached_tokens\":900}}}",
        )
        .into_bytes();
    let server = FakeServer::start(vec![(answer, Duration::ZERO)]).await;
    let (service, paths) = service_with("session-context", |_, config| {
        configure_test_provider(config, &server.base_url, None);
        config
            .mutate_providers(1, |registry| {
                registry.model_policy.context_window_tokens = Some(100_000);
                Ok(())
            })
            .unwrap();
    });
    let service = Arc::new(service);
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();

    let before = service.session_context(session.id).await.unwrap();
    assert_eq!(before.context_window_tokens, Some(100_000));
    assert_eq!(before.usage, None);
    let AgentContextRequest::Next(next) = &before.request else {
        panic!("before any request: {:?}", before.request);
    };
    // Outside a turn no section is rendered, and there is no message yet.
    assert!(next.sections.is_empty(), "{next:#?}");
    assert_eq!(next.user_messages_tokens, 0);
    assert!(next.instructions_tokens > 0);
    assert!(next.tools.iter().any(|tools| tools.count > 0), "{next:#?}");
    assert_adds_up(next);
    assert!(server.requests().is_empty(), "the estimate called no model");

    let turn = tokio::spawn({
        let service = service.clone();
        async move {
            service
                .start_turn(session.id, "Hello there.".to_string().into())
                .await
                .unwrap();
        }
    });
    let sent = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let context = service.session_context(session.id).await.unwrap();
            if let AgentContextRequest::Sent(sent) = context.request {
                break sent;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the turn's request was sent within a minute");
    assert_eq!(sent.reported, None, "the response is still streaming");
    assert!(sent.composition.user_messages_tokens > 0);
    assert!(
        !sent.composition.sections.is_empty(),
        "a turn's request carries its sections: {:#?}",
        sent.composition
    );
    assert_eq!(sent.composition.tools, next.tools);
    assert_adds_up(&sent.composition);

    server.hold.notify_one();
    turn.await.unwrap();
    let after = service.session_context(session.id).await.unwrap();
    let AgentContextRequest::Sent(answered) = &after.request else {
        panic!("after the answer: {:?}", after.request);
    };
    assert_eq!(answered.composition, sent.composition);
    assert_eq!(
        answered.reported,
        Some(AgentReportedPrompt {
            input_tokens: 1_200,
            cache_read_tokens: Some(900),
            cache_write_tokens: None,
        })
    );
    let usage = after.usage.unwrap();
    assert_eq!(usage.cumulative_cache_read_tokens, 900);
    assert_eq!(usage.cache_reporting_responses, 1);

    let service = Arc::into_inner(service).unwrap();
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
