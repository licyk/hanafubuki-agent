//! A code plugin's rows on its page: the grant of what its code asks for,
//! while it is not granted, its guest's state, and its settings, which a
//! form edits as JSON.

use hanafubuki_agent::{AgentPackageStatus, PackagePermissions, WorkspaceAccess};

use super::pages::{action, call, info, After, Choice, Page, Prompt, Row};
use crate::agent_bridge::PluginsCall;

/// **Grant its permissions**, for a code plugin whose grant does not cover
/// what its manifest asks for.
pub(super) fn grant_row(package: &AgentPackageStatus) -> Option<Row> {
    let guest = package
        .guest
        .as_ref()
        .filter(|guest| guest.granted_permissions.as_ref() != Some(&guest.permissions))?;
    Some(action(
        "Grant its permissions",
        &permissions_text(&guest.permissions),
        Some(call(
            PluginsCall::GrantPermissions {
                id: package.id.clone(),
                permissions: guest.permissions.clone(),
            },
            "Granting…",
            After::Stay,
        )),
    ))
}

/// The guest's state, and why it is not running.
pub(super) fn state_row(package: &AgentPackageStatus) -> Option<Row> {
    let status = &package.guest.as_ref()?.status;
    let state = format!("{:?}", status.state).to_lowercase();
    Some(info(
        "Code",
        &match &status.error {
            Some(error) => format!("{state}: {error}"),
            None => state,
        },
    ))
}

/// **Settings**: what its code is configured with, which the settings
/// form edits.
pub(super) fn settings_row(package: &AgentPackageStatus) -> Option<Row> {
    let guest = package.guest.as_ref()?;
    let current = settings_text(package);
    let described = match (&guest.config_schema, guest.settings.is_empty()) {
        (_, false) => current,
        (Some(_), true) => "none yet; its manifest describes them".to_string(),
        (None, true) => "none".to_string(),
    };
    Some(action(
        "Settings",
        &described,
        Some(Choice::Open(Page::Prompt(Prompt::Settings))),
    ))
}

/// Its settings as one line of JSON, the form's starting text.
pub(super) fn settings_text(package: &AgentPackageStatus) -> String {
    let settings = package
        .guest
        .as_ref()
        .map(|guest| guest.settings.clone())
        .unwrap_or_default();
    serde_json::Value::Object(settings).to_string()
}

/// What a code plugin's guest may reach, as its consent says it.
fn permissions_text(permissions: &PackagePermissions) -> String {
    let workspace = match permissions.workspace {
        WorkspaceAccess::None => "no workspace",
        WorkspaceAccess::Read => "reads the workspace",
        WorkspaceAccess::Write => "writes the workspace",
    };
    let network = if permissions.network {
        "uses the network"
    } else {
        "no network"
    };
    match permissions.env.as_slice() {
        [] => format!("Its code runs out of process: {workspace}, {network}"),
        env => format!(
            "Its code runs out of process: {workspace}, {network}, reads {}",
            env.join(", ")
        ),
    }
}
