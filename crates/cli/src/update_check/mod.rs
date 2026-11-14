//! The update check at the terminal interface's launch
//! (site/content/docs/terminal-interface.en.mdx, "Updates"): the notice an
//! earlier check left in the cache, shown from the first frame, and, at
//! most every 20 hours, a check on a thread of its own whose newer release
//! the interface shows when it is found. Only `interactive::run` starts
//! it, after its terminal check; `tui.checkForUpdates = false`,
//! `HANAFUBUKI_AGENT_NO_UPDATE_CHECK`, a debug build without the test
//! release URL (it has no release lookup) and a build from a checkout turn
//! all of it off.

mod cache;
mod check;

use std::ffi::OsString;
use std::path::PathBuf;

use hanafubuki_agent_tui::{TuiUpdateNotice, TuiUpdateNotifier};
use hanafubuki_agent_update::{
    current_version, is_newer, InstallKind, Installation, ManualReason, ReleaseClient,
    UpdateAction, UpdateError, UpdateProxy, Version,
};

use crate::agent::Standalone;
use cache::UpdateCache;
use check::{now_ms, Check};

/// Any non-empty value turns the check and its notice off.
const NO_UPDATE_CHECK_ENV: &str = "HANAFUBUKI_AGENT_NO_UPDATE_CHECK";

/// At the interface's launch: the notice the cache holds, and the check
/// started when it is due; `None`, and nothing started, when the check is
/// off.
pub(crate) fn at_launch(
    standalone: &Standalone,
    notifier: TuiUpdateNotifier,
) -> Option<TuiUpdateNotice> {
    let settings = standalone.runtime.config().standalone_settings().ok()?;
    let installation = Installation::current().ok()?;
    let allowed = allowed(
        settings.tui.check_for_updates,
        std::env::var_os(NO_UPDATE_CHECK_ENV),
        installation.kind(),
    );
    let proxy = UpdateProxy::from_setting(&String::from(settings.network.proxy));
    let launch = Launch {
        data_dir: standalone.runtime.storage_paths().root.clone(),
        installation,
        current: current_version(),
        now_ms: now_ms(),
    };
    launch.start(
        allowed,
        || ReleaseClient::for_this_build(&proxy, None),
        notifier,
    )
}

/// What the launch knows, read from the process.
struct Launch {
    data_dir: PathBuf,
    installation: Installation,
    current: Version,
    now_ms: u64,
}

impl Launch {
    /// The cache's notice and, when due, the check over the client
    /// `client` makes; nothing when not `allowed` or without a client (a
    /// debug build without the test release URL).
    fn start(
        self,
        allowed: bool,
        client: impl FnOnce() -> Result<ReleaseClient, UpdateError>,
        notifier: TuiUpdateNotifier,
    ) -> Option<TuiUpdateNotice> {
        if !allowed {
            return None;
        }
        let client = match client() {
            Ok(client) => client,
            Err(error) => {
                log::debug!("update check: {}", error.kind());
                return None;
            }
        };
        let cache = UpdateCache::load(&self.data_dir);
        let cached = cache
            .latest_version
            .as_ref()
            .filter(|latest| is_newer(latest, &self.current))
            .map(|latest| notice(latest, &self.installation));
        if cache.is_due(self.now_ms) {
            Check {
                client,
                data_dir: self.data_dir,
                installation: self.installation,
            }
            .start(notifier);
        }
        cached
    }
}

/// Whether the check runs: the setting on, the variable unset or empty,
/// and not a build from a checkout, which is never offered a release.
fn allowed(setting: bool, no_update_check: Option<OsString>, kind: &InstallKind) -> bool {
    setting
        && no_update_check.is_none_or(|value| value.is_empty())
        && !matches!(kind, InstallKind::Checkout)
}

/// The notice of `latest`, with the command that installs it here.
fn notice(latest: &Version, installation: &Installation) -> TuiUpdateNotice {
    TuiUpdateNotice {
        version: latest.to_string(),
        action: action_text(&installation.action()).to_string(),
    }
}

/// The footer's command: the update itself, cargo's for a cargo
/// installation (named on Windows too, where the updater does not run it),
/// else `update --check`, which says what to do.
fn action_text(action: &UpdateAction) -> &'static str {
    match action {
        UpdateAction::Replace => "hanafubuki-agent update",
        UpdateAction::Cargo { .. } | UpdateAction::Manual(ManualReason::CargoOnWindows) => {
            "cargo install --locked hanafubuki-agent-cli"
        }
        UpdateAction::Manual(_) => "hanafubuki-agent update --check",
    }
}

#[cfg(test)]
mod tests;
