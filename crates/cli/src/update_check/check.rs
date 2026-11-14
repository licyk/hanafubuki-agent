//! The check itself, on a thread of its own: one release lookup, its
//! answer kept in the cache, and the notice sent when the release is newer
//! than this build. It is silent: a failure leaves the cache as it was
//! (the next launch asks again) and is logged at `debug` as its kind
//! alone. Nothing waits for the thread; it ends with the process.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use hanafubuki_agent::panic::catch_panic;
use hanafubuki_agent_tui::TuiUpdateNotifier;
use hanafubuki_agent_update::{current_version, is_newer, Installation, ReleaseClient, Version};

use super::cache::UpdateCache;
use super::notice;

/// What the thread needs.
pub(super) struct Check {
    /// With no token: the check's one request is anonymous.
    pub(super) client: ReleaseClient,
    pub(super) data_dir: PathBuf,
    pub(super) installation: Installation,
}

impl Check {
    /// Runs on the thread `update-check`, which nothing joins.
    pub(super) fn start(self, notifier: TuiUpdateNotifier) {
        let spawned = std::thread::Builder::new()
            .name("update-check".to_string())
            .spawn(move || {
                if catch_panic(|| self.run(&notifier)).is_err() {
                    log::debug!("update check: panicked");
                }
            });
        if spawned.is_err() {
            log::debug!("update check: the thread did not start");
        }
    }

    fn run(self, notifier: &TuiUpdateNotifier) {
        let current = current_version();
        if let Some(newer) = check(&self.client, &self.data_dir, &current, now_ms()) {
            notifier.notify(notice(&newer, &self.installation.action()));
        }
    }
}

/// Looks the latest release up and keeps it in the cache in `data_dir`;
/// the release when it is newer than `current`.
pub(super) fn check(
    client: &ReleaseClient,
    data_dir: &Path,
    current: &Version,
    now_ms: u64,
) -> Option<Version> {
    let release = match client.latest_blocking() {
        Ok(release) => release,
        Err(error) => {
            log::debug!("update check: {}", error.kind());
            return None;
        }
    };
    let latest = release.version;
    let written = UpdateCache::update(data_dir, |cache| {
        cache.latest_version = Some(latest.clone());
        cache.checked_at_ms = Some(now_ms);
    });
    if written.is_err() {
        log::debug!("update check: the cache was not written");
    }
    is_newer(&latest, current).then_some(latest)
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

#[cfg(test)]
mod tests;
