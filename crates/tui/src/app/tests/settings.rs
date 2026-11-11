//! Configuration in the interface, end to end over the standalone Agent:
//! the first-run setup, `/providers` and the MCP manager, driven by keys,
//! with what they wrote read back from the configuration.

use hanafubuki_agent::test_support::assistant_events;
use hanafubuki_agent::{
    AgentModelPolicyWriteRequest, AgentProviderRegistrySnapshot, McpApplicationSnapshot,
    McpCredentialKey, McpServerId, McpTransportConfig,
};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::Harness;
use crate::transcript_mode::TranscriptMode;

/// A models endpoint's answer listing `ids`.
fn models(ids: &[&str]) -> Vec<u8> {
    let data: Vec<_> = ids
        .iter()
        .map(|id| serde_json::json!({ "id": id }))
        .collect();
    serde_json::json!({ "data": data }).to_string().into_bytes()
}

impl Harness {
    /// Types `text` into the open view, a key at a time.
    fn type_keys(&mut self, text: &str) {
        for character in text.chars() {
            self.key(KeyCode::Char(character));
        }
    }

    fn tab(&mut self, times: usize) {
        for _ in 0..times {
            self.key(KeyCode::Tab);
        }
    }

    /// The Providers as the Agent has them.
    fn registry(&self) -> AgentProviderRegistrySnapshot {
        let agent = self.runtime.slot().agent().unwrap();
        let call = agent.clone();
        agent
            .runtime()
            .block_on(async move { call.provider_registry_snapshot().await })
            .unwrap()
            .unwrap()
    }

    /// Writes the model policy with `model`, keeping the default Provider.
    fn write_model(&self, model: &str) {
        let agent = self.runtime.slot().agent().unwrap();
        let call = agent.clone();
        let registry = self.registry();
        let mut policy = registry.model_policy;
        policy.model = model.to_string();
        agent
            .runtime()
            .block_on(async move {
                call.write_model_policy(AgentModelPolicyWriteRequest {
                    expected_revision: registry.revision,
                    default_provider_id: registry.default_provider_id,
                    policy,
                })
                .await
            })
            .unwrap()
            .unwrap();
    }

    fn mcp(&self) -> McpApplicationSnapshot {
        let agent = self.runtime.slot().agent().unwrap();
        let call = agent.clone();
        agent
            .runtime()
            .block_on(async move { call.mcp_snapshot().await })
            .unwrap()
            .unwrap()
    }

    fn pump_until_transcript_shows(&mut self, text: &str) {
        self.pump_until(&format!("the transcript to show {text:?}"), |harness| {
            harness.transcript().contains(text)
        });
    }
}

#[test]
fn without_a_provider_the_setup_opens_and_the_first_message_waits_for_it() {
    let mut harness = Harness::start_with(
        vec![
            models(&["model-a", "model-b"]),
            models(&["model-a", "model-b"]),
            assistant_events("The answer is 42."),
        ],
        false,
        TranscriptMode::Terminal,
    );
    harness.pump_until_screen_shows("Test: list the models");
    // Closed (the form, then the list), the composer takes a message, which
    // waits in the composer while the setup opens again.
    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("Add a Provider…");
    harness.key(KeyCode::Esc);
    harness.type_and_submit("What is the answer?");
    harness.pump_until_transcript_shows("Set up a model Provider and choose a model first");
    harness.pump_until_screen_shows("Test: list the models");
    assert!(!harness.app.chat_widget.composer_is_empty());
    assert_eq!(harness.app.bridge.session_id(), None);

    let base_url = harness.server.base_url.clone();
    harness.type_keys("Fixture");
    harness.tab(2);
    harness.type_keys(&base_url);
    harness.tab(1);
    harness.type_keys("sk-test");
    let screen = harness.pump_until_screen_shows("API token");
    assert!(!screen.contains("sk-tes"), "{screen}");
    // Enabled, then "Add a header".
    harness.tab(2);
    harness.key(KeyCode::Enter);
    harness.type_keys("X-Team");
    harness.tab(1);
    harness.type_keys("blue");
    // The row's add stop, then Test.
    harness.tab(2);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("The Provider lists 2 models.");
    let registry = harness.registry();
    assert!(registry.providers.is_empty(), "a test saves nothing");
    harness.tab(1);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Choose the model");
    harness.pump_until_screen_shows("model-b");
    harness.key(KeyCode::Enter);
    harness.pump_until_transcript_shows("New sessions run model-a on Fixture.");

    let registry = harness.registry();
    let provider = &registry.providers[0];
    assert_eq!(
        registry.default_provider_id.as_deref(),
        Some(provider.id.as_str())
    );
    assert_eq!(registry.model_policy.model, "model-a");
    assert_eq!(provider.api_token.as_deref(), Some("sk-test"));
    assert_eq!(provider.request_headers[0].name, "X-Team");
    assert_eq!(provider.request_headers[0].value, "blue");
    let test = &harness.server.requests()[0];
    assert_eq!(test.headers["authorization"], "Bearer sk-test");
    assert_eq!(test.headers["x-team"], "blue");

    // The message kept in the composer now starts the session.
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("The answer is 42.");
    let session_id = harness.app.bridge.session_id().unwrap();
    let agent = harness.runtime.slot().agent().unwrap();
    assert_eq!(agent.snapshot(session_id).unwrap().model, "model-a");
}

