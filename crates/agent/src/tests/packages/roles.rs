//! Installed plugins' roles (site/content/docs/roles.en.mdx, "Plugins'
//! roles"): a Claude Code package's agent files and a role file of this
//! project's format enter the catalog as `<plugin>:<role>` while the
//! package is on, and leave it once it is off; what their reading left out
//! is the package's issue; a second package of the same plugin name gives
//! way; a session runs in a plugin's role, and the model is offered it.

use hanafubuki_agent_kernel::{AgentSessionMode, ToolIdentity};
use serde_json::Value;

use super::*;

const REVIEWER: &str = "---\nname: reviewer\ndescription: Reviews a change; never edits.\n\
tools: Read, Grep, Task\nmodel: sonnet\ncolor: green\n---\n\nReview the diff and list the \
problems.\n";
/// Every tool it lists is one this Agent has no counterpart for.
const PLANNER: &str = "---\ndescription: Plans only.\ntools: Task, TodoWrite\nmodel: inherit\n\
---\nPlan.\n";

/// A Claude Code plugin `name` under `paths` whose `agents/` holds the
/// reviewer and the planner.
fn claude_agents_plugin(paths: &TestPaths, name: &str, directory: &str) -> PathBuf {
    let root = paths.app_data_dir.join(directory).join(name);
    write(
        &root.join(".claude-plugin/plugin.json"),
        &json!({ "name": name, "version": "1.0.0" }).to_string(),
    );
    write(&root.join("agents/reviewer.md"), REVIEWER);
    write(&root.join("agents/planning/planner.md"), PLANNER);
    root
}

/// A plugin of this project's format whose `agents` are `./roles`.
fn own_format_plugin(paths: &TestPaths) -> PathBuf {
    let root = paths.app_data_dir.join("sources").join("kit");
    write(
        &root.join(".hanafubuki-plugin/plugin.json"),
        &json!({ "name": "kit", "version": "1.0.0", "agents": "./roles" }).to_string(),
    );
    write(
        &root.join("roles/lead.toml"),
        "description = \"Leads.\"\nextends = \"explorer\"\nspawnable = false\nmood = 1\n",
    );
    // Not the declared path: never read.
    write(
        &root.join("agents/ignored.toml"),
        "description = \"Ignored.\"\n",
    );
    root
}

fn tool(name: &str) -> ToolIdentity<'_> {
    ToolIdentity {
        name,
        owner: "workspace-fs",
        provider: None,
    }
}

fn plugin_roles(service: &AgentApplicationService) -> Vec<String> {
    service
        .roles()
        .roles()
        .map(|role| role.name.clone())
        .filter(|name| name.contains(':'))
        .collect()
}

