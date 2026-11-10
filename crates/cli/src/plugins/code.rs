//! An installed plugin's code: the grant of what it asks for, its
//! settings, and how its guest is doing.

use std::io::{BufRead, IsTerminal, Write};
use std::time::{Duration, Instant};

use hanafubuki_agent::{
    AgentHandle, AgentPackageStatus, GuestState, PackagePermissions, WorkspaceAccess,
};
use serde_json::Value;

use super::print_json;
use crate::agent::block_on;
use crate::exit::Failure;

/// How long a guest has to settle (run, fail or be refused) after a change.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(30);
const SETTLE_POLL: Duration = Duration::from_millis(100);

/// `plugins grant ID`: running it is the consent to what it prints.
pub(crate) fn grant(agent: &AgentHandle, id: String) -> Result<(), Failure> {
    let package = installed(agent, &id)?;
    let Some(guest) = &package.guest else {
        return Err(Failure::new(format!(
            "plugin `{id}` has no code: it asks for nothing"
        )));
    };
    let shown = permissions_text(&guest.permissions);
    grant_permissions(agent, &id, guest.permissions.clone())?;
    println!("Granted plugin `{id}` {shown}.");
    let package = settled(agent, &id)?;
    print_guest(&package);
    Ok(())
}

pub(crate) fn grant_permissions(
    agent: &AgentHandle,
    id: &str,
    permissions: PackagePermissions,
) -> Result<(), Failure> {
    let id = id.to_string();
    block_on(agent, move |agent| async move {
        agent.grant_package_permissions(&id, permissions).await
    })
}

/// Asks on the terminal whether to grant `permissions` to `id`; with no
/// terminal to ask on, `--grant` is the only consent.
pub(crate) fn confirm_grant(id: &str, permissions: &PackagePermissions) -> Result<bool, Failure> {
    let asked = format!(
        "Plugin `{id}`'s code asks for {}.",
        permissions_text(permissions)
    );
    if !std::io::stdin().is_terminal() {
        return Err(Failure::usage(format!(
            "{asked} Pass --grant to grant it without being asked."
        )));
    }
    print!("{asked} Grant it? [y/N] ");
    std::io::stdout().flush().ok();
    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|error| Failure::new(format!("the answer could not be read: {error}")))?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

/// `plugins settings ID [--set JSON | --get]`.
pub(crate) fn settings(
    agent: &AgentHandle,
    id: String,
    set: Option<String>,
) -> Result<(), Failure> {
    let Some(set) = set else {
        let package = installed(agent, &id)?;
        let Some(guest) = package.guest else {
            return Err(Failure::new(format!(
                "plugin `{id}` has no code to configure"
            )));
        };
        return print_json(&serde_json::json!({
            "settings": guest.settings,
            "configSchema": guest.config_schema,
        }));
    };
    let value: Value = serde_json::from_str(&set)
        .map_err(|error| Failure::usage(format!("--set is not JSON: {error}")))?;
    let removed = value.is_null();
    let shown = id.clone();
    block_on(agent, move |agent| async move {
        agent.set_package_settings(&id, value).await
    })?;
    match removed {
        true => println!("Removed the settings of plugin `{shown}`."),
        false => println!("Set the settings of plugin `{shown}`."),
    }
    Ok(())
}

pub(crate) fn installed(agent: &AgentHandle, id: &str) -> Result<AgentPackageStatus, Failure> {
    let id = id.to_string();
    block_on(agent, move |agent| async move {
        let package = agent.package(&id).await?;
        package.ok_or_else(|| format!("plugin `{id}` is not installed"))
    })
}

/// The package `id` once its guest no longer starts or restarts, or as it
/// is after [`SETTLE_TIMEOUT`]. A guest of a package that is on reads
/// `stopped` for a moment while a new start replaces its last one.
pub(crate) fn settled(agent: &AgentHandle, id: &str) -> Result<AgentPackageStatus, Failure> {
    let deadline = Instant::now() + SETTLE_TIMEOUT;
    loop {
        let package = installed(agent, id)?;
        let state = package.guest.as_ref().map(|guest| guest.status.state);
        let moving = match state {
            Some(GuestState::Starting | GuestState::Restarting) => true,
            Some(GuestState::Stopped) => package.enabled,
            _ => false,
        };
        if !moving || Instant::now() >= deadline {
            return Ok(package);
        }
        std::thread::sleep(SETTLE_POLL);
    }
}

/// How the package's guest is doing: its state, why it is not running, what
/// its start left out, how its last process ended and its stderr.
pub(crate) fn print_guest(package: &AgentPackageStatus) {
    for issue in &package.issues {
        println!("  issue:    {issue}");
    }
    let Some(guest) = &package.guest else {
        println!("  It has no code.");
        return;
    };
    let status = &guest.status;
    println!("  guest:    {}", state_text(status.state));
    if let Some(error) = &status.error {
        println!("  error:    {error}");
    }
    for issue in &status.issues {
        println!("  issue:    {issue}");
    }
    if let Some(exit) = &status.last_exit {
        let mut text = format!("{:?}", exit.cause).to_lowercase();
        if let Some(code) = &exit.status {
            text.push_str(&format!(", {code}"));
        }
        if let Some(reason) = &exit.reason {
            text.push_str(&format!(": {reason}"));
        }
        println!("  exit:     {text}");
    }
    if status.state != GuestState::Running && !status.stderr_tail.trim().is_empty() {
        println!("  stderr:");
        for line in status.stderr_tail.trim_end().lines() {
            println!("    {line}");
        }
    }
}

pub(crate) fn state_text(state: GuestState) -> &'static str {
    match state {
        GuestState::Stopped => "stopped",
        GuestState::Blocked => "blocked",
        GuestState::Starting => "starting",
        GuestState::Running => "running",
        GuestState::Restarting => "restarting",
        GuestState::Failed => "failed",
    }
}

pub(crate) fn permissions_text(permissions: &PackagePermissions) -> String {
    let workspace = match permissions.workspace {
        WorkspaceAccess::None => "no workspace access",
        WorkspaceAccess::Read => "reading the workspace",
        WorkspaceAccess::Write => "writing the workspace",
    };
    let network = match permissions.network {
        true => "the network",
        false => "no network",
    };
    let env = match permissions.env.as_slice() {
        [] => "no environment variables".to_string(),
        names => format!("the environment variables {}", names.join(", ")),
    };
    format!("{workspace}, {network} and {env}")
}
