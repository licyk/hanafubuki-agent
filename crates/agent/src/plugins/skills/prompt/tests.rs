use super::*;
use hanafubuki_agent_skills::{
    SkillId, SkillResourceDescriptor, SkillResourceKind, SkillScope, SkillTurnEntry,
    SkillTurnUnavailable,
};

fn snapshot(count: usize) -> SkillTurnCatalog {
    SkillTurnCatalog {
        catalog_revision: 7,
        entries: (0..count)
            .map(|index| SkillTurnEntry {
                id: SkillId::new(SkillScope::User, &format!("skill-{index}")).unwrap(),
                name: format!("Skill {index}"),
                description: "bounded description".to_string(),
                allow_implicit_invocation: true,
                dependencies: Vec::new(),
                directory: format!("/skills/skill-{index}"),
                document: "---\nname: skill\ndescription: work\n---\ndo the work".to_string(),
                resources: vec![SkillResourceDescriptor {
                    path: "references/guide.md".to_string(),
                    kind: SkillResourceKind::Reference,
                    size: 10,
                }],
            })
            .collect(),
        unavailable: Vec::new(),
    }
}

#[test]
fn explicit_only_skills_are_hidden_from_the_model_catalog() {
    let mut catalog = snapshot(2);
    catalog.entries[0].allow_implicit_invocation = false;
    catalog.entries[0].description = "hidden explicit-only description".to_string();
    let rendered = render_skill_catalog(&catalog, None);
    assert_eq!(rendered.included_entries, 1);
    assert!(!rendered
        .content
        .contains("hidden explicit-only description"));
    assert!(rendered.content.contains("Skill 1"));
}

#[test]
fn render_is_stable_and_excludes_instruction_bodies() {
    let snapshot = snapshot(2);
    let first = render_skill_catalog(&snapshot, Some(128_000));
    let second = render_skill_catalog(&snapshot, Some(128_000));
    assert_eq!(first, second);
    assert_eq!(first.included_entries, 2);
    assert!(!first.content.contains("do the work"));
    assert!(first.content.contains("<skills_instructions>"));
    assert!(first.content.contains("Trigger rules"));
}

#[test]
fn entries_show_their_directory_and_unavailable_skills_their_reason() {
    let mut catalog = snapshot(1);
    catalog.unavailable.push(SkillTurnUnavailable {
        id: SkillId::new(SkillScope::User, "remote").unwrap(),
        name: "Remote".to_string(),
        reason: "MCP server `github` is missing: required MCP server is not configured".to_string(),
    });
    let rendered = render_skill_catalog(&catalog, None);
    assert!(rendered.content.contains(
        "- `Skill 0` (skill: `user:skill-0`; directory: `/skills/skill-0`): bounded description\n"
    ));
    assert!(rendered.content.contains(
        "### Unavailable skills\n- `Remote` (skill: `user:remote`): MCP server `github` is missing: required MCP server is not configured\n"
    ));
    assert_eq!(rendered.included_entries, 1);
}

#[test]
fn render_uses_a_stable_omission_marker() {
    let rendered = render_skill_catalog(&snapshot(8), Some(1_000));
    assert!(rendered.omitted_entries > 0);
    assert!(rendered.content.contains("Catalog omitted"));
    assert!(rendered.content.len() <= MAXIMUM_CATALOG_BYTES);
}

#[test]
fn renderer_shortens_descriptions_before_omitting_catalog_identities() {
    let mut snapshot = snapshot(6);
    for entry in &mut snapshot.entries {
        entry.description = "long-description-".repeat(80);
    }
    let rendered = render_skill_catalog(&snapshot, None);
    assert_eq!(rendered.included_entries, 6);
    assert_eq!(rendered.omitted_entries, 0);
    for index in 0..6 {
        assert!(rendered.content.contains(&format!("Skill {index}")));
    }
    assert!(rendered.content.len() <= MAXIMUM_CATALOG_BYTES);
    assert!(!rendered.content.contains(&"long-description-".repeat(80)));
}

#[test]
fn the_listing_needs_an_implicitly_invocable_skill_in_the_turns_catalog() {
    use hanafubuki_agent_contracts::TurnExtensions;
    use std::sync::Arc;

    let render = |turn: &TurnExtensions| {
        SkillCatalogPrompt.contribute(&PromptRequest {
            turn,
            prepared: None,
            context_window_tokens: Some(128_000),
        })
    };
    assert!(render(&TurnExtensions::default()).is_empty());
    let mut explicit_only = snapshot(1);
    explicit_only.entries[0].allow_implicit_invocation = false;
    assert!(render(&TurnExtensions::default().with(Arc::new(explicit_only.clone()))).is_empty());
    // An unavailable Skill alone is still listed, so the model can say why.
    explicit_only.unavailable.push(SkillTurnUnavailable {
        id: SkillId::new(SkillScope::User, "remote").unwrap(),
        name: "remote".to_string(),
        reason: "MCP server `github` is missing".to_string(),
    });
    assert_eq!(
        render(&TurnExtensions::default().with(Arc::new(explicit_only))).len(),
        1
    );

    let catalog = snapshot(2);
    assert_eq!(
        render(&TurnExtensions::default().with(Arc::new(catalog.clone()))),
        vec![PromptFragment {
            kind: SKILL_CATALOG,
            version: None,
            content: render_skill_catalog(&catalog, Some(128_000)).content,
        }]
    );
}
