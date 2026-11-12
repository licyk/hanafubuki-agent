//! `/plugins` and `/mcp` end to end over the standalone Agent: a plugin
//! installed from a marketplace, its hook trusted, turned off and on and
//! uninstalled with its data; a built-in plugin turned off, refused a
//! retry, and on again; a plugin's MCP server in `/mcp verbose`; a
//! plugin whose code failed in `/warnings`.

use std::path::Path;

use hanafubuki_agent::{AgentHandle, AgentPackageSource, AgentPluginStatus, MarketplaceAddRequest};
use ratatui::crossterm::event::KeyCode;
use serde_json::json;

use super::Harness;

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

impl Harness {
    fn agent(&self) -> AgentHandle {
        self.runtime.slot().agent().unwrap()
    }

    /// What `call` makes of the Agent, run on its runtime.
    fn on_agent<T, F>(&self, call: impl FnOnce(AgentHandle) -> F) -> T
    where
        T: Send + 'static,
        F: std::future::Future<Output = Result<T, String>> + Send + 'static,
    {
        let agent = self.agent();
        agent
            .runtime()
            .block_on(call(agent.clone()))
            .unwrap()
            .unwrap()
    }

    /// A marketplace `demo-market` offering the Claude Code plugin `demo`
    /// (a Skill, a Stop hook and an MCP server whose program does not
    /// exist), added through the facade.
    fn add_marketplace(&self) {
        let market = self.home.path().join("market");
        write(
            &market.join(".claude-plugin/marketplace.json"),
            &json!({ "name": "demo-market", "plugins": [{ "name": "demo", "source": "./plugins/demo" }] })
                .to_string(),
        );
        let plugin = market.join("plugins/demo");
        write(
            &plugin.join(".claude-plugin/plugin.json"),
            &json!({ "name": "demo", "version": "1.0.0", "description": "The demo plugin." })
                .to_string(),
        );
        write(
            &plugin.join("skills/greet/SKILL.md"),
            "---\nname: greet\ndescription: Greets.\n---\nSay hello.\n",
        );
        write(
            &plugin.join("hooks/hooks.json"),
            &json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "true" }] }] } })
                .to_string(),
        );
        write(
            &plugin.join(".mcp.json"),
            &json!({ "mcpServers": { "records": { "command": "no-such-mcp-program" } } })
                .to_string(),
        );
        let source = market.display().to_string();
        self.on_agent(|agent| async move {
            agent
                .add_marketplace(MarketplaceAddRequest {
                    source,
                    ..MarketplaceAddRequest::default()
                })
                .await
        });
    }

    fn plugin_ids(&self) -> Vec<String> {
        self.on_agent(|agent| async move { agent.packages().await })
            .into_iter()
            .map(|package| package.id)
            .collect()
    }

    fn builtins(&self) -> Vec<AgentPluginStatus> {
        self.agent().plugin_status()
    }

    fn keys(&mut self, code: KeyCode, count: usize) {
        for _ in 0..count {
            self.key(code);
        }
    }
}

#[test]
fn a_marketplaces_plugin_is_installed_trusted_turned_off_and_on_and_uninstalled() {
    let mut harness = Harness::start(Vec::new());
    harness.add_marketplace();
    harness.type_and_submit("/plugins");
    harness.pump_until_screen_shows("[Installed (0)]");

    harness.key(KeyCode::Right);
    harness.pump_until_screen_shows("[Marketplaces (1)]");
    harness.pump_until_screen_shows("0 of 1 installed");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("demo-market marketplace");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Install plugin");
    harness.key(KeyCode::Down);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("run their commands only once trusted");
    assert!(harness.screen().contains("Installed demo@demo-market."));
    assert_eq!(harness.plugin_ids(), ["demo@demo-market"]);

    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Trusted 1 hook(s).");
    let package = harness.on_agent(|agent| async move { agent.package("demo@demo-market").await });
    assert!(package.unwrap().hooks_to_trust.is_empty());
    harness.key(KeyCode::Down);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("demo@demo-market is off.");
    assert!(harness.screen().contains("Enable plugin"));
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("demo@demo-market is on.");

    // Back, the toggle, update, retry, then uninstall.
    harness.keys(KeyCode::Down, 3);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Uninstall demo@demo-market?");
    harness.keys(KeyCode::Down, 2);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Uninstalled demo@demo-market and deleted its data.");
    assert!(harness.screen().contains("[Installed (0)]"));
    assert!(harness.plugin_ids().is_empty());
}

#[test]
fn a_built_in_plugin_is_turned_off_refused_a_retry_and_turned_on() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/plugins");
    harness.pump_until_screen_shows("[Installed (0)]");
    harness.key(KeyCode::Left);
    harness.pump_until_screen_shows("[Built-in (");
    let goal = harness
        .builtins()
        .iter()
        .position(|plugin| plugin.name == "goal")
        .unwrap();
    harness.keys(KeyCode::Down, goal);

    harness.key(KeyCode::Char(' '));
    harness.pump_until_screen_shows("The built-in plugin goal is off.");
    assert!(!harness.builtins()[goal].enabled);
    harness.key(KeyCode::Char('r'));
    harness.pump_until_screen_shows("goal is disabled");
    harness.key(KeyCode::Char(' '));
    harness.pump_until_screen_shows("The built-in plugin goal is on.");
    assert!(harness.builtins()[goal].enabled);
}

#[test]
fn mcp_verbose_lists_a_plugins_server() {
    let mut harness = Harness::start(Vec::new());
    harness.add_marketplace();
    harness.on_agent(|agent| async move {
        agent
            .install_package(AgentPackageSource::Marketplace {
                marketplace: "demo-market".to_string(),
                plugin: "demo".to_string(),
            })
            .await
    });

    harness.type_and_submit("/mcp verbose");
    harness.pump_until("the report", |harness| {
        harness
            .transcript()
            .contains("records (plugin demo@demo-market)")
    });
}

#[test]
fn warnings_show_a_plugin_whose_code_failed() {
    let mut harness = Harness::start(Vec::new());
    // A WebAssembly plugin, which runs without a sandbox, whose module
    // cannot load: its guest fails to start.
    let source = harness.home.path().join("sources/broken");
    write(
        &source.join(".hanafubuki-plugin/plugin.json"),
        &json!({
            "name": "broken",
            "version": "1.0.0",
            "runtime": { "type": "wasm", "module": "./broken.wasm" },
        })
        .to_string(),
    );
    write(&source.join("broken.wasm"), "not a module");
    harness.on_agent(|agent| async move {
        agent
            .install_package(AgentPackageSource::Directory {
                path: source,
                marketplace: None,
            })
            .await
    });
    harness.pump_until("the guest to fail", |harness| {
        harness
            .on_agent(|agent| async move { agent.packages().await })
            .iter()
            .any(|package| {
                package
                    .guest
                    .as_ref()
                    .is_some_and(|guest| guest.status.state == hanafubuki_agent::GuestState::Failed)
            })
    });

    harness.type_and_submit("/warnings");
    let list = harness.pump_until_screen_shows("d dismiss");
    assert!(
        list.contains("[installed plugins] Installed plugin `broken@local`: its code failed:"),
        "{list}"
    );
}
