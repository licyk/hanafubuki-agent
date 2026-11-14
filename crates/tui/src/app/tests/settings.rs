//! Configuration in the interface, end to end over the standalone Agent:
//! the first-run setup, the Providers, the model policy and the MCP
//! servers in the settings center, driven by keys, with what they wrote
//! read back from the configuration.

use std::sync::Arc;

use hanafubuki_agent::test_support::{assistant_events, FakeListing};
use hanafubuki_agent::{
    AgentModelPolicyWriteRequest, AgentProviderRegistrySnapshot, McpApplicationSnapshot,
    McpCredentialKey, McpServerId, McpTransportConfig,
};
use ratatui::crossterm::event::KeyCode;
use tokio::sync::Notify;

use super::Harness;
use crate::agent_bridge::{AgentReply, McpReply, TurnUpdate};
use crate::app_event::AppEvent;
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
            assistant_events("The answer is 42."),
        ],
        false,
        TranscriptMode::Terminal,
    );
    // The center opens at the Providers; closed, the composer takes a
    // message, which waits there while the center opens again.
    harness.pump_until_screen_shows("Settings › Providers");
    harness.app.close_settings_center(&mut harness.tui);
    harness.type_and_submit("What is the answer?");
    harness.pump_until_transcript_shows("Set up a model Provider and choose a model first");
    harness.pump_until_screen_shows("Settings › Providers");
    assert!(!harness.app.chat_widget.composer_is_empty());
    assert_eq!(harness.app.bridge.session_id(), None);

    harness.key(KeyCode::Char('a'));
    harness.pump_until_screen_shows("Providers › Add a Provider");
    harness.edit_setting("Name", "Fixture");
    let base_url = harness.server.base_url.clone();
    harness.edit_setting("Base URL", &base_url);
    harness.select_setting("API token");
    harness.key(KeyCode::Enter);
    for character in "sk-test".chars() {
        harness.key(KeyCode::Char(character));
    }
    let screen = harness.screen();
    assert!(!screen.contains("sk-tes"), "a secret is dots: {screen}");
    harness.key(KeyCode::Enter);
    harness.select_setting("Add a Provider");
    harness.key(KeyCode::Enter);
    // The record added is selected; its model page lists the models.
    harness.pump_until_screen_shows("› Fixture");
    harness.key(KeyCode::Enter);
    harness.select_setting("Choose the model");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("model-b");
    harness.key(KeyCode::Enter);
    // The center's notice says it, the interface knowing the setup done.
    harness.pump_until_screen_shows("Saved");
    harness.leave_settings();

    let registry = harness.registry();
    let provider = &registry.providers[0];
    assert_eq!(
        registry.default_provider_id.as_deref(),
        Some(provider.id.as_str())
    );
    assert_eq!(registry.model_policy.model, "model-a");
    assert_eq!(provider.api_token.as_deref(), Some("sk-test"));
    let listing = &harness.server.requests()[0];
    assert_eq!(listing.headers["authorization"], "Bearer sk-test");

    // The message kept in the composer now starts the session.
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("The answer is 42.");
    let session_id = harness.app.bridge.session_id().unwrap();
    let agent = harness.runtime.slot().agent().unwrap();
    assert_eq!(agent.snapshot(session_id).unwrap().model, "model-a");
}

#[test]
fn with_a_provider_but_no_model_the_setup_is_the_providers_model_page() {
    let mut harness = Harness::start(vec![models(&["model-a"])]);
    harness.write_model("");
    // `/new` looks again, as a start does.
    harness.type_and_submit("/new");
    harness.pump_until_screen_shows("Settings › Providers");
    harness.select_setting("Fixture Provider");
    harness.key(KeyCode::Enter);
    harness.select_setting("Choose the model");
    harness.key(KeyCode::Enter);
    let picker = harness.pump_until_screen_shows("model-a");
    assert!(
        picker.contains("Fixture Provider becomes the default"),
        "{picker}"
    );
    harness.key(KeyCode::Enter);
    harness.pump_until("the model to be saved", |harness| {
        harness.registry().model_policy.model == "model-a"
    });
}

