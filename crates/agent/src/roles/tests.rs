use std::collections::BTreeSet;
use std::path::PathBuf;

use hanafubuki_agent_config::{InstructionsMode, RoleDefinition, RoleSource};
use hanafubuki_agent_kernel::{AgentPermissionMode, ToolIdentity, ToolPattern};

use super::resolve::resolve;
use super::*;

fn role(name: &str) -> RoleDefinition {
    let mut role = RoleDefinition::empty(
        name.to_string(),
        RoleOrigin::File {
            path: PathBuf::from(format!("/roles/{name}.toml")),
        },
    );
    role.description = Some(format!("{name}'s description"));
    role
}

fn extending(name: &str, parent: &str) -> RoleDefinition {
    let mut role = role(name);
    role.extends = Some(parent.to_string());
    role
}

fn source(layer: RoleLayer, definitions: Vec<RoleDefinition>) -> RoleSource {
    RoleSource {
        layer,
        definitions,
        issues: Vec::new(),
    }
}

fn user(definitions: Vec<RoleDefinition>) -> Vec<RoleSource> {
    vec![source(RoleLayer::User, definitions)]
}

fn texts(patterns: &[&str]) -> Vec<String> {
    patterns.iter().map(|text| text.to_string()).collect()
}

fn tool(name: &str) -> ToolIdentity<'_> {
    ToolIdentity {
        name,
        owner: "workspace-fs",
        provider: None,
    }
}

fn usable<'a>(catalog: &'a AgentRoleCatalog, name: &str) -> &'a AgentResolvedRole {
    catalog
        .role(name)
        .unwrap_or_else(|| panic!("{name} is not usable: {:#?}", catalog.issues()))
}

fn unusable<'a>(catalog: &'a AgentRoleCatalog, name: &str) -> &'a str {
    &catalog
        .unusable_role(name)
        .unwrap_or_else(|| panic!("{name} is usable"))
        .reason
}

fn provenance<'a>(role: &'a AgentResolvedRole, field: &str) -> Vec<&'a str> {
    role.provenance
        .get(field)
        .map(|roles| roles.iter().map(String::as_str).collect())
        .unwrap_or_default()
}

#[test]
fn the_built_in_roles_are_default_and_explorer() {
    let catalog = resolve(Vec::new());
    let names: Vec<_> = catalog.roles().map(|role| role.name.as_str()).collect();
    assert_eq!(names, [DEFAULT_ROLE, EXPLORER_ROLE]);
    assert!(catalog.issues().is_empty());

    let default = usable(&catalog, DEFAULT_ROLE);
    assert_eq!(default.source, AgentRoleSource::BuiltIn);
    assert_eq!(default.scope.tools, Default::default());
    assert!(default.scope.plugins_off.is_empty());
    assert!(default.spawnable);

    let explorer = usable(&catalog, EXPLORER_ROLE);
    assert!(explorer.spawnable);
    assert!(explorer.description.contains("`explorer`"));
    for name in [
        "list_directory",
        "search_text",
        "read_file",
        "view_image",
        "web_search",
        "fetch_web_page",
    ] {
        assert!(explorer.scope.tools.allows(&tool(name)), "{name}");
    }
    for name in ["apply_patch", "exec_command", "spawn_agent"] {
        assert!(!explorer.scope.tools.allows(&tool(name)), "{name}");
    }
}

