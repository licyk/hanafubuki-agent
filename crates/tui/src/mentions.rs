//! The mention pickers' native capability catalog. Identities stay separate from labels.

use crate::bottom_pane::mentions::{Candidate, MentionType, Selection};
use crate::bottom_pane::skill_popup::MentionItem;
use hanafubuki_agent::{AgentPackageStatus, SkillAvailabilityState, SkillCatalogEntry};

#[derive(Clone, Debug, Default)]
pub(crate) struct Catalog {
    pub(crate) at: Vec<Candidate>,
    pub(crate) dollar: Vec<MentionItem>,
}

impl Catalog {
    pub(crate) fn build(
        skills: Vec<SkillCatalogEntry>,
        plugins: Vec<AgentPackageStatus>,
        servers: Vec<String>,
    ) -> Self {
        let mut catalog = Self::default();
        let plugins: Vec<_> = plugins.into_iter().filter(|p| p.enabled).collect();
        for skill in skills
            .into_iter()
            .filter(|s| s.enabled && s.availability.state == SkillAvailabilityState::Ready)
        {
            let name = skill.name.clone();
            let display_name = skill.display_name().to_owned();
            let description = Some(skill.summary().to_owned());
            let search_terms = vec![
                name.clone(),
                display_name.clone(),
                skill.id.as_str().to_owned(),
            ];
            let insert_text = format!("${name}");
            let path = Some(format!("skill://{}", skill.id.as_str()));
            catalog.dollar.push(MentionItem {
                display_name: display_name.clone(),
                description: description.clone(),
                search_terms: search_terms.clone(),
                insert_text: insert_text.clone(),
                path: path.clone(),
                category_tag: Some("[Skill]".into()),
                sort_rank: 1,
            });
            if !plugins
                .iter()
                .any(|p| p.content.skills.iter().any(|id| id == skill.id.as_str()))
            {
                catalog.at.push(Candidate {
                    display_name,
                    description,
                    search_terms,
                    mention_type: MentionType::Skill,
                    selection: Selection::Tool { insert_text, path },
                });
            }
        }
        for plugin in plugins {
            let name = plugin.id.split('@').next().unwrap_or(&plugin.id);
            let label = plugin_label(name, &plugin.display_name);
            catalog.at.push(Candidate {
                display_name: plugin.display_name.clone(),
                description: plugin.description,
                search_terms: vec![plugin.id.clone(), name.to_owned(), plugin.display_name],
                mention_type: MentionType::Plugin,
                selection: Selection::Tool {
                    insert_text: format!("@{label}"),
                    path: Some(format!("plugin://{}", plugin.id)),
                },
            });
        }
        for name in servers {
            catalog.dollar.push(MentionItem {
                display_name: name.clone(),
                description: Some("MCP server".into()),
                search_terms: vec![name.clone()],
                insert_text: format!("${name}"),
                path: Some(format!("mcp://{name}")),
                category_tag: Some("[MCP]".into()),
                sort_rank: 0,
            });
        }
        catalog
    }
}

fn title_case(name: &str) -> String {
    let mut capital = true;
    name.chars()
        .map(|ch| {
            let value = if capital { ch.to_ascii_uppercase() } else { ch };
            capital = matches!(ch, '-' | '_');
            value
        })
        .collect()
}

fn plugin_label(name: &str, display: &str) -> String {
    let parts = name.split(['-', '_']).collect::<Vec<_>>();
    let words = display
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() == words.len()
        && parts
            .iter()
            .zip(&words)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    {
        let mut words = words.into_iter();
        let mut label = words.next().unwrap_or_default().to_owned();
        for separator in name.chars().filter(|ch| matches!(ch, '-' | '_')) {
            label.push(separator);
            label.push_str(words.next().unwrap_or_default());
        }
        label
    } else {
        title_case(name)
    }
}
