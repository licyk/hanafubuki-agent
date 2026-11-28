//! What a guest may reach of the Agent: the host functions `view` and
//! `invoke` reach only the two services below and [`AgentActions`], each in
//! a realm its guest's entry shares with its host-services entry (`isolate`
//! with the label `guest:<id>`), which the guest's entry declares
//! (`inject`).
//!
//! Everything else stays out of reach: kardia's checked access finds only
//! what the guest's chain of fibers provides or declares, and nothing else
//! on that chain has a JSON view (`view`) or is a guest's service
//! (`invoke`). A guest that provides a service other than its `plugin` is
//! stopped (`Shared::on_service`).

use std::sync::Arc;

use hanafubuki_agent_foundation::paths::ResolvedPath;
use hanafubuki_agent_packages::WorkspaceAccess;
use hanafubuki_agent_workspace_fs::{
    DirectoryListing, FileReadResult, ListDirectoryRequest, ReadFileRequest, ReadFileStart,
    WorkspaceFsRequest, WorkspaceFsResult, WorkspaceFsService, MAX_DIRECTORY_DEPTH,
    MAX_LIST_ENTRIES, MAX_READ_PAGE_BYTES,
};
use kardia_core::{Context, PluginDef};
use kardia_hmr::{register_action, HostAction};
use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::Value;

use crate::package::{access_name, GuestPackage};
use crate::spawn::{SpawnContext, WorkspaceRoots};

/// The Agent's workspace as the guest may reach it: `{ "access", "roots" }`,
/// the roots only when it may read them.
pub(crate) const WORKSPACE_SERVICE: &str = "workspace";
/// The guest's own settings, its configuration.
pub(crate) const SETTINGS_SERVICE: &str = "settings";
/// What the guest calls with `invoke` ([`AgentActions`]).
pub(crate) const AGENT_SERVICE: &str = "agent";

/// The plugin of `package`'s host-services entry: it provides the workspace
/// as granted (nothing to a WASM guest, which reads no file), read again at
/// every view, the settings it is configured with, and the methods it may
/// call.
pub(crate) fn host_services(package: &GuestPackage, context: &Arc<SpawnContext>) -> PluginDef {
    let granted = package.granted.clone().unwrap_or_default();
    let access = match package.needs_grant() {
        true => package.permissions.workspace.min(granted.workspace),
        false => WorkspaceAccess::None,
    };
    let roots = context.workspace.clone();
    let files = context.files.clone();
    let id = package.id.key();
    PluginDef::new(move |ctx: Context, settings: Value| {
        let roots = roots.clone();
        let files = files.clone();
        let id = id.clone();
        async move {
            ctx.provide_serialized_as(WORKSPACE_SERVICE, WorkspaceView { access, roots })?;
            ctx.provide_serialized_as(SETTINGS_SERVICE, settings.clone())?;
            register_action(
                &ctx,
                AGENT_SERVICE,
                AgentActions {
                    files,
                    access,
                    settings,
                    id,
                },
            )?;
            Ok(())
        }
    })
    .name(format!("guest-host:{}", package.id.key()))
    .provides([WORKSPACE_SERVICE, SETTINGS_SERVICE, AGENT_SERVICE])
}

/// What the guest may call: the workspace through the Agent's file service,
/// which applies the workspace policy and the protected roots, and its own
/// settings. Each method answers a guest's error, which the call that made
/// it gets.
struct AgentActions {
    files: WorkspaceFsService,
    access: WorkspaceAccess,
    settings: Value,
    /// Whose guest this is, for its errors.
    id: String,
}

impl AgentActions {
    /// The file service, unless the guest may not read the workspace: a
    /// WASM guest never may.
    fn readable(&self) -> Result<&WorkspaceFsService, String> {
        match self.access {
            WorkspaceAccess::Read | WorkspaceAccess::Write => Ok(&self.files),
            WorkspaceAccess::None => Err(format!(
                "plugin {}: the workspace is not readable; a package's code reads it once the \
                 user grants `workspace: \"read\"`",
                self.id
            )),
        }
    }

    /// The listing of `dir`, a directory the workspace policy allows, and
    /// its children's.
    fn list(
        &self,
        dir: &str,
        depth: usize,
        entries: Option<usize>,
    ) -> Result<DirectoryListing, String> {
        let request = WorkspaceFsRequest::ListDirectory(ListDirectoryRequest {
            path: dir.to_string(),
            max_depth: depth.clamp(1, MAX_DIRECTORY_DEPTH),
            max_entries: entries
                .unwrap_or(MAX_LIST_ENTRIES)
                .clamp(1, MAX_LIST_ENTRIES),
        });
        match self
            .readable()?
            .execute(&request, None, &Default::default())
        {
            Ok(WorkspaceFsResult::DirectoryListing(listing)) => Ok(listing),
            Ok(other) => Err(format!("listing {dir} answered {other:?}")),
            Err(error) => Err(error.to_string()),
        }
    }

    /// `file`, a file the workspace policy allows, from byte `offset` to
    /// `end_line`.
    fn read(
        &self,
        file: &str,
        offset: usize,
        end_line: Option<u64>,
    ) -> Result<FileReadResult, String> {
        let request = WorkspaceFsRequest::ReadFile(ReadFileRequest {
            path: file.to_string(),
            start: ReadFileStart::Offset(offset),
            end_line,
            max_bytes: MAX_READ_PAGE_BYTES,
            line_numbers: false,
        });
        match self
            .readable()?
            .execute(&request, None, &Default::default())
        {
            Ok(WorkspaceFsResult::FileRead(read)) => Ok(read),
            Ok(other) => Err(format!("reading {file} answered {other:?}")),
            Err(error) => Err(error.to_string()),
        }
    }
}

impl HostAction for AgentActions {
    fn call(&self, method: &str, args: Value) -> kardia_core::Result<Value> {
        let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default();
        let whole = |key: &str| args.get(key).and_then(Value::as_u64).map(|n| n as usize);
        let answer = match method {
            "workspace.list" => self
                .list(
                    text("path"),
                    whole("depth").unwrap_or(1),
                    whole("maxEntries"),
                )
                .and_then(|listing| serde_json::to_value(listing).map_err(|e| e.to_string())),
            "workspace.read" => self
                .read(
                    text("path"),
                    whole("offset").unwrap_or(0),
                    args.get("endLine").and_then(Value::as_u64),
                )
                .and_then(|read| serde_json::to_value(read).map_err(|e| e.to_string())),
            "settings.get" => Ok(self.settings.clone()),
            other => Err(format!(
                "no method {other}: `workspace.list` and `workspace.read` read the workspace, \
                 and `settings.get` answers the guest's settings"
            )),
        };
        answer.map_err(kardia_core::Error::msg)
    }
}

struct WorkspaceView {
    access: WorkspaceAccess,
    roots: WorkspaceRoots,
}

impl Serialize for WorkspaceView {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let roots: Vec<String> = match self.access {
            WorkspaceAccess::None => Vec::new(),
            WorkspaceAccess::Read | WorkspaceAccess::Write => (self.roots)()
                .iter()
                .filter_map(|root| ResolvedPath::resolve(root).ok())
                .map(|root| root.native_text().to_string())
                .collect(),
        };
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("access", access_name(self.access))?;
        map.serialize_entry("roots", &roots)?;
        map.end()
    }
}
