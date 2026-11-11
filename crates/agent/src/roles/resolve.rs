//! From the sources' definitions to the catalog: the highest source keeps a
//! name, the first definition by path within a source; each role's chain is
//! followed through `extends` and merged root first, every rule only
//! narrowing.

use std::collections::{BTreeMap, BTreeSet};

use hanafubuki_agent_config::{InstructionsMode, RoleDefinition, RoleSource};
use hanafubuki_agent_foundation::agent_settings::ModelReasoningEffort;
use hanafubuki_agent_kernel::{
    AgentCapabilityScope, AgentToolScope, ToolPattern, MAX_SCOPE_TOOL_PATTERNS,
};

use super::builtin::{built_in_roles, DEFAULT_ROLE};
use super::{
    AgentResolvedRole, AgentRoleCatalog, AgentRoleIssue, AgentRoleLink, AgentRoleSource,
    AgentUnusableRole, MAX_ROLE_CHAIN,
};

/// One role as its source defines it, its patterns read.
pub(super) struct Candidate {
    pub(super) name: String,
    pub(super) source: AgentRoleSource,
    pub(super) description: Option<String>,
    pub(super) extends: Option<String>,
    pub(super) instructions: Option<String>,
    pub(super) instructions_mode: InstructionsMode,
    pub(super) model: Option<String>,
    pub(super) reasoning_effort: Option<ModelReasoningEffort>,
    pub(super) spawnable: Option<bool>,
    /// The role's own narrowing: at most one allow list.
    pub(super) scope: AgentCapabilityScope,
    pub(super) unusable: Option<String>,
}

pub(super) fn resolve(sources: Vec<RoleSource>) -> AgentRoleCatalog {
    let mut issues = Vec::new();
    let mut candidates: BTreeMap<String, Candidate> = built_in_roles()
        .into_iter()
        .map(|candidate| (candidate.name.clone(), candidate))
        .collect();
    for source in sources {
        issues.extend(source.issues.into_iter().map(|message| AgentRoleIssue {
            role: None,
            message: format!("{} roles: {message}", source.layer),
        }));
        let mut kept: BTreeMap<String, AgentRoleSource> = BTreeMap::new();
        for definition in source.definitions {
            let place = AgentRoleSource::Configured {
                layer: source.layer.clone(),
                origin: definition.origin.clone(),
            };
            // `default` always narrows nothing: a session switched back to
            // it has the empty scope.
            if definition.name == DEFAULT_ROLE {
                issues.push(AgentRoleIssue {
                    role: Some(definition.name.clone()),
                    message: format!(
                        "{place} is not usable: the name is reserved for the built-in role, \
                         which narrows nothing; give this role another name"
                    ),
                });
                continue;
            }
            if let Some(first) = kept.get(&definition.name) {
                issues.push(AgentRoleIssue {
                    role: Some(definition.name.clone()),
                    message: format!(
                        "{place} defines it again; {first} keeps the name, and this one is ignored"
                    ),
                });
                continue;
            }
            kept.insert(definition.name.clone(), place.clone());
            let candidate = candidate(definition, place, &mut issues);
            candidates.insert(candidate.name.clone(), candidate);
        }
    }
    let mut roles = BTreeMap::new();
    let mut unusable = BTreeMap::new();
    for (name, candidate) in &candidates {
        match resolve_role(candidate, &candidates) {
            Ok(role) => {
                roles.insert(name.clone(), role);
            }
            Err(reason) => {
                issues.push(AgentRoleIssue {
                    role: Some(name.clone()),
                    message: format!("{} is not usable: {reason}", candidate.source),
                });
                unusable.insert(
                    name.clone(),
                    AgentUnusableRole {
                        name: name.clone(),
                        source: candidate.source.clone(),
                        reason,
                    },
                );
            }
        }
    }
    AgentRoleCatalog {
        roles,
        unusable,
        issues,
    }
}

