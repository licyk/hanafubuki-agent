//! Agent Plugins' rules and a broken package.

use hanafubuki_agent_packages::AGENT_PLUGINS_SCHEMA;

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_plugins_packages_skills_are_direct_children_and_its_hooks_never_run() {
    let (service, paths, _) = service_with_config("packages-agent-plugins", |_, _| {});
    let source = paths.app_data_dir.join("sources/portable");
    write(
        &source.join("plugin.json"),
        &json!({ "$schema": AGENT_PLUGINS_SCHEMA, "name": "portable", "version": "1.0.0" })
            .to_string(),
    );
    let document = "---\nname: direct\ndescription: A direct child.\n---\nBody.\n";
    write(&source.join("skills/direct/SKILL.md"), document);
    write(&source.join("skills/group/nested/SKILL.md"), document);
    write(
        &source.join(".codex-plugin/plugin.json"),
        &json!({ "name": "portable", "hooks": "./hooks.json" }).to_string(),
    );
    write(
        &source.join("hooks.json"),
        &json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "true" }] }] } })
            .to_string(),
    );
    // Nothing listens there: the server is the package's, connected or not.
    write(
        &source.join("mcp.json"),
        &json!({
            "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
            "mcpServers": { "portable": { "type": "streamable-http", "url": "http://127.0.0.1:9/mcp" } },
        })
        .to_string(),
    );
    install(&paths, &source, "portable@local");

    service.reload_packages().await.unwrap();

    assert_eq!(plugin_skill_ids(&service), ["plugin:portable@local:direct"]);
    assert!(service.hooks_list().await.unwrap().hooks.is_empty());
    let status = package(&service, "portable@local").await;
    assert_eq!(status.content.mcp_servers, ["portable"]);
    assert!(status
        .issues
        .contains(&"an Agent Plugins package's hooks are not run".to_string()));
    let snapshot = service.mcp_snapshot().await.unwrap();
    assert_eq!(snapshot.plugin_servers[0].plugin, "portable@local");
    finish(service, paths).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broken_package_or_component_costs_only_itself() {
    let (service, paths, _) = service_with_config("packages-broken", |_, _| {});
    let good = claude_plugin(&paths, "good", json!({}), None);
    write(&good.join(".mcp.json"), "{ not json");
    let broken = claude_plugin(&paths, "broken", json!({}), None);
    install(&paths, &good, "good@local");
    let broken_root = install(&paths, &broken, "broken@local");
    write(&broken_root.join(".claude-plugin/plugin.json"), "{");

    service.reload_packages().await.unwrap();

    assert_eq!(plugin_skill_ids(&service), ["plugin:good@local:search"]);
    let good = package(&service, "good@local").await;
    assert!(good.issues.iter().any(
        |issue| issue.starts_with("MCP server `*` is left out:") && issue.contains(".mcp.json")
    ));
    let broken = package(&service, "broken@local").await;
    assert!(broken.enabled);
    assert_eq!(broken.format, None);
    assert_eq!(broken.content, crate::AgentPackageContent::default());
    assert!(broken
        .issues
        .iter()
        .any(|issue| issue.starts_with("invalid plugin.json")));
    finish(service, paths).await;
}