#[test]
fn description_model_effort_and_spawnable_are_the_childs_else_the_parents() {
    let mut parent = role("parent");
    parent.model = Some("parent-model".into());
    parent.reasoning_effort = Some(ModelReasoningEffort::High);
    parent.spawnable = Some(false);
    let mut child = extending("child", "parent");
    child.description = None;
    let mut grandchild = extending("grandchild", "child");
    grandchild.model = Some("own-model".into());
    grandchild.reasoning_effort = Some(ModelReasoningEffort::Low);
    grandchild.spawnable = Some(true);
    let catalog = resolve(user(vec![parent, child, grandchild]));

    let child = usable(&catalog, "child");
    assert_eq!(child.description, "parent's description");
    assert_eq!(child.model.as_deref(), Some("parent-model"));
    assert_eq!(child.reasoning_effort, Some(ModelReasoningEffort::High));
    assert!(!child.spawnable);
    assert_eq!(provenance(child, "description"), ["parent"]);
    assert_eq!(provenance(child, "model"), ["parent"]);

    let grandchild = usable(&catalog, "grandchild");
    assert_eq!(grandchild.description, "grandchild's description");
    assert_eq!(grandchild.model.as_deref(), Some("own-model"));
    assert_eq!(grandchild.reasoning_effort, Some(ModelReasoningEffort::Low));
    assert!(grandchild.spawnable);
    assert_eq!(provenance(grandchild, "spawnable"), ["grandchild"]);
    let chain: Vec<_> = grandchild
        .chain
        .iter()
        .map(|link| link.name.as_str())
        .collect();
    assert_eq!(chain, ["grandchild", "child", "parent"]);
}

#[test]
fn instructions_append_by_default_and_replace_only_the_ancestors() {
    let mut root = role("root");
    root.instructions = Some("Root.".into());
    let mut appended = extending("appended", "root");
    appended.instructions = Some("Appended.".into());
    let mut replaced = extending("replaced", "appended");
    replaced.instructions = Some("Replaced.".into());
    replaced.instructions_mode = InstructionsMode::Replace;
    let mut cleared = extending("cleared", "appended");
    cleared.instructions_mode = InstructionsMode::Replace;
    let silent = extending("silent", "appended");
    let catalog = resolve(user(vec![root, appended, replaced, cleared, silent]));

    let appended = usable(&catalog, "appended");
    assert_eq!(appended.instructions.as_deref(), Some("Root.\n\nAppended."));
    assert_eq!(provenance(appended, "instructions"), ["appended", "root"]);
    let replaced = usable(&catalog, "replaced");
    assert_eq!(replaced.instructions.as_deref(), Some("Replaced."));
    assert_eq!(provenance(replaced, "instructions"), ["replaced"]);
    let cleared = usable(&catalog, "cleared");
    assert_eq!(cleared.instructions, None);
    assert!(provenance(cleared, "instructions").is_empty());
    let silent = usable(&catalog, "silent");
    assert_eq!(silent.instructions.as_deref(), Some("Root.\n\nAppended."));
}

#[test]
fn the_permission_mode_cap_is_the_stricter() {
    let mut root = role("root");
    root.permission_mode_cap = Some(AgentPermissionMode::AutoReview);
    let mut wider = extending("wider", "root");
    wider.permission_mode_cap = Some(AgentPermissionMode::FullAccess);
    let mut stricter = extending("stricter", "root");
    stricter.permission_mode_cap = Some(AgentPermissionMode::Manual);
    let unset = extending("unset", "root");
    let catalog = resolve(user(vec![root, wider, stricter, unset]));

    let wider = usable(&catalog, "wider");
    assert_eq!(
        wider.scope.permission_mode_cap,
        Some(AgentPermissionMode::AutoReview)
    );
    assert_eq!(provenance(wider, "permissionModeCap"), ["root"]);
    let stricter = usable(&catalog, "stricter");
    assert_eq!(
        stricter.scope.permission_mode_cap,
        Some(AgentPermissionMode::Manual)
    );
    assert_eq!(provenance(stricter, "permissionModeCap"), ["stricter"]);
    assert_eq!(
        usable(&catalog, "unset").scope.permission_mode_cap,
        Some(AgentPermissionMode::AutoReview)
    );
}

#[test]
fn allow_is_intersected_and_an_absent_one_is_the_parents() {
    let mut reader = extending("reader", EXPLORER_ROLE);
    reader.tools_allow = Some(texts(&["read_*", "exec_command"]));
    let same = extending("same", EXPLORER_ROLE);
    let mut nothing = extending("nothing", EXPLORER_ROLE);
    nothing.tools_allow = Some(Vec::new());
    let catalog = resolve(user(vec![reader, same, nothing]));

    let reader = usable(&catalog, "reader");
    assert!(reader.scope.tools.allows(&tool("read_file")));
    assert!(!reader.scope.tools.allows(&tool("search_text")));
    // Its own allow list cannot add what explorer leaves out.
    assert!(!reader.scope.tools.allows(&tool("exec_command")));
    assert_eq!(provenance(reader, "tools.allow"), ["reader", EXPLORER_ROLE]);

    let same = usable(&catalog, "same");
    assert_eq!(
        same.scope.tools,
        usable(&catalog, EXPLORER_ROLE).scope.tools
    );
    assert!(!usable(&catalog, "nothing")
        .scope
        .tools
        .allows(&tool("read_file")));
}