/// `definition` with its patterns read: one that is not a pattern is left
/// out with an issue, which never widens the role (an allow list loses an
/// entry; a deny entry that is no pattern denies nothing).
fn candidate(
    definition: RoleDefinition,
    source: AgentRoleSource,
    issues: &mut Vec<AgentRoleIssue>,
) -> Candidate {
    let RoleDefinition {
        name,
        origin: _,
        description,
        extends,
        instructions,
        instructions_mode,
        model,
        reasoning_effort,
        permission_mode_cap,
        spawnable,
        tools_allow,
        tools_deny,
        plugins_off,
        unusable,
        issues: read_issues,
    } = definition;
    let mut report = |message: String| {
        issues.push(AgentRoleIssue {
            role: Some(name.clone()),
            message: format!("{source}: {message}"),
        })
    };
    for issue in read_issues {
        report(issue);
    }
    let mut patterns = |key: &str, texts: Vec<String>| -> BTreeSet<ToolPattern> {
        texts
            .into_iter()
            .filter_map(|text| match ToolPattern::parse(&text) {
                Ok(pattern) => Some(pattern),
                Err(error) => {
                    report(format!("`{key}` entry `{text}` is left out: {error}"));
                    None
                }
            })
            .collect()
    };
    let allow = tools_allow.map(|texts| patterns("tools.allow", texts));
    let deny = patterns("tools.deny", tools_deny);
    let plugins_off = plugins_off
        .into_iter()
        .filter(|plugin| !plugin.trim().is_empty())
        .collect();
    Candidate {
        scope: AgentCapabilityScope {
            role: Some(name.clone()),
            tools: AgentToolScope {
                allow: allow.into_iter().collect(),
                deny,
            },
            plugins_off,
            permission_mode_cap,
        },
        name,
        source,
        description,
        extends,
        instructions,
        instructions_mode,
        model,
        reasoning_effort,
        spawnable,
        unusable,
    }
}

/// The role `candidate` with its chain merged; the error says why it is
/// not usable.
fn resolve_role(
    candidate: &Candidate,
    candidates: &BTreeMap<String, Candidate>,
) -> Result<AgentResolvedRole, String> {
    let chain = chain(candidate, candidates)?;
    let mut merged = Merged::default();
    for link in chain.iter().rev() {
        merged.extend(link);
    }
    let description = merged
        .description
        .ok_or_else(|| "it has no description, and neither has a role it extends".to_string())?;
    // A session's scope holds no more: past it, the session would allow no
    // plugin tool at all.
    let patterns = merged
        .scope
        .tools
        .allow
        .iter()
        .map(BTreeSet::len)
        .sum::<usize>()
        + merged.scope.tools.deny.len();
    if patterns > MAX_SCOPE_TOOL_PATTERNS {
        return Err(format!(
            "its chain's tool patterns number {patterns}, more than {MAX_SCOPE_TOOL_PATTERNS}"
        ));
    }
    let mut provenance = merged.provenance;
    for roles in provenance.values_mut() {
        roles.reverse();
    }
    Ok(AgentResolvedRole {
        name: candidate.name.clone(),
        description,
        source: candidate.source.clone(),
        chain: chain
            .iter()
            .map(|link| AgentRoleLink {
                name: link.name.clone(),
                source: link.source.clone(),
            })
            .collect(),
        instructions: merged.instructions,
        model: merged.model,
        reasoning_effort: merged.reasoning_effort,
        spawnable: merged.spawnable.unwrap_or(true),
        scope: AgentCapabilityScope {
            role: Some(candidate.name.clone()),
            ..merged.scope
        },
        provenance,
    })
}

