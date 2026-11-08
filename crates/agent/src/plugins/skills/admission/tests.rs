//! The turn's catalog keeps only Skills whose MCP dependencies are
//! reachable and lists the others as unavailable, and every kept Skill is
//! selectable with the context it adopts.

use hanafubuki_agent_contracts::SelectableSkills;
use hanafubuki_agent_skills::{
    SkillId, SkillMcpTransport, SkillScope, SkillToolDependency, SkillTurnEntry,
};

use super::*;
use crate::plugins::tests::fixtures::disconnected_mcp_application;

fn entry(folder: &str, implicit: bool, dependencies: Vec<SkillToolDependency>) -> SkillTurnEntry {
    SkillTurnEntry {
        id: SkillId::new(SkillScope::User, folder).unwrap(),
        name: folder.to_string(),
        description: format!("{folder} description"),
        allow_implicit_invocation: implicit,
        dependencies,
        directory: format!("/skills/{folder}"),
        document: format!("---\nname: {folder}\ndescription: work\n---\nDo the {folder} work."),
        resources: Vec::new(),
    }
}

#[test]
fn a_skill_whose_mcp_dependency_is_unreachable_is_listed_unavailable_with_its_server() {
    let settings = std::env::temp_dir().join(format!(
        "hanafubuki-skills-admission-{}-mcp-settings.json",
        std::process::id()
    ));
    let mut catalog = SkillTurnCatalog {
        catalog_revision: 3,
        entries: vec![
            entry("plain", true, Vec::new()),
            entry(
                "remote",
                true,
                vec![SkillToolDependency {
                    server_id: "absent".to_string(),
                    description: "An MCP server nobody configured".to_string(),
                    transport: Some(SkillMcpTransport::StreamableHttp),
                    url: Some("https://example.test/mcp".to_string()),
                }],
            ),
        ],
        unavailable: Vec::new(),
    };

    retain_available_skill_tools(&mut catalog, &disconnected_mcp_application(settings));

    let kept = catalog
        .entries
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(kept, ["user:plain"]);
    assert_eq!(
        catalog.unavailable,
        [SkillTurnUnavailable {
            id: SkillId::new(SkillScope::User, "remote").unwrap(),
            name: "remote".to_string(),
            reason: "MCP server `absent` is missing: required MCP server is not configured"
                .to_string(),
        }]
    );
}

#[test]
fn every_catalog_entry_is_selectable_with_the_context_it_adopts() {
    let catalog = SkillTurnCatalog {
        catalog_revision: 5,
        entries: vec![
            entry("listed", true, Vec::new()),
            entry("explicit-only", false, Vec::new()),
        ],
        unavailable: Vec::new(),
    };

    let turn = turn_extensions(catalog.clone(), vec!["skills_list".to_string()]);

    assert_eq!(turn.get::<SkillTurnCatalog>(), Some(&catalog));
    let selectable = turn.get::<SelectableSkills>().unwrap();
    let ids = selectable
        .skills
        .iter()
        .map(|skill| skill.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["user:listed", "user:explicit-only"]);
    assert_eq!(selectable.mention_exclusions, ["skills_list"]);
    assert_eq!(
        selectable.skills[1].context,
        "<skill>\n<name>explicit-only</name>\n<skill_id>user:explicit-only</skill_id>\n<directory>/skills/explicit-only</directory>\n---\nname: explicit-only\ndescription: work\n---\nDo the explicit-only work.\n</skill>"
    );
}

#[test]
fn tool_names_are_mention_exclusions_deduplicated_ignoring_case() {
    let names = ["view_image", "Skills_List", "skills_list"]
        .map(str::to_string)
        .to_vec();

    assert_eq!(
        mention_exclusions(names, &[]),
        ["Skills_List", "view_image"]
    );
}