#[test]
fn the_model_policy_saves_reasoning_context_and_a_custom_budget() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/providers policy");
    harness.pump_until_screen_shows("Settings › Model policy");
    // No level is known: the effort is the Provider's or typed.
    harness.select_setting("Reasoning effort");
    harness.key(KeyCode::Enter);
    let offered = harness.pump_until_screen_shows("2. Custom…");
    assert!(offered.contains("1. Default (current)"), "{offered}");
    assert!(!offered.contains("3. "), "{offered}");
    // Custom types it in the row.
    harness.key(KeyCode::Char('2'));
    for character in "high".chars() {
        harness.key(KeyCode::Char(character));
    }
    harness.key(KeyCode::Enter);
    harness.pump_until("the effort to be saved", |harness| {
        !harness.screen().contains("Saving…")
    });
    harness.edit_setting("Context window", "123456");
    assert_eq!(
        harness.registry().model_policy.context_window_tokens,
        Some(123_456)
    );
    harness.select_setting("Custom turn budget");
    harness.key(KeyCode::Enter);
    harness.pump_until("the budget to be saved", |harness| {
        harness.registry().model_policy.turn_budget.is_some()
    });

    let policy = harness.registry().model_policy;
    assert_eq!(
        policy
            .reasoning_effort
            .as_ref()
            .map(|effort| effort.as_str()),
        Some("high")
    );
    // The budget starts from the Agent's.
    let budget = policy.turn_budget.unwrap();
    assert_eq!(budget.maximum_cumulative_model_input_tokens, 8_000_000);
    assert_eq!(budget.maximum_tool_invocations, 256);
    assert_eq!(budget.approval_timeout_seconds, 900);
    harness.select_setting("Tool calls");
}

/// Once the default model's listing names its reasoning levels, the model
/// policy says them beside the reasoning effort.
#[test]
fn the_model_policy_names_the_listed_reasoning_levels() {
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
    harness.pump_until_screen_shows("Settings › Model policy");
    harness.select_setting("Reasoning effort");
    let screen = harness.pump_until_screen_shows("The model takes");
    assert!(screen.contains("low, high"), "{screen}");
    // And offers them, as `/effort` does.
    harness.key(KeyCode::Enter);
    let offered = harness.pump_until_screen_shows("4. Custom…");
    for row in ["1. low", "2. high", "3. Default (current)"] {
        assert!(offered.contains(row), "{row}:\n{offered}");
    }
}

/// `/effort` sets the session's effort: picked from what it offers (Default
/// and Custom when no level is known), typed in Custom, or given inline.
#[test]
fn effort_sets_the_sessions_reasoning_effort() {
    let mut harness = Harness::start(Vec::new());
    let effort = |harness: &mut Harness| {
        let agent = harness.runtime.slot().agent().unwrap();
        agent
            .snapshot(harness.app.bridge.session_id().unwrap())
            .unwrap()
            .reasoning_effort
            .map(|effort| effort.to_string())
    };
    harness.type_and_submit("/effort");
    let offered = harness.pump_until_screen_shows("Select Reasoning Effort");
    assert!(offered.contains("1. Default (current)"), "{offered}");
    assert!(offered.contains("2. Custom…"), "{offered}");
    harness.key(KeyCode::Char('2'));
    harness.pump_until_screen_shows("Custom reasoning effort");
    for character in "max".chars() {
        harness.key(KeyCode::Char(character));
    }
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Reasoning effort set to max");
    assert_eq!(effort(&mut harness).as_deref(), Some("max"));

    harness.type_and_submit("/effort default");
    harness.pump_until_screen_shows("Reasoning effort set to the Provider's default");
    assert_eq!(effort(&mut harness), None);
}