#[test]
fn with_a_provider_but_no_model_the_setup_is_the_model_picker() {
    let mut harness = Harness::start(vec![models(&["model-a"])]);
    harness.write_model("");
    // `/new` looks again, as a start does.
    harness.type_and_submit("/new");
    harness.pump_until_screen_shows("Choose the model");
    harness.pump_until_screen_shows("model-a");
    harness.key(KeyCode::Enter);
    harness.pump_until_transcript_shows("New sessions run model-a on Fixture Provider.");
    let registry = harness.registry();
    assert_eq!(registry.model_policy.model, "model-a");
}

#[test]
fn a_save_over_a_moved_registry_reads_it_again_and_keeps_the_form() {
    let mut harness = Harness::start(vec![models(&["fixture-model"])]);
    harness.type_and_submit("/providers");
    harness.pump_until_screen_shows("★ Fixture Provider");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Edit Fixture Provider");
    // Another writer moves the registry.
    harness.write_model("fixture-model");
    harness.key_with(KeyCode::Char('u'), KeyModifiers::CONTROL);
    harness.type_keys("Renamed");
    // Up wraps to Save.
    harness.key(KeyCode::Up);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("changed while this screen was open");
    assert!(harness.screen().contains("Renamed"));
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Choose the model");
    let registry = harness.registry();
    assert_eq!(registry.providers[0].display_name, "Renamed");
    assert_eq!(registry.revision, 3);
    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("★ Renamed");
}

#[test]
fn a_policy_save_over_a_moved_registry_reads_it_again_and_keeps_the_form() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/providers");
    harness.pump_until_screen_shows("★ Fixture Provider");
    harness.key(KeyCode::Char('p'));
    harness.pump_until_screen_shows("Model policy");
    harness.tab(2);
    harness.type_keys("low");
    // Reasoning, Advanced, Save.
    harness.tab(2);
    // Another writer moves the registry before the form submits.
    harness.write_model("fixture-model");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("changed while this screen was open");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved.");
    let registry = harness.registry();
    assert_eq!(
        registry
            .model_policy
            .reasoning_effort
            .as_ref()
            .map(|effort| effort.as_str()),
        Some("low")
    );
    assert_eq!(registry.revision, 3);
}

