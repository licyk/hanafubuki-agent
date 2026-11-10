//! Application use cases for the Tauri-free Skill catalog authority.

pub use hanafubuki_agent_skills::{
    SkillAssetRead, SkillAvailability, SkillAvailabilityState, SkillCatalogEntry,
    SkillCatalogListRequest, SkillCatalogSnapshot, SkillCreateRequest, SkillDeleteFileRequest,
    SkillDuplicateRequest, SkillError, SkillFileRead, SkillId, SkillInstallRequest, SkillInterface,
    SkillLoadIssue, SkillMcpTransport, SkillRead, SkillResourceDescriptor, SkillResourceKind,
    SkillResourceRead, SkillRoots, SkillScope, SkillSourceInspection, SkillToolDependency,
    SkillTurnCatalog, SkillWriteFileRequest,
};

use std::path::PathBuf;
use std::sync::Arc;

use hanafubuki_agent_skills::{repository_skill_roots, PluginSkillRoot, SkillService};

use crate::WorkspaceRootsPort;

pub trait SkillCatalogInvalidationPort: Send + Sync {
    fn invalidate_root(&self, root: &std::path::Path) -> Result<(), String>;
    fn set_watcher_diagnostic(&self, message: Option<String>);
}

/// The catalog authority as the rest of the application sees it: every call
/// forwards to the domain service and flattens its error into a message the
/// command layer can report. A catalog that could not open leaves the Agent
/// without Skills rather than without the Agent: every call returns why, and
/// the `skills` plugin fails with it.
///
/// With a `workspace`, the repository roots follow the Agent's workspace
/// roots: they are resolved again before every catalog read, so a turn and
/// the settings surface see the Skills of the workspace as it is now.
#[derive(Clone)]
pub(crate) struct AgentSkillApplication {
    service: Result<SkillService, String>,
    workspace: Option<Arc<dyn WorkspaceRootsPort>>,
}

impl AgentSkillApplication {
    pub(crate) fn open(roots: SkillRoots, workspace: Option<Arc<dyn WorkspaceRootsPort>>) -> Self {
        let service = SkillService::new(roots).map_err(|error| {
            log::error!("the Skill catalog did not open; the Agent runs without Skills: {error}");
            format!("the Skill catalog is unavailable: {error}")
        });
        let skills = Self { service, workspace };
        if let Err(error) = skills.follow_workspace() {
            log::warn!("agent Skill repository roots were not resolved: {error}");
        }
        skills
    }

    fn follow_workspace(&self) -> Result<(), String> {
        let (Ok(service), Some(workspace)) = (&self.service, &self.workspace) else {
            return Ok(());
        };
        let roots = workspace.roots()?;
        service
            .set_repo_roots(repository_skill_roots(roots.roots()))
            .map_err(to_message)
    }

    /// The catalog, or why it did not open.
    pub(crate) fn available(&self) -> Result<(), String> {
        self.service.as_ref().map(|_| ()).map_err(Clone::clone)
    }

    /// The roots discovery reads now, nearest repository first.
    pub(crate) fn discovery_roots(&self) -> Option<Vec<PathBuf>> {
        self.service.as_ref().ok().map(|service| {
            service
                .discovery_roots()
                .into_iter()
                .map(|(root, _)| root)
                .collect()
        })
    }

    fn service(&self) -> Result<&SkillService, String> {
        self.service.as_ref().map_err(Clone::clone)
    }

    pub(crate) fn list(&self) -> Result<SkillCatalogSnapshot, String> {
        self.follow_workspace()?;
        self.service()?.list().map_err(to_message)
    }

    pub(crate) fn list_force_reload(
        &self,
        force_reload: bool,
    ) -> Result<SkillCatalogSnapshot, String> {
        self.follow_workspace()?;
        self.service()?
            .list_with_request(SkillCatalogListRequest { force_reload })
            .map_err(to_message)
    }

    /// The enabled installed plugins' Skill roots, read from now on.
    pub(crate) fn set_plugin_roots(&self, roots: Vec<PluginSkillRoot>) -> Result<(), String> {
        self.service()?.set_plugin_roots(roots).map_err(to_message)
    }

    pub(crate) fn invalidate_root(&self, root: &std::path::Path) -> Result<(), String> {
        self.service()?.invalidate_root(root).map_err(to_message)
    }

    pub(crate) fn set_watcher_diagnostic(&self, message: Option<String>) {
        if let Ok(service) = &self.service {
            service.set_watcher_diagnostic(message);
        }
    }

    pub(crate) fn read(&self, id: &SkillId) -> Result<SkillRead, String> {
        self.service()?.read(id).map_err(to_message)
    }

    pub(crate) fn read_file(&self, id: &SkillId, path: &str) -> Result<SkillFileRead, String> {
        self.service()?.read_file(id, path).map_err(to_message)
    }

    pub(crate) fn read_enabled_resource(
        &self,
        id: &SkillId,
        path: &str,
    ) -> Result<SkillResourceRead, String> {
        self.service()?
            .read_enabled_resource(id, path)
            .map_err(to_message)
    }

    pub(crate) fn read_enabled_asset(
        &self,
        id: &SkillId,
        path: &str,
    ) -> Result<SkillAssetRead, String> {
        self.service()?
            .read_enabled_asset(id, path)
            .map_err(to_message)
    }

    pub(crate) fn enable(&self, id: &SkillId) -> Result<SkillCatalogSnapshot, String> {
        self.service()?.enable(id).map_err(to_message)
    }

    pub(crate) fn disable(&self, id: &SkillId) -> Result<SkillCatalogSnapshot, String> {
        self.service()?.disable(id).map_err(to_message)
    }

    pub(crate) fn inspect_source(
        &self,
        source_directory: &std::path::Path,
    ) -> Result<SkillSourceInspection, String> {
        self.service()?
            .inspect_source(source_directory)
            .map_err(to_message)
    }

    pub(crate) fn install(
        &self,
        request: SkillInstallRequest,
    ) -> Result<SkillCatalogEntry, String> {
        self.service()?.install(request).map_err(to_message)
    }

    pub(crate) fn create(&self, request: SkillCreateRequest) -> Result<SkillCatalogEntry, String> {
        self.service()?.create(request).map_err(to_message)
    }

    pub(crate) fn duplicate(
        &self,
        request: SkillDuplicateRequest,
    ) -> Result<SkillCatalogEntry, String> {
        self.service()?.duplicate(request).map_err(to_message)
    }

    pub(crate) fn write_file(
        &self,
        request: SkillWriteFileRequest,
    ) -> Result<SkillCatalogEntry, String> {
        self.service()?.write_file(request).map_err(to_message)
    }

    pub(crate) fn delete_file(
        &self,
        request: SkillDeleteFileRequest,
    ) -> Result<SkillCatalogEntry, String> {
        self.service()?.delete_file(request).map_err(to_message)
    }

    pub(crate) fn delete(&self, id: &SkillId) -> Result<SkillCatalogSnapshot, String> {
        self.service()?.delete(id).map_err(to_message)
    }

    pub(crate) fn turn_catalog(&self) -> Result<SkillTurnCatalog, String> {
        self.follow_workspace()?;
        self.service()?.turn_catalog().map_err(to_message)
    }
}

fn to_message(error: SkillError) -> String {
    error.to_string()
}