/// The model policy's window, unset, names the window the default model's
/// listing reported; the model list shows what each entry reports, in
/// columns, with where it came from under the list; and
/// `/status` shows a session's window with its source, or that it is
/// unknown; `/model forget` says when nothing was learned.
#[test]
fn the_reported_window_is_shown_in_the_policy_the_picker_and_status() {
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
    let picker = harness.pump_until_screen_shows("200K  ?         yes");
    assert!(picker.contains("fixture-model (current)"), "{picker}");
    let detail: String = picker.split_whitespace().collect();
    assert!(
        detail.contains("context200K(reported)") && detail.contains("visionyes(reported)"),
        "{picker}"
    );
    harness.key(KeyCode::Esc);
    harness.type_and_submit("/providers policy");
    harness.pump_until_screen_shows("Settings › Model policy");
    harness.select_setting("Context window");
    harness.pump_until_screen_shows("Unset, the model's: 200000 (reported)");
    harness.app.close_settings_center(&mut harness.tui);

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

/// The test Provider's listing, held: the default model with a window and
/// images, and another reporting nothing.
fn held_listing(release: &Arc<Notify>) -> FakeListing {
    FakeListing::ok(
        r#"{"data":[{"id":"fixture-model","context_window":200000,"input_modalities":["text","image"]},{"id":"first-model"}]}"#,
    )
    .held(release.clone())
}

/// The test Provider's record in the center, at `action`.
fn at_provider_action(harness: &mut Harness, action: &str) {
    harness.type_and_submit("/providers");
    harness.pump_until_screen_shows("Settings › Providers");
    harness.select_setting("Fixture Provider");
    harness.key(KeyCode::Enter);
    harness.select_setting(action);
    harness.key(KeyCode::Enter);
}

/// The Provider page's model list shows the loading line until its listing
/// answers, then its models in `/model`'s columns with the selected one's
/// facts under them; the model chosen is saved as the policy's.
#[test]
fn the_provider_page_lists_with_the_loading_line_then_the_fact_columns() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let release = Arc::new(Notify::new());
        let mut harness = Harness::listing(vec![held_listing(&release)], mode, |_| {});
        at_provider_action(&mut harness, "Choose the model");
        let screen = harness.pump_until_screen_shows("Listing the models…");
        assert!(!screen.contains("first-model"), "{mode:?}:\n{screen}");

        release.notify_one();
        let screen = harness.pump_until_screen_shows("first-model");
        assert!(
            !screen.contains("Listing the models…"),
            "{mode:?}:\n{screen}"
        );
        assert!(screen.contains("Context  Thinking  Vision"), "{screen}");
        let row = |text: &str| {
            screen
                .lines()
                .find(|line| line.contains(text))
                .unwrap_or_else(|| panic!("no row {text}:\n{screen}"))
                .to_string()
        };
        assert!(
            row("fixture-model (current)").contains("200K  ?         yes"),
            "{screen}"
        );
        assert!(row("first-model").contains("?  ?         ?"), "{screen}");
        let unwrapped: String = screen.split_whitespace().collect();
        assert!(
            unwrapped.contains("fixture-model·FixtureProvider:context200K(reported)"),
            "{screen}"
        );

        // The listing comes sorted, first-model above the default one.
        harness.key(KeyCode::Up);
        harness.key(KeyCode::Enter);
        harness.pump_until("the model to be saved", |harness| {
            harness.registry().model_policy.model == "first-model"
        });
    }
}

/// List models says so while its listing is held, then what it listed.
#[test]
fn list_models_says_it_lists_until_the_provider_answers() {
    let release = Arc::new(Notify::new());
    let mut harness = Harness::listing(
        vec![held_listing(&release)],
        TranscriptMode::Terminal,
        |_| {},
    );
    at_provider_action(&mut harness, "List models");
    let screen = harness.pump_until_screen_shows("List models…");
    assert!(!screen.contains("Saving…"), "{screen}");
    release.notify_one();
    harness.pump_until_screen_shows("2 models: first-model, fixture-model");
}

#[test]
fn the_configuration_center_opens_the_model_policy_directly() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/config modelPolicy");
    harness.pump_until_screen_shows("Settings › Model policy");
}

#[test]
fn the_providers_command_opens_the_model_policy_directly() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/providers policy");
    harness.pump_until_screen_shows("Model policy");
}

