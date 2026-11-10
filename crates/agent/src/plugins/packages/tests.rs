use hanafubuki_agent_skills::{SkillId, SkillTurnCatalog, SkillTurnEntry};

use super::prompt::{prompt_safe_description, render_available_plugins};
use super::{AvailablePlugins, PluginSummary};

fn summary(id: &str, mcp_servers: &[&str]) -> PluginSummary {
    let name = id.split('@').next().unwrap().to_string();
    PluginSummary {
        id: id.to_string(),
        display_name: format!("{name} plugin"),
        namespace: name,
        description: Some("  Does\n   things.  ".to_string()),
        mcp_servers: mcp_servers
            .iter()
            .map(|server| server.to_string())
            .collect(),
    }
}

fn catalog(ids: &[&str]) -> SkillTurnCatalog {
    SkillTurnCatalog {
        entries: ids
            .iter()
            .map(|id| SkillTurnEntry {
                id: SkillId::from_untrusted(*id).unwrap(),
                name: "skill".to_string(),
                description: String::new(),
                allow_implicit_invocation: true,
                dependencies: Vec::new(),
                directory: String::new(),
                document: String::new(),
                resources: Vec::new(),
            })
            .collect(),
        ..SkillTurnCatalog::default()
    }
}

#[test]
fn only_plugins_with_skills_or_mcp_servers_are_listed() {
    let plugins = AvailablePlugins(vec![
        summary("hooks-only@local", &[]),
        summary("skilled@local", &[]),
        summary("served@local", &["files", "remote"]),
    ]);

    let rendered =
        render_available_plugins(&plugins, Some(&catalog(&["plugin:skilled@local:search"])))
            .unwrap();

    assert!(!rendered.contains("hooks-only"));
    assert!(rendered.contains(
        "- `skilled plugin` (`skilled@local`): Does things. Skills: prefixed with `skilled:`.\n"
    ));
    assert!(rendered.contains(
        "- `served plugin` (`served@local`): Does things. MCP servers: `files`, `remote`.\n"
    ));
    assert!(rendered.starts_with("<plugins_instructions>\n"));
    assert!(rendered.ends_with("</plugins_instructions>"));
}

#[test]
fn no_plugin_with_content_is_no_section() {
    let plugins = AvailablePlugins(vec![summary("hooks-only@local", &[])]);
    assert_eq!(render_available_plugins(&plugins, None), None);
    assert_eq!(
        render_available_plugins(&AvailablePlugins::default(), None),
        None
    );
}

#[test]
fn a_description_is_one_line_of_at_most_1024_characters() {
    assert_eq!(prompt_safe_description(Some(" \n ")), None);
    let long = "é ".repeat(2_000);
    let safe = prompt_safe_description(Some(&long)).unwrap();
    assert_eq!(safe.chars().count(), 1_024);
    assert!(!safe.contains('\n'));
}

#[test]
fn the_section_stays_within_its_bound() {
    let plugins = AvailablePlugins(
        (0..40)
            .map(|index| PluginSummary {
                description: Some("x".repeat(1_024)),
                ..summary(&format!("p{index}@local"), &["server"])
            })
            .collect(),
    );

    let rendered = render_available_plugins(&plugins, None).unwrap();

    assert!(rendered.len() <= 8_000);
    assert!(rendered.contains("more plugin(s) omitted"));
}
