//! What the facade's calls on the installed plugins take and return:
//! what to install, what an install or an uninstall did, the marketplaces
//! the user added, and the change the plugin event sink is told of.

use std::path::PathBuf;

use hanafubuki_agent_packages::{MarketplaceSourceType, PackagePermissions};
use serde::Serialize;

use super::AgentPackageStatus;
use crate::AgentHook;

/// The marketplace a package installed from a directory is filed under
/// when the caller names none: its id is `<name>@local`.
pub const LOCAL_MARKETPLACE: &str = "local";

/// What to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentPackageSource {
    /// The entry `plugin` of the added marketplace `marketplace`.
    Marketplace { marketplace: String, plugin: String },
    /// The package in the directory `path`, filed under `marketplace`
    /// ([`LOCAL_MARKETPLACE`] when `None`); its manifest names it.
    Directory {
        path: PathBuf,
        marketplace: Option<String>,
    },
}

/// What an install or an update did, and what the package asks for before
/// all of it works.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPackageInstall {
    /// The package as it is now: on, its content loaded.
    pub package: AgentPackageStatus,
    /// Where it was installed.
    pub root: PathBuf,
    /// The Skills its `commands/` became.
    pub migrated_commands: Vec<String>,
    /// What its manifest's reading left out.
    pub issues: Vec<String>,
    /// Its hooks, which do not run until they are trusted
    /// (`trust_hooks`).
    pub hooks_to_trust: Vec<AgentHook>,
    /// What its code asks for and the user has not granted yet, which its
    /// guest waits for (`grant_package_permissions`); none for a package
    /// of content, which runs no code of its own.
    pub permissions: Option<PackagePermissions>,
}

/// What an uninstall did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPackageUninstall {
    pub id: String,
    /// The data directory a purge deleted.
    pub purged_data: Option<PathBuf>,
}

/// A change to an installed package, as the plugin event sink is told of
/// it (`AgentPluginEventSink::package_changed`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPackageChange {
    /// `<plugin>@<marketplace>`.
    pub id: String,
    pub kind: AgentPackageChangeKind,
    /// Why it failed, for `failed`.
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPackageChangeKind {
    Installed,
    Removed,
    Enabled,
    Disabled,
    Updated,
    /// A call that would have changed it failed; it is as it was.
    Failed,
}

/// A marketplace the user added (`[marketplaces]`), with its plugins.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMarketplace {
    pub name: String,
    /// The name its manifest gives a person.
    pub display_name: Option<String>,
    pub source_type: MarketplaceSourceType,
    /// A git URL, or a local marketplace's directory.
    pub source: String,
    pub ref_name: Option<String>,
    pub sparse_paths: Vec<String>,
    /// Off turns off every package from it.
    pub enabled: bool,
    pub last_updated: Option<String>,
    pub last_revision: Option<String>,
    /// Where its manifest is read: the directory, or the checkout of a git
    /// marketplace.
    pub root: PathBuf,
    pub plugins: Vec<AgentMarketplacePlugin>,
    /// The entries its manifest has that cannot be used.
    pub issues: Vec<String>,
    /// Why it cannot be read at all; it lists no plugins then.
    pub error: Option<String>,
}

/// A plugin a marketplace offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMarketplacePlugin {
    /// `<plugin>@<marketplace>`.
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub description: Option<String>,
    /// A local source's manifest's version.
    pub version: Option<String>,
    /// The marketplace lets it be installed.
    pub installable: bool,
    pub installed: bool,
    /// Installed and on.
    pub enabled: bool,
}

/// What adding a marketplace did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMarketplaceAdded {
    pub name: String,
    pub root: PathBuf,
    /// The same source was added before; nothing changed.
    pub already_added: bool,
}

/// What upgrading one marketplace did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMarketplaceUpgrade {
    pub name: String,
    /// Its checkout was replaced by its remote's revision; otherwise it was
    /// up to date (or failed).
    pub upgraded: bool,
    pub error: Option<String>,
}
