use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use hanafubuki_agent_contracts::{
    ContextRenderer, ContextRole, ContextSection, ContextSectionKind, ContextSlot,
    WORLD_STATE_PREAMBLE,
};

use super::ConversationItem;

/// Sections of kinds plugins name beyond the known kinds
/// (`ContextSectionKind::KNOWN`), a host's context among them. The built-in
/// kinds have their own slots, so they never use up what plugins may add.
pub const MAXIMUM_AGENT_PLUGIN_WORLD_STATE_SECTIONS: usize = 7;
/// Every section a world state may hold: one per known kind (5) and the
/// plugin kinds. The byte bounds below bound its size.
pub const MAXIMUM_AGENT_WORLD_STATE_SECTIONS: usize = 5 + MAXIMUM_AGENT_PLUGIN_WORLD_STATE_SECTIONS;
pub const MAXIMUM_AGENT_WORLD_STATE_SECTION_BYTES: usize = 32 * 1024;
pub const MAXIMUM_AGENT_WORLD_STATE_TOTAL_BYTES: usize = 64 * 1024;

/// The renderer of the kind the kernel writes itself: the tool catalog's
/// omissions for each sampling step. A host context that steering carries is
/// rendered by the renderer its host's plugin registers.
pub(crate) fn kernel_context_renderers() -> [ContextRenderer; 1] {
    [ContextRenderer {
        kind: ContextSectionKind::TOOL_CATALOG_OMISSIONS,
        slot: ContextSlot::WorldState,
        role: ContextRole::Developer,
        preamble: WORLD_STATE_PREAMBLE,
        epilogue: "",
        versioned: true,
    }]
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorldStateSection {
    pub kind: ContextSectionKind,
    pub version: [u8; 32],
    pub content: String,
}

impl AgentWorldStateSection {
    /// The version is a digest of the kind name and the content, so a section
    /// whose text did not change is not written to history again.
    pub fn new(kind: ContextSectionKind, content: String) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(kind.as_str().as_bytes());
        hasher.update([0]);
        hasher.update(content.as_bytes());
        Self {
            kind,
            version: hasher.finalize().into(),
            content,
        }
    }
}

impl From<ContextSection> for AgentWorldStateSection {
    fn from(section: ContextSection) -> Self {
        Self::new(section.kind, section.content)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentWorldStateSnapshot {
    pub sections: Vec<AgentWorldStateSection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentWorldStateUpdate {
    Replace {
        kind: ContextSectionKind,
        version: [u8; 32],
        content: String,
    },
    Remove {
        kind: ContextSectionKind,
        previous_version: [u8; 32],
    },
}

pub(crate) fn effective_world_state<T: std::borrow::Borrow<crate::AgentHistoryItem>>(
    history: &[T],
) -> BTreeMap<ContextSectionKind, AgentWorldStateSection> {
    let mut state = BTreeMap::new();
    for item in history {
        let ConversationItem::WorldStateUpdate { updates } = &item.borrow().content else {
            continue;
        };
        for update in updates {
            match update {
                AgentWorldStateUpdate::Replace {
                    kind,
                    version,
                    content,
                } => {
                    state.insert(
                        kind.clone(),
                        AgentWorldStateSection {
                            kind: kind.clone(),
                            version: *version,
                            content: content.clone(),
                        },
                    );
                }
                AgentWorldStateUpdate::Remove { kind, .. } => {
                    state.remove(kind);
                }
            }
        }
    }
    state
}

pub(crate) fn world_state_diff<T: std::borrow::Borrow<crate::AgentHistoryItem>>(
    history: &[T],
    snapshot: AgentWorldStateSnapshot,
) -> Vec<AgentWorldStateUpdate> {
    let previous = effective_world_state(history);
    let next = snapshot
        .sections
        .into_iter()
        .map(|section| (section.kind.clone(), section))
        .collect::<BTreeMap<_, _>>();
    let mut updates = Vec::new();
    for (kind, section) in &next {
        if previous.get(kind).map(|value| value.version) != Some(section.version) {
            updates.push(AgentWorldStateUpdate::Replace {
                kind: kind.clone(),
                version: section.version,
                content: section.content.clone(),
            });
        }
    }
    for (kind, section) in previous {
        if !next.contains_key(&kind) {
            updates.push(AgentWorldStateUpdate::Remove {
                kind,
                previous_version: section.version,
            });
        }
    }
    updates
}

pub(crate) fn full_world_state_checkpoint<T: std::borrow::Borrow<crate::AgentHistoryItem>>(
    history: &[T],
) -> Option<ConversationItem> {
    let updates = effective_world_state(history)
        .into_values()
        .map(|section| AgentWorldStateUpdate::Replace {
            kind: section.kind,
            version: section.version,
            content: section.content,
        })
        .collect::<Vec<_>>();
    (!updates.is_empty()).then_some(ConversationItem::WorldStateUpdate { updates })
}

pub(crate) fn replace_world_state_section(
    state: &mut BTreeMap<ContextSectionKind, AgentWorldStateSection>,
    section: AgentWorldStateSection,
) -> Option<ConversationItem> {
    if state.get(&section.kind).map(|value| value.version) == Some(section.version) {
        return None;
    }
    state.insert(section.kind.clone(), section.clone());
    Some(ConversationItem::WorldStateUpdate {
        updates: vec![AgentWorldStateUpdate::Replace {
            kind: section.kind,
            version: section.version,
            content: section.content,
        }],
    })
}

pub(crate) fn remove_world_state_section(
    state: &mut BTreeMap<ContextSectionKind, AgentWorldStateSection>,
    kind: ContextSectionKind,
) -> Option<ConversationItem> {
    let previous = state.remove(&kind)?;
    Some(ConversationItem::WorldStateUpdate {
        updates: vec![AgentWorldStateUpdate::Remove {
            kind,
            previous_version: previous.version,
        }],
    })
}

#[cfg(test)]
mod tests;
