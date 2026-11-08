use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

use parking_lot::Mutex;

use super::*;

/// More than one full catalog, so a session alternating between two
/// catalogs keeps both.
const MAX_CACHED_TOOL_SPECS: usize = 2 * MAX_AGENT_TOOL_CATALOG_ENTRIES;

/// The planned tools whose schemas and provider specs passed
/// `validate_planned_tool`, so a tool is validated once per catalog it comes
/// from rather than on every sampling step.
///
/// A tool is keyed by `(provider_id, tool_id, generation)`, the identity a
/// source keeps stable only while the tool does not change (a kernel or
/// registered tool's generation is a digest of its specs), together with the
/// form its provider spec was projected to. A new catalog (an MCP server
/// relisted, a plugin restarted) brings new generations and is validated
/// again. Shared by the service across sessions, like the search index cache.
#[derive(Clone, Default)]
pub(crate) struct AgentToolSpecCache {
    inner: Arc<Mutex<SpecCacheState>>,
}

#[derive(Default)]
struct SpecCacheState {
    validated: BTreeSet<SpecKey>,
    insertion_order: VecDeque<SpecKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SpecKey {
    selection: AgentToolSelection,
    form: u8,
    namespace_description: Option<String>,
}

impl SpecKey {
    fn of(tool: &AgentPlannedTool) -> Self {
        let form = match &tool.provider_spec {
            ToolSpec::Function(_) => 0,
            ToolSpec::Namespace(_) => 1,
            ToolSpec::ToolSearch(_) => 2,
            ToolSpec::HostedWebSearch(_) => 3,
            ToolSpec::Freeform(_) => 4,
        };
        Self {
            selection: AgentToolSelection::from_planned_tool(tool),
            form,
            namespace_description: tool.namespace_description.clone(),
        }
    }
}

impl AgentToolSpecCache {
    /// Runs `validate` unless `tool`'s specs already passed it, and records
    /// a pass.
    pub(super) fn validate_once(
        &self,
        tool: &AgentPlannedTool,
        validate: impl FnOnce() -> Result<(), AgentToolCatalogError>,
    ) -> Result<(), AgentToolCatalogError> {
        let key = SpecKey::of(tool);
        if self.inner.lock().validated.contains(&key) {
            return Ok(());
        }
        validate()?;
        let mut cached = self.inner.lock();
        if cached.validated.insert(key.clone()) {
            while cached.validated.len() > MAX_CACHED_TOOL_SPECS {
                let Some(oldest) = cached.insertion_order.pop_front() else {
                    break;
                };
                cached.validated.remove(&oldest);
            }
            cached.insertion_order.push_back(key);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.inner.lock().validated.len()
    }
}