#[test]
fn the_provider_policy_form_saves_reasoning_context_and_the_whole_budget() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/providers");
    harness.pump_until_screen_shows("★ Fixture Provider");
    harness.key(KeyCode::Char('p'));
    harness.pump_until_screen_shows("Model policy");

    // Default Provider, model, reasoning, then the Advanced switch.
    harness.tab(2);
    harness.type_keys("high");
    harness.tab(1);
    harness.key(KeyCode::Enter);
    // Image input, then the context window.
    harness.tab(2);
    harness.type_keys("123456");
    // Automatic compaction, threshold, remote compaction, turn budget.
    harness.tab(4);
    harness.key(KeyCode::Right);
    // The eleven budget fields, then Save.
    harness.tab(12);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved.");

    let policy = harness.registry().model_policy;
    assert_eq!(
        policy
            .reasoning_effort
            .as_ref()
            .map(|effort| effort.as_str()),
        Some("high")
    );
    assert_eq!(policy.context_window_tokens, Some(123_456));
    let budget = policy.turn_budget.expect("the custom budget was saved");
    assert_eq!(budget.maximum_cumulative_model_input_tokens, 8_000_000);
    assert_eq!(budget.maximum_cumulative_model_output_tokens, 1_000_000);
    assert_eq!(budget.maximum_tool_invocations, 256);
    assert_eq!(budget.maximum_tool_output_bytes, 16 * 1024 * 1024);
    assert_eq!(budget.maximum_hook_fragment_bytes, 1024 * 1024);
    assert_eq!(budget.maximum_pending_input_items, 64);
    assert_eq!(budget.maximum_in_flight_tools, 8);
    assert_eq!(budget.maximum_consecutive_stalled_steps, 4);
    assert_eq!(budget.model_stream_idle_timeout_seconds, 120);
    assert_eq!(budget.tool_idle_timeout_seconds, 600);
    assert_eq!(budget.approval_timeout_seconds, 900);
}

/// Once the default model's listing names its reasoning levels, the policy
/// form offers them as a choice (free text while none are known, as the
/// tests above type it), and saves the one chosen.
#[test]
fn the_policy_form_offers_the_listed_reasoning_levels() {
    let listing = serde_json::json!({"data": [
        {"id": "fixture-model", "reasoning": {"supported_efforts": ["high", "low"]}}
    ]});
    let mut harness = Harness::start(vec![listing.to_string().into_bytes()]);
    let agent = harness.runtime.slot().agent().unwrap();
    let call = agent.clone();
    agent
        .runtime()
        .block_on(async move {
            call.list_provider_models(hanafubuki_agent::AgentProviderModelCatalogRequest::default())
                .await
        })
        .unwrap()
        .unwrap();
    harness.type_and_submit("/providers policy");
    let screen = harness.pump_until_screen_shows("○ low");
    assert!(
        screen.contains("● Provider default") && screen.contains("○ high"),
        "{screen}"
    );
    // Reasoning: low, then Advanced and Save.
    harness.tab(2);
    harness.key(KeyCode::Right);
    harness.tab(2);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved.");
    assert_eq!(
        harness
            .registry()
            .model_policy
            .reasoning_effort
            .as_ref()
            .map(|effort| effort.as_str()),
        Some("low")
    );
}

/// The policy form's window field, left empty, names the window the
/// default model's listing reported in its placeholder; the model picker
/// shows what each entry reports; and `/status` shows a session's window
/// with its source, or that it is unknown; `/model forget` says when
/// nothing was learned.
#[test]
fn the_reported_window_is_shown_in_the_policy_form_the_picker_and_status() {
    let listing = serde_json::json!({"data": [
        {"id": "fixture-model", "context_window": 200000, "input_modalities": ["text", "image"]}
    ]});
    let mut harness = Harness::start(vec![
        assistant_events("First."),
        listing.to_string().into_bytes(),
        assistant_events("Second."),
    ]);
    harness.exchange("hello", "First.");
    harness.type_and_submit("/status");
    harness.pump_until_screen_shows("Context window:");
    assert!(
        harness
            .transcript()
            .contains("unknown (compacts on the Provider's error)"),
        "{}",
        harness.transcript()
    );

    harness.type_and_submit("/model");
    let picker = harness.pump_until_screen_shows("200K · vision");
    assert!(picker.contains("fixture-model"), "{picker}");
    harness.key(KeyCode::Esc);
    harness.type_and_submit("/providers policy");
    harness.pump_until_screen_shows("Model policy");
    // Advanced, after the default Provider, the model and the reasoning.
    harness.tab(3);
    harness.key(KeyCode::Char(' '));
    harness.pump_until_screen_shows("200K, reported");
    // Back to the list, then closed.
    harness.key(KeyCode::Esc);
    harness.key(KeyCode::Esc);

    harness.exchange("again", "Second.");
    harness.type_and_submit("/status");
    harness.pump_until("the reported window in /status", |harness| {
        harness.transcript().contains("200K, reported)")
    });
    harness.type_and_submit("/model forget");
    harness.pump_until("the forgetting", |harness| {
        harness
            .transcript()
            .contains("Nothing was learned of fixture-model to forget.")
    });
}

