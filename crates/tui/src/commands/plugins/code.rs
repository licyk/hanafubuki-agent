//! A code plugin's rows on its page: the grant of what its code asks for,
//! while it is not granted, and its guest's state.

use hanafubuki_agent::{AgentPackageStatus, PackagePermissions, WorkspaceAccess};

use super::pages::{action, call, info, After, Row};
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
