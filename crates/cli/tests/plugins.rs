//! `plugins` and `plugins marketplace`, run as the built binary in a
//! temporary home: a local marketplace added, its plugin installed, listed,
//! turned off and on, updated, shown and removed; a plugin installed from a
//! directory; its MCP server in `mcp list`, and none in safe mode.

mod support;

use std::path::Path;

use serde_json::{json, Value};
use support::Home;

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// A Claude Code plugin `name` at `root`: a Skill, a hook and an MCP server
/// whose program does not exist (it is listed, connected or not).
fn plugin(root: &Path, name: &str, version: &str) {
    write(
        &root.join(".claude-plugin/plugin.json"),
        &json!({ "name": name, "version": version, "description": "A demo plugin." }).to_string(),
    );
    write(
        &root.join("skills/greet/SKILL.md"),
        "---\nname: greet\ndescription: Greets.\n---\nSay hello.\n",
    );
    write(
        &root.join("hooks/hooks.json"),
        &json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "true" }] }] } })
            .to_string(),
    );
    write(
        &root.join(".mcp.json"),
        &json!({ "mcpServers": { format!("{name}-records"): { "command": "no-such-mcp-program" } } })
            .to_string(),
    );
}

fn skills(home: &Home) -> String {
    home.run(&["skills", "list"]).expect(0).stdout()
}

#[test]
fn a_marketplaces_plugin_is_added_listed_turned_off_and_on_updated_and_removed() {
    let home = Home::new();
    let market = home.project.join("market");
    write(
        &market.join(".claude-plugin/marketplace.json"),
        &json!({ "name": "demo-market", "plugins": [{ "name": "demo", "source": "./plugins/demo" }] })
            .to_string(),
    );
    plugin(&market.join("plugins/demo"), "demo", "1.0.0");
    let market_arg = market.to_str().unwrap();

    let added = home
        .run(&["plugins", "marketplace", "add", market_arg])
        .expect(0)
        .stdout();
    assert!(added.contains("Added marketplace `demo-market`"), "{added}");
    let again = home
        .run(&["plugins", "marketplace", "add", market_arg])
        .expect(0)
        .stdout();
    assert!(again.contains("is already added"), "{again}");
    let listed = home
        .run(&["plugins", "marketplace", "list"])
        .expect(0)
        .stdout();
    assert!(listed.contains("demo-market  local, on"), "{listed}");
    let available = home
        .run(&["plugins", "list", "--available"])
        .expect(0)
        .stdout();
    assert!(
        available.contains("demo@demo-market  not installed"),
        "{available}"
    );

    // A selection that cannot be used fails before the Agent starts.
    home.run(&["plugins", "add", "demo"]).expect(2);
    home.run(&[
        "plugins",
        "add",
        "demo@demo-market",
        "--marketplace",
        "other",
    ])
    .expect(2);
    let installed = home
        .run(&["plugins", "add", "demo", "--marketplace", "demo-market"])
        .expect(0)
        .stdout();
    assert!(
        installed.contains("Added plugin `demo@demo-market` (version 1.0.0)."),
        "{installed}"
    );
    assert!(
        installed.contains("1 of its hooks wait to be trusted"),
        "{installed}"
    );
    assert!(home.file("config.toml").contains("enabled = true"));
    let list = home.run(&["plugins", "list"]).expect(0).stdout();
    assert!(list.contains("demo@demo-market  enabled  1.0.0"), "{list}");
    assert!(skills(&home).contains("plugin:demo@demo-market:greet"));
    let mcp = home.run(&["mcp", "list"]).expect(0).stdout();
    let server = mcp
        .lines()
        .find(|line| line.starts_with("demo-records  stdio no-such-mcp-program"));
    assert!(
        server.is_some_and(|line| line.ends_with("(plugin demo@demo-market)")),
        "{mcp}"
    );
    let safe = home.run(&["--safe-mode", "mcp", "list"]).expect(0).stdout();
    assert!(!safe.contains("demo-records"), "{safe}");

    let off = home
        .run(&["plugins", "disable", "demo@demo-market"])
        .expect(0)
        .stdout();
    assert!(off.contains("Plugin `demo@demo-market` is off."), "{off}");
    assert!(!skills(&home).contains("plugin:demo@demo-market:greet"));
    home.run(&["plugins", "enable", "demo@demo-market"])
        .expect(0);
    assert!(skills(&home).contains("plugin:demo@demo-market:greet"));

    plugin(&market.join("plugins/demo"), "demo", "1.1.0");
    let updated = home
        .run(&["plugins", "update", "demo@demo-market"])
        .expect(0)
        .stdout();
    assert!(updated.contains("(version 1.1.0)"), "{updated}");
    let info = home
        .run(&["plugins", "info", "demo@demo-market", "--json"])
        .expect(0)
        .stdout();
    let info: Value = serde_json::from_str(&info).unwrap();
    assert_eq!(info["version"], "1.1.0");
    assert_eq!(info["enabled"], true);
    assert_eq!(
        info["content"]["skills"][0],
        "plugin:demo@demo-market:greet"
    );
    assert_eq!(info["hooksToTrust"].as_array().unwrap().len(), 1);
    let retried = home
        .run(&["plugins", "retry", "demo@demo-market"])
        .expect(0)
        .stdout();
    assert!(retried.contains("status:      enabled"), "{retried}");

    let removed = home
        .run(&["plugins", "remove", "demo@demo-market", "--purge-data"])
        .expect(0)
        .stdout();
    assert!(
        removed.contains("Removed plugin `demo@demo-market`."),
        "{removed}"
    );
    assert!(removed.contains("Deleted its data"), "{removed}");
    let list = home.run(&["plugins", "list"]).expect(0).stdout();
    assert!(list.contains("No plugins are installed."), "{list}");
    let run = home
        .run(&["plugins", "remove", "demo@demo-market"])
        .expect(1);
    assert!(
        run.stderr().contains("is not installed"),
        "{}",
        run.stderr()
    );

    home.run(&["plugins", "marketplace", "remove", "demo-market"])
        .expect(0);
    let listed = home
        .run(&["plugins", "marketplace", "list", "--json"])
        .expect(0)
        .stdout();
    assert_eq!(serde_json::from_str::<Value>(&listed).unwrap(), json!([]));
}

#[test]
fn a_directory_is_installed_under_local_or_the_marketplace_named() {
    let home = Home::new();
    let source = home.project.join("solo");
    plugin(&source, "solo", "0.1.0");

    let installed = home
        .run(&["plugins", "add", source.to_str().unwrap(), "--json"])
        .expect(0)
        .stdout();
    let installed: Value = serde_json::from_str(&installed).unwrap();
    assert_eq!(installed["package"]["id"], "solo@local");
    assert_eq!(installed["permissions"], json!([]));
    home.run(&[
        "plugins",
        "add",
        source.to_str().unwrap(),
        "--marketplace",
        "mine",
    ])
    .expect(0);
    let list = home.run(&["plugins", "list", "--json"]).expect(0).stdout();
    let list: Value = serde_json::from_str(&list).unwrap();
    let ids: Vec<&str> = list["installed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|package| package["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["solo@local", "solo@mine"]);
}
