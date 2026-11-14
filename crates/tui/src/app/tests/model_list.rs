//! `/model` end to end: its list opens before any listing answers, each
//! Provider's models arrive as its own listing does, a list closed first
//! drops what comes after, a failed Provider is listed again with Enter,
//! and a model chosen is the session's.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent::test_support::{FakeListing, FakeServer};
use hanafubuki_agent::{AgentConfig, AgentImageInputPolicy};
use ratatui::crossterm::event::KeyCode;
use tokio::sync::Notify;

use super::Harness;
use crate::agent_bridge::AgentReply;
use crate::app_event::{AgentCommand, AppEvent, Op};
use crate::transcript_mode::TranscriptMode;

const MODES: [TranscriptMode; 2] = [TranscriptMode::Terminal, TranscriptMode::Owned];

/// The test Provider's listing: the session's model and another.
const FIXTURE: &str =
    r#"{"data":[{"id":"fixture-model","context_window":200000},{"id":"first-model"}]}"#;

/// A second Provider, `Another Provider`, at `base_url`, before the test
/// one by name though after it in the file; and a model policy with a
/// window and no images.
fn add_another_provider(config: &AgentConfig, base_url: &str) {
    config
        .mutate_providers(1, |registry| {
            registry.model_policy.context_window_tokens = Some(128_000);
            registry.model_policy.input_capabilities.image_input = AgentImageInputPolicy::Disabled;
            let mut provider = registry.providers[0].clone();
            provider.id = "provider-2".into();
            provider.display_name = "Another Provider".into();
            provider.base_url = base_url.into();
            registry.providers.push(provider);
            Ok(())
        })
        .unwrap();
}

/// The row of the screen that names `text`.
fn row<'a>(screen: &'a str, text: &str) -> &'a str {
    screen
        .lines()
        .find(|line| line.contains(text))
        .unwrap_or_else(|| panic!("no row {text}:\n{screen}"))
}