#[test]
fn deny_and_plugins_off_are_united_and_attributed_through_the_chain() {
    let mut grandparent = role("grandparent");
    grandparent.tools_deny = texts(&["exec_command"]);
    grandparent.plugins_off = texts(&["titles"]);
    let parent = extending("parent", "grandparent");
    let mut child = extending("child", "parent");
    child.tools_deny = texts(&["apply_patch"]);
    child.plugins_off = texts(&["goal"]);
    let catalog = resolve(user(vec![grandparent, parent, child]));

    let child = usable(&catalog, "child");
    let deny: BTreeSet<_> = ["exec_command", "apply_patch"]
        .into_iter()
        .map(|text| ToolPattern::parse(text).unwrap())
        .collect();
    assert_eq!(child.scope.tools.deny, deny);
    assert_eq!(
        child.scope.plugins_off,
        ["goal".to_string(), "titles".to_string()].into()
    );
    assert_eq!(provenance(child, "tools.deny"), ["child", "grandparent"]);
    assert_eq!(provenance(child, "plugins.off"), ["child", "grandparent"]);
    assert_eq!(child.scope.role.as_deref(), Some("child"));
    assert_eq!(
        provenance(usable(&catalog, "parent"), "tools.deny"),
        ["grandparent"]
    );
}

#[test]
fn a_pattern_that_cannot_be_read_is_left_out_with_an_issue() {
    let mut odd = role("odd");
    odd.tools_allow = Some(texts(&["read_file", "mode:x"]));
    odd.tools_deny = texts(&[""]);
    let catalog = resolve(user(vec![odd]));
    let odd = usable(&catalog, "odd");
    assert_eq!(odd.scope.tools.allow[0].len(), 1);
    assert!(odd.scope.tools.deny.is_empty());
    let issues: Vec<_> = catalog.issues().iter().map(ToString::to_string).collect();
    assert_eq!(issues.len(), 2, "{issues:?}");
    assert!(issues[0].starts_with("role `odd`: user file /roles/odd.toml: `tools.allow`"));
}