#[test]
fn the_configuration_center_opens_the_model_policy_directly() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/config providers");
    harness.pump_until_screen_shows("Model policy");
}

#[test]
fn the_providers_command_opens_the_model_policy_directly() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/providers policy");
    harness.pump_until_screen_shows("Model policy");
}

#[test]
fn the_mcp_manager_adds_a_server_with_a_secret_and_deletes_it() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/mcp");
    harness.pump_until_screen_shows("Add a server…");
    harness.key(KeyCode::Char('a'));
    harness.pump_until_screen_shows("Add an MCP server");
    harness.type_keys("tools");
    harness.tab(2);
    harness.type_keys("sh");
    harness.tab(1);
    harness.type_keys("-c \"exit 0\"");
    // The working directory, then "Add a variable".
    harness.tab(2);
    harness.key(KeyCode::Enter);
    harness.type_keys("API_KEY");
    harness.tab(1);
    harness.type_keys("hunter2");
    harness.tab(1);
    harness.key(KeyCode::Char(' '));
    // The add stop, then Enabled: off, so nothing is started.
    harness.tab(2);
    harness.key(KeyCode::Char(' '));
    // Required, Advanced, Save.
    harness.tab(3);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved.");

    let id = McpServerId::parse("tools").unwrap();
    let key = McpCredentialKey::for_server_secret(&id, "API_KEY");
    let settings = harness.mcp().settings;
    let server = &settings.servers[0];
    assert_eq!(server.id, id);
    assert!(!server.enabled);
    let McpTransportConfig::Stdio(stdio) = &server.transport else {
        panic!("a stdio server: {server:?}");
    };
    assert_eq!(stdio.program, "sh");
    assert_eq!(stdio.argv, ["-c", "exit 0"]);
    assert!(stdio.public_environment.is_empty());
    assert_eq!(stdio.credential_environment["API_KEY"], key);
    assert_eq!(settings.credentials[&key].value, "hunter2");

    // Edited again, the secret is dots.
    harness.key(KeyCode::Enter);
    let screen = harness.pump_until_screen_shows("Edit MCP server tools");
    assert!(
        screen.contains("API_KEY") && !screen.contains("hunter2"),
        "{screen}"
    );
    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("Add a server…");

    harness.key(KeyCode::Char('d'));
    harness.pump_until_screen_shows("Delete the MCP server tools?");
    harness.key(KeyCode::Enter);
    harness.pump_until("the server and its secret to go", |harness| {
        let settings = harness.mcp().settings;
        settings.servers.is_empty() && settings.credentials.is_empty()
    });
}

#[test]
fn mcp_verbose_writes_the_report_and_a_bad_argument_says_how() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/mcp verbose");
    harness.pump_until_transcript_shows("MCP Servers");
    harness.type_and_submit("/mcp what");
    harness.pump_until_transcript_shows("Usage: /mcp [verbose | login ID]");
    harness.type_and_submit("/mcp login ../x");
    harness.pump_until_transcript_shows("is not an MCP server id");
}

#[test]
fn an_mcp_login_the_server_refuses_says_why_in_the_manager() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/mcp login nope");
    harness.pump_until_screen_shows("There is no MCP server nope.");
    harness.key(KeyCode::Esc);
    harness.type_and_submit("/mcp");
    harness.pump_until_screen_shows("Add a server…");
    harness.key(KeyCode::Char('a'));
    harness.pump_until_screen_shows("Add an MCP server");
    harness.type_keys("docs");
    harness.key(KeyCode::Tab);
    harness.key(KeyCode::Right);
    harness.key(KeyCode::Tab);
    harness.type_keys("http://127.0.0.1:9/mcp");
    // OAuth (disabled), the headers' add stop, Enabled: off.
    harness.tab(3);
    harness.key(KeyCode::Char(' '));
    harness.tab(3);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved.");
    harness.key(KeyCode::Esc);
    harness.type_and_submit("/mcp login docs");
    harness.pump_until_screen_shows("OAuth is disabled for this MCP server");
}