#[test]
fn the_list_shows_at_once_and_each_provider_s_models_as_it_answers() {
    for mode in MODES {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (first, second) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        let slow = runtime.block_on(FakeServer::with_listings(
            Vec::new(),
            vec![FakeListing::ok(r#"{"data":[{"id":"slow-model"}]}"#).held(second.clone())],
        ));
        let mut harness = Harness::listing(
            vec![FakeListing::ok(FIXTURE).held(first.clone())],
            mode,
            |config| add_another_provider(config, &slow.base_url),
        );

        harness.type_and_submit("/model");
        let screen = harness.pump_until_screen_shows("Listing models… 0 of 2 Providers answered");
        assert!(
            row(&screen, "fixture-model (current)").contains("Fixture Provider"),
            "{mode:?}:\n{screen}"
        );
        assert!(
            row(&screen, "Another Provider").contains("listing…"),
            "{screen}"
        );
        assert!(!screen.contains("first-model"), "{screen}");
        let unwrapped: String = screen.split_whitespace().collect();
        assert!(
            unwrapped.contains("Themodelpolicysetseverysession'swindowto128K;imagesareneversent."),
            "{screen}"
        );

        first.notify_one();
        let screen = harness.pump_until_screen_shows("1 of 2 Providers answered");
        assert!(
            row(&screen, "first-model").contains("Fixture Provider"),
            "{mode:?}:\n{screen}"
        );
        assert!(
            row(&screen, "fixture-model (current)").contains("200K  ?         ?"),
            "the session's model takes its listing's window:\n{screen}"
        );
        assert!(
            row(&screen, "Another Provider").contains("listing…"),
            "{screen}"
        );

        let at = |text| screen.lines().position(|line| line.contains(text));
        assert!(
            at("Another Provider") < at("first-model"),
            "the Providers by name:\n{screen}"
        );
        assert!(
            screen.contains("models.dev can fill these"),
            "models.dev is off:\n{screen}"
        );

        second.notify_one();
        let screen = harness.pump_until_screen_shows("slow-model");
        assert!(!screen.contains("Listing models…"), "{mode:?}:\n{screen}");
        assert!(
            row(&screen, "slow-model").contains("Another Provider"),
            "{screen}"
        );

        // The pinned row, slow-model, first-model.
        harness.key(KeyCode::Down);
        harness.key(KeyCode::Enter);
        harness.pump_until("the model changed", |harness| {
            harness.transcript().contains("Model changed to slow-model")
        });
        assert!(!harness.app.chat_widget.has_command_view("model"));
    }
}

#[test]
fn a_list_closed_before_its_answer_leaves_no_view_and_no_error() {
    let release = Arc::new(Notify::new());
    let mut harness = Harness::listing(
        vec![
            FakeListing::ok(FIXTURE).held(release.clone()),
            FakeListing::ok(FIXTURE),
        ],
        TranscriptMode::Terminal,
        |_| {},
    );
    harness.type_and_submit("/model");
    harness.pump_until_screen_shows("Listing models… 0 of 1 Provider answered");
    harness.key(KeyCode::Esc);
    harness.pump_until("the list to close", |harness| {
        !harness.app.chat_widget.has_command_view("model")
    });

    release.notify_one();
    // The answer arrives with no list to take it.
    let quiet = Instant::now() + Duration::from_secs(1);
    harness.pump_until("the late answer", |_| Instant::now() > quiet);
    assert!(!harness.app.chat_widget.has_command_view("model"));
    let transcript = harness.transcript();
    assert!(
        !transcript.contains('■') && !transcript.contains('⚠'),
        "no error:\n{transcript}"
    );

    // A new list hears its own listing only.
    harness.type_and_submit("/model");
    let screen = harness.pump_until_screen_shows("first-model");
    assert!(!screen.contains("Listing models…"), "{screen}");
    assert_eq!(harness.server.listing_requests().len(), 2);
}

#[test]
fn a_session_that_cannot_be_created_ends_the_list_s_wait() {
    let mut harness = Harness::listing(Vec::new(), TranscriptMode::Terminal, |_| {});
    harness.type_and_submit("/model");
    // The session is asked for, and the listing waits for it; then its
    // creation fails, before the real one answers.
    loop {
        let event = harness.events.recv_timeout(super::DEADLINE).unwrap();
        let listing = matches!(
            &event,
            AppEvent::Op(Op::Command(AgentCommand::ListModels { .. }))
        );
        assert!(harness.app.handle_event(&mut harness.tui, event));
        if listing {
            break;
        }
    }
    let refused = AgentReply::SessionNotCreated("the session was refused".to_string());
    if let Some(update) = harness.app.bridge.reply(refused) {
        harness.app.show_update(update);
    }
    let screen =
        harness.pump_until_screen_shows("The Providers were not read: the session was refused");
    assert!(!screen.contains("Listing models…"), "{screen}");
}

#[test]
fn enter_on_a_provider_that_failed_lists_it_again() {
    let mut harness = Harness::listing(
        vec![FakeListing::failing(401), FakeListing::ok(FIXTURE)],
        TranscriptMode::Terminal,
        |_| {},
    );
    harness.type_and_submit("/model");
    let screen = harness.pump_until_screen_shows("not listed:");
    assert!(
        screen
            .lines()
            .any(|line| line.trim_start().starts_with("Fixture Provider")
                && line.contains("not listed:")),
        "the Provider's row says it:\n{screen}"
    );
    assert_eq!(harness.server.listing_requests().len(), 1);

    harness.key(KeyCode::Down);
    harness.pump_until_screen_shows("enter lists again");
    harness.key(KeyCode::Enter);
    let screen = harness.pump_until_screen_shows("first-model");
    assert_eq!(harness.server.listing_requests().len(), 2);
    assert!(!screen.contains("not listed:"), "{screen}");
}
