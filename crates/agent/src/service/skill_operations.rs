use super::AgentApplicationService;
use crate::{
    SkillCatalogEntry, SkillCatalogInvalidationPort, SkillCatalogSnapshot, SkillCreateRequest,
    SkillDeleteFileRequest, SkillDuplicateRequest, SkillFileRead, SkillId, SkillInstallRequest,
    SkillRead, SkillResourceRead, SkillSourceInspection, SkillWriteFileRequest,
};
use hanafubuki_agent_kernel::{AgentGeneration, AgentRevision, AgentSessionId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What the Agent settings surface needs to know about Skills in one session:
/// which are available and which the current turn already adopted. Those are
/// the only two states a Skill has — a Skill is on or off in the catalog, and
/// a turn either adopted it or did not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkillSessionState {
    pub session_id: AgentSessionId,
    pub generation: AgentGeneration,
    pub revision: AgentRevision,
    pub available: Vec<SkillId>,
    pub current_turn_active: Vec<SkillId>,
}

/// Install a Skill from a directory the user picked.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalSkillInstallRequest {
    pub source_directory: PathBuf,
    /// Replace a managed Skill that already occupies the same folder name.
    #[serde(default)]
    pub overwrite: bool,
}

impl AgentApplicationService {
    pub fn list_skills(&self) -> Result<SkillCatalogSnapshot, String> {
        self.enrich_skill_catalog_dependencies(self.skills.list()?)
    }

    pub fn list_skills_with_force_reload(
        &self,
        force_reload: bool,
    ) -> Result<SkillCatalogSnapshot, String> {
        self.enrich_skill_catalog_dependencies(self.skills.list_force_reload(force_reload)?)
    }

    /// Whether the MCP servers a Skill declares are actually reachable is the
    /// MCP authority's answer, not the catalog's.
    fn enrich_skill_catalog_dependencies(
        &self,
        mut snapshot: SkillCatalogSnapshot,
    ) -> Result<SkillCatalogSnapshot, String> {
        for entry in &mut snapshot.entries {
            let statuses = self
                .mcp_application
                .skill_dependency_statuses(&entry.dependencies)?;
            entry.availability = super::mcp::dependency_availability(&statuses);
        }
        Ok(snapshot)
    }

    pub fn skill_session_state(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentSkillSessionState, String> {
        let snapshot = self
            .core
            .snapshot(session_id)
            .map_err(|error| error.to_string())?;
        let available = self
            .list_skills()?
            .entries
            .into_iter()
            .filter(|entry| {
                entry.enabled
                    && entry.availability.state
                        == hanafubuki_agent_skills::SkillAvailabilityState::Ready
            })
            .map(|entry| entry.id)
            .collect();
        Ok(AgentSkillSessionState {
            session_id,
            generation: snapshot.generation,
            revision: snapshot.revision,
            available,
            current_turn_active: self
                .core
                .active_skills(session_id)
                .map_err(|error| error.to_string())?,
        })
    }

    pub fn read_skill(&self, id: &SkillId) -> Result<SkillRead, String> {
        self.skills.read(id)
    }

    pub fn read_skill_file(&self, id: &SkillId, path: &str) -> Result<SkillFileRead, String> {
        self.skills.read_file(id, path)
    }

    pub fn read_skill_resource(
        &self,
        id: &SkillId,
        path: &str,
    ) -> Result<SkillResourceRead, String> {
        self.skills.read_enabled_resource(id, path)
    }

    pub fn enable_skill(&self, id: &SkillId) -> Result<SkillCatalogSnapshot, String> {
        self.skills.enable(id)
    }

    pub fn disable_skill(&self, id: &SkillId) -> Result<SkillCatalogSnapshot, String> {
        self.skills.disable(id)
    }

    pub fn inspect_skill_source(
        &self,
        source_directory: &std::path::Path,
    ) -> Result<SkillSourceInspection, String> {
        self.skills.inspect_source(source_directory)
    }

    pub fn install_local_skill(
        &self,
        request: LocalSkillInstallRequest,
    ) -> Result<SkillCatalogEntry, String> {
        self.skills.install(SkillInstallRequest {
            source_directory: request.source_directory,
            overwrite: request.overwrite,
        })
    }

    pub fn create_skill(&self, request: SkillCreateRequest) -> Result<SkillCatalogEntry, String> {
        self.skills.create(request)
    }

    pub fn duplicate_skill(
        &self,
        request: SkillDuplicateRequest,
    ) -> Result<SkillCatalogEntry, String> {
        self.skills.duplicate(request)
    }

    pub fn write_skill_file(
        &self,
        request: SkillWriteFileRequest,
    ) -> Result<SkillCatalogEntry, String> {
        self.skills.write_file(request)
    }

    pub fn delete_skill_file(
        &self,
        request: SkillDeleteFileRequest,
    ) -> Result<SkillCatalogEntry, String> {
        self.skills.delete_file(request)
    }

    pub fn delete_skill(&self, id: &SkillId) -> Result<SkillCatalogSnapshot, String> {
        self.skills.delete(id)
    }

    /// The roots the Skill catalog reads now, for its watcher; none when the
    /// catalog did not open.
    pub(crate) fn skill_discovery_roots(&self) -> Option<Vec<PathBuf>> {
        self.skills.discovery_roots()
    }
}

impl SkillCatalogInvalidationPort for AgentApplicationService {
    fn invalidate_root(&self, root: &std::path::Path) -> Result<(), String> {
        self.skills.invalidate_root(root)
    }

    fn set_watcher_diagnostic(&self, message: Option<String>) {
        self.skills.set_watcher_diagnostic(message);
    }
}