#[test]
fn an_mcp_server_is_added_with_a_secret_and_deleted_in_the_center() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/mcp");
    harness.pump_until_screen_shows("Settings › MCP servers");
    harness.key(KeyCode::Char('a'));
    harness.pump_until_screen_shows("MCP servers › Add a server");
    harness.edit_setting("Server id", "tools");
    // Off, so nothing is started: it starts on, as an add writes it.
    harness.select_setting("Enabled");
    harness.pump_until_screen_shows("[x] On");
    harness.key(KeyCode::Enter);
    harness.select_setting("Transport");
    harness.key(KeyCode::Enter);
    harness.select_setting("1. Standard input and output");
    harness.key(KeyCode::Enter);
    harness.edit_setting("Program", "sh");
    harness.select_setting("Arguments");
    harness.key(KeyCode::Enter);
    for argument in ["-c", "exit 0"] {
        harness.select_setting("+ Add an item");
        harness.key(KeyCode::Enter);
        for character in argument.chars() {
            harness.key(KeyCode::Char(character));
        }
        harness.key(KeyCode::Enter);
    }
    harness.key(KeyCode::Esc);
    harness.select_setting("Add a server");
    harness.key(KeyCode::Enter);
    harness.pump_until("the server to be added", |harness| {
        !harness.mcp().settings.servers.is_empty()
    });

    // Its secret, in MCP secrets.
    harness.pump_until_screen_shows("› tools");
    harness.key(KeyCode::Esc);
    harness.select_setting("MCP secrets");
    harness.key(KeyCode::Enter);
    harness.key(KeyCode::Char('a'));
    harness.edit_setting("Server and name", "tools/API_KEY");
    harness.edit_setting("Value", "hunter2");
    harness.select_setting("Add a secret");
    harness.key(KeyCode::Enter);
    let screen = harness.pump_until_screen_shows("tools · API_KEY");
    assert!(!screen.contains("hunter2"), "{screen}");

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
    assert_eq!(stdio.credential_environment["API_KEY"], key);
    assert_eq!(settings.credentials[&key].value, "hunter2");

    // Deleted after asking, its secret with it.
    harness.key(KeyCode::Esc);
    harness.select_setting("MCP servers");
    harness.key(KeyCode::Enter);
    harness.select_setting("tools ");
    harness.key(KeyCode::Enter);
    harness.select_setting("Delete");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Settings › Delete?");
    harness.select_setting("2. Delete");
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
fn an_mcp_login_opens_over_its_server_and_says_why_it_failed() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/mcp login nope");
    harness.pump_until_screen_shows("There is no `nope` to open.");
    harness.app.close_settings_center(&mut harness.tui);
    let revision = harness.mcp().settings.revision;
    let add = serde_json::from_value(serde_json::json!({
        "collection": "mcp",
        "record": "docs",
        "action": "add",
        "expectedRevision": revision,
        "values": {
            "enabled": false,
            "transport.type": "streamable_http",
            "transport.url": "http://127.0.0.1:9/mcp",
        },
    }))
    .unwrap();
    let agent = harness.runtime.slot().agent().unwrap();
    let call = agent.clone();
    agent
        .runtime()
        .block_on(async move { call.run_settings_action(add).await })
        .unwrap()
        .unwrap();
    harness.type_and_submit("/mcp login docs");
    harness.pump_until_screen_shows("OAuth is disabled for this MCP server");
    // Esc goes back to the server's form, which has the login.
    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("MCP servers › docs");
    assert!(harness.screen().contains("Log in"));

    // A login that ends on its own (logged in) closes its page, with no
    // key to say so: the next key is the form's, never the composer's.
    harness.select_setting("Log in");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("OAuth is disabled for this MCP server");
    let docs = McpServerId::parse("docs").unwrap();
    let logged_in = AppEvent::AgentReply(AgentReply::Update(Box::new(TurnUpdate::Mcp(
        McpReply::LoggedIn(docs),
    ))));
    assert!(harness.app.handle_event(&mut harness.tui, logged_in));
    harness.pump_until("the login's page to close", |harness| {
        harness
            .app
            .workspace
            .settings()
            .is_some_and(|center| center.hosted_command().is_none())
    });
    harness.key(KeyCode::Char('x'));
    assert!(harness.app.chat_widget.composer_is_empty());
}

#[test]
fn the_global_model_field_lists_facts_and_accepts_a_manual_id() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let listing = || {
            FakeListing::ok(
                r#"{"data":[{"id":"fixture-model","context_window":200000,"input_modalities":["text","image"]},{"id":"first-model"}]}"#,
            )
        };
        let mut harness = Harness::listing(vec![listing(), listing()], mode, |_| {});
        harness.type_and_submit("/config modelPolicy");
        harness.pump_until_screen_shows("Default Provider");
        harness.select_setting("Model");
        harness.key(KeyCode::Enter);
        let screen = harness.pump_until_screen_shows("first-model");
        assert!(screen.contains("Context  Thinking  Vision"), "{screen}");
        assert!(screen.contains("200K"), "{screen}");
        harness.key(KeyCode::Up);
        harness.key(KeyCode::Enter);
        harness.pump_until("the global model is saved", |h| {
            h.registry().model_policy.model == "first-model"
        });
        harness.pump_until_screen_shows("Default Provider");
        harness.select_setting("Model");
        harness.key(KeyCode::Enter);
        harness.pump_until_screen_shows("Enter a model id…");
        harness.key(KeyCode::Down);
        harness.key(KeyCode::Down);
        harness.key(KeyCode::Enter);
        harness.pump_until_screen_shows("Use this model");
        for character in "manual-model".chars() {
            harness.key(KeyCode::Char(character));
        }
        harness.key(KeyCode::Tab);
        harness.key(KeyCode::Enter);
        harness.pump_until("the manual model is saved", |h| {
            h.registry().model_policy.model == "manual-model"
        });
        assert_eq!(harness.app.bridge.session_id(), None);
    }
}