/// Narrowing is monotonic: over random chains, a role never allows a tool
/// its parent does not, nor one any role of its chain denies, and its cap
/// is never wider than its parent's.
#[test]
fn a_child_never_allows_what_its_parent_does_not() {
    const POOL: [&str; 10] = [
        "read_file",
        "read_*",
        "search_text",
        "*_text",
        "exec_command",
        "plugin:web",
        "plugin:workspace-fs",
        "provider:mcp.*",
        "mcp__github__*",
        "*",
    ];
    let tools = [
        ToolIdentity {
            name: "read_file",
            owner: "workspace-fs",
            provider: None,
        },
        ToolIdentity {
            name: "search_text",
            owner: "workspace-fs",
            provider: None,
        },
        ToolIdentity {
            name: "web_search",
            owner: "web",
            provider: None,
        },
        ToolIdentity {
            name: "exec_command",
            owner: "execution",
            provider: None,
        },
        ToolIdentity {
            name: "mcp__github__list",
            owner: "mcp",
            provider: Some("mcp.github"),
        },
    ];
    let modes = [
        None,
        Some(AgentPermissionMode::Manual),
        Some(AgentPermissionMode::AutoReview),
        Some(AgentPermissionMode::FullAccess),
    ];
    let mut seed: u64 = 0x5eed_0fa1_1c4a;
    let mut next = |bound: usize| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((seed >> 33) as usize) % bound
    };
    // Both outcomes occur, so the property is not met vacuously.
    let (mut allowed, mut refused) = (0, 0);
    for _ in 0..500 {
        // Room for `explorer` above the first.
        let length = 1 + next(MAX_ROLE_CHAIN - 1);
        let mut definitions = Vec::new();
        for index in 0..length {
            let mut definition = role(&format!("r{index}"));
            if index > 0 {
                definition.extends = Some(format!("r{}", index - 1));
            } else if next(2) == 0 {
                definition.extends = Some(EXPLORER_ROLE.to_string());
            }
            if next(2) == 0 {
                let count = next(4);
                definition.tools_allow = Some(
                    (0..count)
                        .map(|_| POOL[next(POOL.len())].to_string())
                        .collect(),
                );
            }
            definition.tools_deny = (0..next(3))
                .map(|_| POOL[next(POOL.len())].to_string())
                .collect();
            definition.permission_mode_cap = modes[next(modes.len())];
            definitions.push(definition);
        }
        let own_denies: Vec<Vec<ToolPattern>> = definitions
            .iter()
            .map(|definition| {
                definition
                    .tools_deny
                    .iter()
                    .map(|text| ToolPattern::parse(text).unwrap())
                    .collect()
            })
            .collect();
        let catalog = resolve(user(definitions));
        for index in 1..length {
            let child = usable(&catalog, &format!("r{index}"));
            let parent = usable(&catalog, &format!("r{}", index - 1));
            for tool in &tools {
                let denied_above = own_denies[..=index]
                    .iter()
                    .flatten()
                    .any(|pattern| pattern.matches(tool));
                if child.scope.tools.allows(tool) {
                    allowed += 1;
                    assert!(parent.scope.tools.allows(tool), "{tool:?}: {child:#?}");
                    assert!(!denied_above, "{tool:?}: {child:#?}");
                } else if parent.scope.tools.allows(tool) {
                    refused += 1;
                }
            }
            if let Some(cap) = parent.scope.permission_mode_cap {
                let child_cap = child.scope.permission_mode_cap.expect("a cap is kept");
                assert_eq!(
                    hanafubuki_agent_kernel::stricter_permission_mode(cap, child_cap),
                    child_cap
                );
            }
        }
    }
    assert!(allowed > 100 && refused > 100, "{allowed} {refused}");
}

#[test]
fn a_cycle_a_missing_or_unusable_parent_and_a_deep_chain_are_unusable() {
    let mut broken = role("broken");
    broken.unusable = Some("the file is unusable".into());
    let mut deep = vec![role("d0")];
    deep.extend(
        (1..=MAX_ROLE_CHAIN)
            .map(|index| extending(&format!("d{index}"), &format!("d{}", index - 1))),
    );
    let mut definitions = vec![
        extending("a", "b"),
        extending("b", "a"),
        extending("selfish", "selfish"),
        extending("orphan", "nobody"),
        extending("grandorphan", "orphan"),
        broken,
        extending("heir", "broken"),
    ];
    definitions.extend(deep);
    let catalog = resolve(user(definitions));

    assert!(unusable(&catalog, "a").contains("a → b → a is a cycle"));
    assert!(unusable(&catalog, "selfish").contains("extends itself"));
    assert!(
        unusable(&catalog, "orphan").contains("`orphan` extends `nobody`, which no source defines")
    );
    assert!(unusable(&catalog, "grandorphan").contains("`orphan` extends `nobody`"));
    assert_eq!(unusable(&catalog, "broken"), "the file is unusable");
    assert!(unusable(&catalog, "heir").contains("`broken`, which it extends, is not usable"));
    let last = MAX_ROLE_CHAIN - 1;
    usable(&catalog, &format!("d{last}"));
    assert!(unusable(&catalog, &format!("d{MAX_ROLE_CHAIN}")).contains("more than 8 roles"));
    let issue = catalog
        .issues()
        .iter()
        .find(|issue| issue.role.as_deref() == Some("orphan"))
        .unwrap();
    assert!(issue.message.contains("is not usable"));
}

#[test]
fn a_role_needs_a_description_of_its_own_or_its_chains() {
    let mut root = role("root");
    root.description = None;
    let mut child = extending("child", "root");
    child.description = None;
    let mut described = extending("described", "root");
    described.description = Some("Mine.".into());
    let catalog = resolve(user(vec![root, child, described]));
    assert!(unusable(&catalog, "root").contains("no description"));
    assert!(unusable(&catalog, "child").contains("no description"));
    assert_eq!(usable(&catalog, "described").description, "Mine.");
}

