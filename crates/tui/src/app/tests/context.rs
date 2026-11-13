//! `/context` on an emulated terminal, in both presentations: before any
//! request the next one's estimate, during a turn the request it sent with
//! no count from the Provider yet, and after it that request with the
//! Provider's count and the prompt cache.

use hanafubuki_agent::test_support::HOLD;

use super::*;

const MODES: [TranscriptMode; 2] = [TranscriptMode::Terminal, TranscriptMode::Owned];

/// An answer that holds before it completes, then reports 1,200 prompt
/// tokens of which 900 were read from the cache.
fn cached_answer() -> Vec<u8> {
    String::from_utf8(assistant_events("Hi."))
        .unwrap()
        .replace(
            "data: {\"type\":\"response.completed\"",
            &format!("{HOLD}data: {{\"type\":\"response.completed\""),
        )
        .replace(
            "\"end_turn\":true}",
            "\"end_turn\":true,\"usage\":{\"input_tokens\":1200,\"output_tokens\":3,\"input_tokens_details\":{\"cached_tokens\":900}}}",
        )
        .into_bytes()
}

#[test]
fn context_shows_the_next_request_then_the_one_sent_and_its_cache() {
    for mode in MODES {
        let mut harness = Harness::start_configured(
            vec![(cached_answer(), Duration::ZERO)],
            true,
            mode,
            |config| {
                config
                    .mutate_providers(1, |registry| {
                        registry.model_policy.context_window_tokens = Some(100_000);
                        Ok(())
                    })
                    .unwrap();
            },
        );

        harness.type_and_submit("/context");
        let screen = harness.pump_until_screen_shows("Cache hit rate: last (no request yet)");
        assert!(
            screen.contains("No request sent yet. The next: about"),
            "{mode:?}:\n{screen}"
        );
        assert!(screen.contains("Context window: 100k"), "{screen}");
        assert!(screen.contains("Instructions"), "{screen}");
        assert!(screen.contains("Free"), "{screen}");

        harness.type_and_submit("Hello.");
        // The answer streams up to its hold, without a line to show yet.
        harness.pump_until("the request sent", |harness| {
            harness.server.requests().len() == 1
        });
        assert!(harness.app.chat_widget.is_task_running());
        harness.type_and_submit("/context");
        let screen = harness.pump_until_screen_shows("no count from the Provider");
        assert!(screen.contains("User messages"), "{mode:?}:\n{screen}");
        assert!(screen.contains("Context sections"), "{screen}");

        harness.server.hold.notify_one();
        harness.pump_until_turn_ends();
        harness.type_and_submit("/context");
        let screen = harness.pump_until_screen_shows("the Provider counted 1.2k");
        assert!(
            screen.contains("Cache hit rate: last 75.0% · session 75.0% — 900 of 1.2k prompt tokens from cache (1 request)"),
            "{mode:?}:\n{screen}"
        );
        assert!(
            screen.contains("Cached tokens: last 900 · session 900"),
            "{screen}"
        );
        assert!(
            screen.contains("Uncached tokens: last 300 · session 300"),
            "{screen}"
        );
    }
}
