//! The Skill candidates captured for one user turn.

use crate::model::{SkillAvailabilityState, SkillError, SkillId, SkillTurnCatalog, SkillTurnEntry};

use super::{SkillService, MAX_TURN_DOCUMENT_BYTES};

const TRUNCATION_NOTICE: &str = "\n\n<!-- SKILL.md truncated to fit the model context -->\n\n";

impl SkillService {
    /// Every enabled, locally available Skill. Callers apply external
    /// dependency availability before admitting the turn.
    pub fn turn_catalog(&self) -> Result<SkillTurnCatalog, SkillError> {
        let snapshot = self.list()?;
        let ids = snapshot
            .entries
            .iter()
            .filter(|entry| {
                entry.enabled && entry.availability.state == SkillAvailabilityState::Ready
            })
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        self.build_turn_catalog(snapshot.revision, &ids)
    }

    fn build_turn_catalog(
        &self,
        revision: u64,
        ids: &[SkillId],
    ) -> Result<SkillTurnCatalog, SkillError> {
        let mut entries = self.with_catalog(|catalog| {
            ids.iter()
                .filter_map(|id| catalog.skills.get(id))
                .map(|skill| SkillTurnEntry {
                    id: skill.entry.id.clone(),
                    name: skill.entry.display_name().to_string(),
                    description: skill.entry.summary().to_string(),
                    allow_implicit_invocation: skill.entry.allow_implicit_invocation,
                    dependencies: skill.entry.dependencies.clone(),
                    directory: skill.entry.directory.clone(),
                    document: bounded_document(&skill.document),
                    resources: skill.entry.resources.clone(),
                })
                .collect::<Vec<_>>()
        });
        entries.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(SkillTurnCatalog {
            catalog_revision: revision,
            entries,
            unavailable: Vec::new(),
        })
    }
}

/// Keep the head and the tail of an over-long document: the head carries the
/// frontmatter and the routing instructions, and the tail usually carries the
/// notes that qualify them.
fn bounded_document(document: &str) -> String {
    if document.len() <= MAX_TURN_DOCUMENT_BYTES {
        return document.to_string();
    }
    let available = MAX_TURN_DOCUMENT_BYTES.saturating_sub(TRUNCATION_NOTICE.len());
    let head_budget = available / 2;
    let tail_budget = available.saturating_sub(head_budget);
    let head = &document[..floor_boundary(document, head_budget)];
    let tail = &document[ceil_boundary(document, document.len().saturating_sub(tail_budget))..];
    format!("{head}{TRUNCATION_NOTICE}{tail}")
}

fn floor_boundary(value: &str, index: usize) -> usize {
    let mut index = index.min(value.len());
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_boundary(value: &str, index: usize) -> usize {
    let mut index = index.min(value.len());
    while index < value.len() && !value.is_char_boundary(index) {
        index += 1;
    }
    index
}