#[test]
fn within_a_source_the_first_keeps_a_name() {
    let mut first = role("twin");
    first.model = Some("first".into());
    let mut second = role("twin");
    second.model = Some("second".into());
    second.origin = RoleOrigin::File {
        path: PathBuf::from("/roles/z/twin.toml"),
    };
    let catalog = resolve(user(vec![first, second]));
    assert_eq!(usable(&catalog, "twin").model.as_deref(), Some("first"));
    let issue = catalog
        .issues()
        .iter()
        .find(|issue| issue.message.contains("defines it again"))
        .unwrap();
    assert!(issue.message.starts_with("user file /roles/z/twin.toml"));
    assert!(issue
        .message
        .contains("user file /roles/twin.toml keeps the name"));
}

#[test]
fn a_higher_source_replaces_a_role_whole() {
    let mut user_role = role("mine");
    user_role.model = Some("user".into());
    user_role.tools_deny = texts(&["exec_command"]);
    let mut profile_role = role("mine");
    profile_role.model = Some("profile".into());
    let mut project_role = role("mine");
    project_role.instructions = Some("Project.".into());
    // The user's explorer replaces the built-in one, narrowing nothing.
    let user_explorer = role(EXPLORER_ROLE);
    let mut project_broken = role("shadowed");
    project_broken.unusable = Some("the file is unusable".into());
    let sources = vec![
        source(
            RoleLayer::User,
            vec![user_role, user_explorer, role("shadowed")],
        ),
        source(
            RoleLayer::Profile {
                name: "work".into(),
            },
            vec![profile_role],
        ),
        source(RoleLayer::Project, vec![project_role, project_broken]),
    ];
    let catalog = resolve(sources);

    let mine = usable(&catalog, "mine");
    assert_eq!(mine.model, None);
    assert!(mine.scope.tools.deny.is_empty());
    assert_eq!(mine.instructions.as_deref(), Some("Project."));
    assert!(matches!(
        &mine.source,
        AgentRoleSource::Configured {
            layer: RoleLayer::Project,
            ..
        }
    ));
    let explorer = usable(&catalog, EXPLORER_ROLE);
    assert!(explorer.scope.tools.allow.is_empty());
    // An unusable role still takes the name: it never falls back to the
    // one below.
    assert_eq!(unusable(&catalog, "shadowed"), "the file is unusable");
}

#[test]
fn the_catalog_loads_from_the_configuration() {
    let root = tempfile::tempdir().unwrap();
    let config_dir = root.path().to_path_buf();
    std::fs::create_dir_all(config_dir.join("agents")).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        "[agents.inline]\ndescription = \"Inline.\"\nextends = \"explorer\"\n",
    )
    .unwrap();
    std::fs::write(
        config_dir.join("agents").join("filed.toml"),
        "description = \"Filed.\"\ntools.deny = [\"fetch_web_page\"]\n",
    )
    .unwrap();
    let catalog = AgentRoleCatalog::load(&AgentConfig::new(&config_dir));
    let names: Vec<_> = catalog.roles().map(|role| role.name.as_str()).collect();
    assert_eq!(names, [DEFAULT_ROLE, EXPLORER_ROLE, "filed", "inline"]);
    let inline = usable(&catalog, "inline");
    assert!(inline.scope.tools.allows(&tool("read_file")));
    assert!(!inline.scope.tools.allows(&tool("apply_patch")));
    assert!(inline
        .source
        .to_string()
        .starts_with("user [agents.inline] in "));
    assert!(catalog.issues().is_empty(), "{:?}", catalog.issues());

    let roles = AgentRoles::load(&AgentConfig::new(&config_dir));
    std::fs::remove_file(config_dir.join("agents").join("filed.toml")).unwrap();
    assert!(roles.current().role("filed").is_some());
    roles.rebuild(&AgentConfig::new(&config_dir));
    assert!(roles.current().role("filed").is_none());
}