/// `candidate`, then each role it extends: never a silent fallback, so a
/// parent that is missing, unusable, in a cycle or too deep makes the role
/// unusable.
fn chain<'a>(
    candidate: &'a Candidate,
    candidates: &'a BTreeMap<String, Candidate>,
) -> Result<Vec<&'a Candidate>, String> {
    let mut chain: Vec<&Candidate> = Vec::new();
    let mut current = candidate;
    loop {
        if let Some(why) = &current.unusable {
            return Err(match chain.is_empty() {
                true => why.clone(),
                false => format!("`{}`, which it extends, is not usable: {why}", current.name),
            });
        }
        chain.push(current);
        let Some(parent) = &current.extends else {
            return Ok(chain);
        };
        if parent == &current.name {
            return Err(format!(
                "`{parent}` extends itself; to build on the `{parent}` of a source below, \
                 give this role another name"
            ));
        }
        if chain.iter().any(|link| &link.name == parent) {
            let names: Vec<&str> = chain.iter().map(|link| link.name.as_str()).collect();
            return Err(format!(
                "its chain {} → {parent} is a cycle",
                names.join(" → ")
            ));
        }
        if chain.len() == MAX_ROLE_CHAIN {
            return Err(format!("its chain holds more than {MAX_ROLE_CHAIN} roles"));
        }
        current = candidates.get(parent).ok_or_else(|| {
            format!(
                "`{}` extends `{parent}`, which no source defines",
                current.name
            )
        })?;
    }
}

/// A chain merged so far, root first.
#[derive(Default)]
struct Merged {
    description: Option<String>,
    instructions: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<ModelReasoningEffort>,
    spawnable: Option<bool>,
    scope: AgentCapabilityScope,
    /// Root first until the end, then reversed.
    provenance: BTreeMap<&'static str, Vec<String>>,
}

impl Merged {
    /// Merges `link`, the next role toward the leaf: a field it sets
    /// replaces its parent's, instructions append or replace, and its
    /// narrowing joins its parents'.
    fn extend(&mut self, link: &Candidate) {
        let name = &link.name;
        let mut decide = |key: &'static str, set: bool| {
            if set {
                self.provenance.insert(key, vec![name.clone()]);
            }
        };
        decide("description", link.description.is_some());
        decide("model", link.model.is_some());
        decide("reasoningEffort", link.reasoning_effort.is_some());
        decide("spawnable", link.spawnable.is_some());
        replace_if_set(&mut self.description, &link.description);
        replace_if_set(&mut self.model, &link.model);
        replace_if_set(&mut self.reasoning_effort, &link.reasoning_effort);
        replace_if_set(&mut self.spawnable, &link.spawnable);
        self.merge_instructions(link);
        self.merge_scope(link);
    }

    fn merge_instructions(&mut self, link: &Candidate) {
        let contributors = self.provenance.entry("instructions").or_default();
        match (link.instructions_mode, &link.instructions) {
            (InstructionsMode::Replace, own) => {
                contributors.clear();
                contributors.extend(own.as_ref().map(|_| link.name.clone()));
                self.instructions = own.clone();
            }
            (InstructionsMode::Append, None) => {}
            (InstructionsMode::Append, Some(own)) => {
                contributors.push(link.name.clone());
                self.instructions = Some(match self.instructions.take() {
                    Some(parent) => format!("{parent}\n\n{own}"),
                    None => own.clone(),
                });
            }
        }
        if contributors.is_empty() {
            self.provenance.remove("instructions");
        }
    }

    fn merge_scope(&mut self, link: &Candidate) {
        let own = &link.scope;
        let cap_before = self.scope.permission_mode_cap;
        self.scope = self.scope.narrow(own);
        let mut contribute = |key: &'static str, contributes: bool| {
            if contributes {
                self.provenance
                    .entry(key)
                    .or_default()
                    .push(link.name.clone());
            }
        };
        contribute("tools.allow", !own.tools.allow.is_empty());
        contribute("tools.deny", !own.tools.deny.is_empty());
        contribute("plugins.off", !own.plugins_off.is_empty());
        if own.permission_mode_cap.is_some() && self.scope.permission_mode_cap != cap_before {
            self.provenance
                .insert("permissionModeCap", vec![link.name.clone()]);
        }
    }
}

fn replace_if_set<T: Clone>(field: &mut Option<T>, own: &Option<T>) {
    if own.is_some() {
        field.clone_from(own);
    }
}
