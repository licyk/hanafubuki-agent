//! The update check at the terminal interface's launch
//! (site/content/docs/terminal-interface.en.mdx, "Updates"): the prompt for
//! a newer release an earlier check left in the cache, shown before the
//! Agent starts; the notice of it, shown from the first frame; and, at
//! most every 20 hours, a check on a thread of its own whose newer release
//! the interface shows when it is found. Only `interactive::run` starts
//! it, after its terminal check; `tui.checkForUpdates = false`,
//! `HANAFUBUKI_AGENT_NO_UPDATE_CHECK`, a debug build without the test
//! release URL (it has no release lookup) and a build from a checkout turn
//! all of it off.

mod cache;
mod check;
mod prompt;

use std::ffi::OsString;
use std::path::PathBuf;

use hanafubuki_agent_tui::{TuiUpdateChoice, TuiUpdateNotice, TuiUpdateNotifier, TuiUpdatePrompt};
use hanafubuki_agent_update::{
    current_version, is_newer, sweep_outdated, InstallKind, Installation, ManualReason,
    ReleaseClient, UpdateAction, UpdateError, UpdateProxy, Version,
};

use crate::agent::Standalone;
use crate::exit::Failure;
use cache::UpdateCache;
use check::{now_ms, Check};

/// Any non-empty value turns the check and its notice off.
const NO_UPDATE_CHECK_ENV: &str = "HANAFUBUKI_AGENT_NO_UPDATE_CHECK";

/// How the launch goes on.
pub(crate) enum AtLaunch {
    /// Into the interface, with the notice of a newer release the cache
    /// holds.
    Interface(Option<TuiUpdateNotice>),
    /// "Update now" was chosen and the update ran; the process ends with
    /// its result, nothing of the Agent having started.
    Exit(Result<u8, Failure>),
}

/// At the interface's launch, before the Agent starts: on Windows the
/// executables earlier updates moved aside removed; then, unless the check
/// is off, the prompt (not when a prompt argument was given), the notice
/// the cache holds, and the check started when it is due.
pub(crate) fn at_launch(
    standalone: &Standalone,
    notifier: TuiUpdateNotifier,
    prompt_argument: bool,
) -> AtLaunch {
    let Ok(settings) = standalone.runtime.config().standalone_settings() else {
        return AtLaunch::Interface(None);
    };
    let Ok(installation) = Installation::current() else {
        return AtLaunch::Interface(None);
    };
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
    let started = launch.start(
        allowed,
        || ReleaseClient::for_this_build(&proxy, None),
        (!prompt_argument).then_some(prompt::ask),
        notifier,
    );
    match started {
        Started::Interface(notice) => AtLaunch::Interface(notice),
        // A fresh lookup, with the token `update` sends.
        Started::UpdateNow(installation) => AtLaunch::Exit(
            crate::update::client(&proxy)
                .and_then(|client| crate::update::update(&installation, &client, false)),
        ),
    }
}

/// What the launch knows, read from the process.
struct Launch {
    data_dir: PathBuf,
    installation: Installation,
    current: Version,
    now_ms: u64,
}

/// What [`Launch::start`] leaves to do.
enum Started {
    Interface(Option<TuiUpdateNotice>),
    UpdateNow(Installation),
}

impl Launch {
    /// On Windows, the executables earlier updates moved aside removed,
    /// whether the check is on or not. Then the prompt, put to `ask` when
    /// there is one; then the cache's notice and, when due, the check over
    /// the client `client` makes. Nothing of the check when not `allowed`
    /// or without a client (a debug build without the test release URL); no
    /// prompt without `ask` (a prompt argument).
    fn start(
        self,
        allowed: bool,
        client: impl FnOnce() -> Result<ReleaseClient, UpdateError>,
        ask: Option<impl FnOnce(TuiUpdatePrompt) -> TuiUpdateChoice>,
        notifier: TuiUpdateNotifier,
    ) -> Started {
        // One directory read; a file a running process still maps stays.
        sweep_outdated(&self.installation);
        if !allowed {
            return Started::Interface(None);
        }
        let client = match client() {
            Ok(client) => client,
            Err(error) => {
                log::debug!("update check: {}", error.kind());
                return Started::Interface(None);
            }
        };
        let cache = UpdateCache::load(&self.data_dir);
        let newer = cache
            .latest_version
            .as_ref()
            .filter(|latest| is_newer(latest, &self.current));
        let action = newer.map(|_| self.installation.action());
        if let (Some(latest), Some(action), Some(ask)) = (newer, &action, ask) {
            let offered = prompt::offer(
                latest,
                cache.dismissed_version.as_ref(),
                action,
                &self.current,
            );
            match offered.map(ask) {
                Some(TuiUpdateChoice::UpdateNow) => return Started::UpdateNow(self.installation),
                Some(TuiUpdateChoice::SkipVersion) => prompt::dismiss(&self.data_dir, latest),
                Some(TuiUpdateChoice::Skip) | None => {}
            }
        }
        // Shown for a dismissed version too: only the prompt respects it.
        let cached = newer
            .zip(action)
            .map(|(latest, action)| notice(latest, &action));
        if cache.is_due(self.now_ms) {
            Check {
                client,
                data_dir: self.data_dir,
                installation: self.installation,
            }
            .start(notifier);
        }
        Started::Interface(cached)
    }
}

/// Whether the check runs: the setting on, the variable unset or empty,
/// and not a build from a checkout, which is never offered a release.
fn allowed(setting: bool, no_update_check: Option<OsString>, kind: &InstallKind) -> bool {
    setting
        && no_update_check.is_none_or(|value| value.is_empty())
        && !matches!(kind, InstallKind::Checkout)
}

/// The notice of `latest`, with the command that installs it here, where
/// `action` is what can be done.
fn notice(latest: &Version, action: &UpdateAction) -> TuiUpdateNotice {
    TuiUpdateNotice {
        version: latest.to_string(),
        action: action_text(action).to_string(),
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
