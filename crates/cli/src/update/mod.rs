//! `update`: this executable updated to the latest release, or, with
//! `--check`, only told whether one is newer
//! (site/content/docs/cli-reference.en.mdx, "update"). It reads the proxy
//! from the configuration and starts no Agent: it takes no data
//! directory's lock, so it runs beside a running Agent, which keeps its
//! version until it restarts.

mod install;

use hanafubuki_agent::AgentRuntimeHost;
use hanafubuki_agent_update::{
    compare, current_version, release_notes_url, Comparison, InstallKind, Installation,
    ManualReason, Release, ReleaseClient, UpdateAction, UpdateError, UpdateProxy, Version,
};

use crate::agent::Standalone;
use crate::cli::GlobalArgs;
use crate::exit::{Failure, SUCCESS};

pub(crate) use install::update;

/// The token the release lookup sends, when set; never the download.
const TOKEN_ENV: &str = "GITHUB_TOKEN";
const INSTALLATION_PAGE: &str = "https://licyk.github.io/hanafubuki-agent/docs/installation/";

pub(crate) fn run(global: &GlobalArgs, check: bool, force: bool) -> Result<u8, Failure> {
    // First: once another process replaced the file, Linux reads this
    // process's executable as `<path> (deleted)`.
    let installation = Installation::current().map_err(failure)?;
    let standalone = Standalone::open(global, AgentRuntimeHost::default());
    let proxy = match &standalone {
        Ok(standalone) => proxy(standalone),
        Err(failure) => {
            eprintln!(
                "warning: the configuration could not be read ({}); the update uses the system \
                 proxy",
                failure.message
            );
            UpdateProxy::System
        }
    };
    let result = client(&proxy).and_then(|client| {
        if check {
            report(&installation, &client)
        } else {
            update(&installation, &client, force)
        }
    });
    if let Ok(standalone) = standalone {
        standalone.shutdown();
    }
    result
}

/// `network.proxy`, or the system's when the settings do not read.
pub(crate) fn proxy(standalone: &Standalone) -> UpdateProxy {
    standalone
        .runtime
        .config()
        .standalone_settings()
        .map(|settings| UpdateProxy::from_setting(&String::from(settings.network.proxy)))
        .unwrap_or_default()
}

/// The release lookup's client: `GITHUB_TOKEN` when set (D17).
pub(crate) fn client(proxy: &UpdateProxy) -> Result<ReleaseClient, Failure> {
    let token = match std::env::var(TOKEN_ENV) {
        Ok(token) => Some(token),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => return Err(failure(UpdateError::InvalidToken)),
    };
    ReleaseClient::for_this_build(proxy, token).map_err(failure)
}

/// `update --check`: the versions, the executable, how it is installed,
/// and what to do.
fn report(installation: &Installation, client: &ReleaseClient) -> Result<u8, Failure> {
    let release = client.latest_blocking().map_err(failure)?;
    let action = installation.action();
    let fields = [
        ("Current version", current_version().to_string()),
        ("Latest release", release.version.to_string()),
        (
            "Executable",
            installation.executable().native_text().to_string(),
        ),
        ("Installed as", installed_as(installation, &action)),
        ("Release notes", release_notes_url(&release.version)),
    ];
    for (label, value) in fields {
        println!("{label:<17}{value}");
    }
    println!();
    println!("{}", conclusion(installation, &action, &release));
    Ok(SUCCESS)
}

fn installed_as(installation: &Installation, action: &UpdateAction) -> String {
    match installation.kind() {
        InstallKind::Release if *action == UpdateAction::Manual(ManualReason::Unwritable) => {
            "a released binary (its directory is not writable)".to_string()
        }
        InstallKind::Release => "a released binary".to_string(),
        InstallKind::Cargo { root, .. } => format!("cargo install (in {})", root.display()),
        InstallKind::Checkout => "a build from a checkout".to_string(),
    }
}

/// `--check`'s last line.
fn conclusion(installation: &Installation, action: &UpdateAction, release: &Release) -> String {
    let latest = &release.version;
    let current = current_version();
    match compare(latest, &current) {
        Comparison::Ahead => format!(
            "This build ({current}) is newer than the latest release ({latest}); nothing to \
             install."
        ),
        Comparison::UpToDate => format!("hanafubuki-agent {latest} is the latest release."),
        Comparison::Available => match action {
            UpdateAction::Replace => format!(
                "An update is available: run `hanafubuki-agent update` to install {latest}."
            ),
            UpdateAction::Cargo { command } => run_to_install(command, latest),
            UpdateAction::Manual(reason) => manual(installation, *reason, latest),
        },
    }
}

/// Why `update` cannot install `latest` here, and what to do instead.
fn manual(installation: &Installation, reason: ManualReason, latest: &Version) -> String {
    let directory = installation
        .executable()
        .native()
        .parent()
        .map(|directory| directory.display().to_string())
        .unwrap_or_default();
    match reason {
        ManualReason::Checkout => format!(
            "This executable was built from a checkout ({directory}): update the checkout and \
             build it again."
        ),
        ManualReason::Unwritable => format!(
            "An update is available, but {directory} is not writable: install {latest} by hand \
             ({INSTALLATION_PAGE}), or run the update as a user who can write there."
        ),
        ManualReason::NoReleaseForPlatform => format!(
            "An update is available, but there is no released binary for this platform: \
             install {latest} the way you installed this one."
        ),
        ManualReason::CargoOnWindows => run_to_install(
            &installation.kind().cargo_command().unwrap_or_default(),
            latest,
        ),
    }
}

fn run_to_install(command: &[String], latest: &Version) -> String {
    format!(
        "An update is available: run `{}` to install {latest}.",
        command.join(" ")
    )
}

fn failure(error: UpdateError) -> Failure {
    Failure::new(error.to_string())
}

#[cfg(test)]
mod tests;