#[tokio::test]
async fn a_plugins_roles_are_in_the_catalog_while_it_is_on() {
    let server = FakeServer::start(vec![(assistant_events("Reviewed."), Duration::ZERO)]).await;
    let (service, paths, config) = service_with_config("packages-roles", |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    install(
        &paths,
        &claude_agents_plugin(&paths, "demo", "sources"),
        "demo@local",
    );
    install(&paths, &own_format_plugin(&paths), "kit@local");
    set_enabled(&config, "demo@local", false);
    service.reload_packages().await.unwrap();
    assert_eq!(plugin_roles(&service), ["kit:lead"]);
    assert!(package(&service, "demo@local")
        .await
        .content
        .roles
        .is_empty());

    set_enabled(&config, "demo@local", true);
    service.reload_packages().await.unwrap();

    assert_eq!(
        plugin_roles(&service),
        ["demo:planner", "demo:reviewer", "kit:lead"]
    );
    let catalog = service.roles();
    let reviewer = catalog.role("demo:reviewer").unwrap();
    assert_eq!(reviewer.description, "Reviews a change; never edits.");
    assert_eq!(
        reviewer.instructions.as_deref(),
        Some("Review the diff and list the problems.")
    );
    assert_eq!(reviewer.model, None);
    assert!(reviewer.spawnable);
    assert!(reviewer.scope.tools.allows(&tool("read_file")));
    assert!(reviewer.scope.tools.allows(&tool("search_text")));
    assert!(!reviewer.scope.tools.allows(&tool("exec_command")));
    assert!(reviewer
        .source
        .to_string()
        .starts_with("plugin `demo@local` file "));
    // A `tools` that maps to nothing allows nothing; it never widens.
    let planner = catalog.role("demo:planner").unwrap();
    assert!(!planner.scope.tools.allows(&tool("read_file")));
    let lead = catalog.role("kit:lead").unwrap();
    assert!(!lead.spawnable);
    assert!(lead.scope.tools.allows(&tool("read_file")));
    assert!(!lead.scope.tools.allows(&tool("apply_patch")));
    assert!(catalog.role("kit:ignored").is_none());

    let demo = package(&service, "demo@local").await;
    assert_eq!(demo.content.roles, ["demo:planner", "demo:reviewer"]);
    let expected = [
        "role `demo:planner`: the tool `Task` has no counterpart in this Agent; it is left out",
        "role `demo:planner`: the tool `TodoWrite` has no counterpart in this Agent; it is left \
         out",
        "role `demo:reviewer`: the tool `Task` has no counterpart in this Agent; it is left out",
        "role `demo:reviewer`: `model` `sonnet` is ignored: Claude's models are not this \
         Agent's, so the model is inherited",
    ];
    assert!(!demo.issues.iter().any(|issue| issue.contains("color")));
    for issue in expected {
        assert!(
            demo.issues.iter().any(|seen| seen == issue),
            "{issue}: {:?}",
            demo.issues
        );
    }
    let kit = package(&service, "kit@local").await;
    assert_eq!(kit.content.roles, ["kit:lead"]);
    assert!(
        kit.issues
            .iter()
            .any(|issue| issue.starts_with("role `kit:lead`: `mood`")),
        "{:?}",
        kit.issues
    );

    let session = service
        .create_session(AgentSessionMode::Default, Some("demo:reviewer".to_string()))
        .await
        .unwrap();
    assert_eq!(session.role.as_deref(), Some("demo:reviewer"));
    service
        .start_turn(session.id, "Review it.".to_string().into())
        .await
        .unwrap();
    let request: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    let tools: Vec<&str> = request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(tools.contains(&"read_file") && tools.contains(&"search_text"));
    assert!(!tools.contains(&"exec_command") && !tools.contains(&"apply_patch"));
    let text = request.to_string();
    assert!(text.contains("Review the diff and list the problems."));
    // The spawnable plugin roles are offered to the model; `kit:lead` is not
    // spawnable.
    assert!(text.contains("`demo:reviewer`"), "{text}");
    assert!(!text.contains("`kit:lead`"));

    set_enabled(&config, "demo@local", false);
    service.reload_packages().await.unwrap();

    assert_eq!(plugin_roles(&service), ["kit:lead"]);
    let refused = service
        .create_session(AgentSessionMode::Default, Some("demo:reviewer".to_string()))
        .await
        .unwrap_err();
    assert!(
        refused.contains("no role is named `demo:reviewer`"),
        "{refused}"
    );
    finish(service, paths).await;
}

#[tokio::test]
async fn of_two_plugins_of_one_name_the_first_by_id_keeps_its_roles() {
    let (service, paths, config) = service_with_config("packages-roles-twins", |_, _| {});
    install(
        &paths,
        &claude_agents_plugin(&paths, "demo", "first"),
        "demo@alpha",
    );
    let second = claude_agents_plugin(&paths, "demo", "second");
    write(
        &second.join("agents/extra.md"),
        "---\ndescription: Extra.\n---\n",
    );
    install(&paths, &second, "demo@beta");
    set_enabled(&config, "demo@alpha", true);
    set_enabled(&config, "demo@beta", true);
    service.reload_packages().await.unwrap();

    assert_eq!(plugin_roles(&service), ["demo:planner", "demo:reviewer"]);
    assert!(service
        .roles()
        .role("demo:reviewer")
        .unwrap()
        .source
        .to_string()
        .starts_with("plugin `demo@alpha` "));
    let beta = package(&service, "demo@beta").await;
    assert!(beta.content.roles.is_empty());
    assert!(beta.issues.iter().any(|issue| issue
        == "its roles are left out: `demo@alpha` comes first, and its roles are named `demo:…` \
            already"));

    set_enabled(&config, "demo@alpha", false);
    service.reload_packages().await.unwrap();
    assert_eq!(
        plugin_roles(&service),
        ["demo:extra", "demo:planner", "demo:reviewer"]
    );
    finish(service, paths).await;
}

#[test]
fn validating_a_plugin_lists_what_its_roles_leave_out() {
    let root = tempfile::tempdir().unwrap();
    let plugin = root.path().join("demo");
    write(
        &plugin.join(".claude-plugin/plugin.json"),
        &json!({ "name": "demo" }).to_string(),
    );
    write(&plugin.join("agents/reviewer.md"), REVIEWER);

    let validation = crate::validate_package(&plugin).unwrap();

    assert!(
        validation.issues.iter().any(|issue| issue
            == "role `demo:reviewer`: the tool `Task` has no counterpart in this Agent; it is \
                left out"),
        "{:?}",
        validation.issues
    );
}
