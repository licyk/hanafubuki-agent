//! The update prompt at the interface's launch (D13 of the self-update
//! plan): before the Agent starts, when the cache's release is newer than
//! this build, it is not the version "Skip until next version" chose, and
//! this installation can be updated from here: replaced, or reinstalled by
//! cargo on Unix. The manual cases get no prompt; the notice names their
//! step.

use std::path::Path;

use hanafubuki_agent_tui::{TuiUpdateChoice, TuiUpdatePrompt};
use hanafubuki_agent_update::{release_notes_url, UpdateAction, Version};

use super::cache::UpdateCache;

/// The prompt for `latest`, a release newer than `current`; `None` when it
/// is the `dismissed` one or `action` is a manual step.
pub(super) fn offer(
    latest: &Version,
    dismissed: Option<&Version>,
    action: &UpdateAction,
    current: &Version,
) -> Option<TuiUpdatePrompt> {
    if dismissed == Some(latest) {
        return None;
    }
    let update_now = match action {
        UpdateAction::Replace => {
            format!("Update now (downloads {latest} and replaces this executable)")
        }
        UpdateAction::Cargo { command } => format!("Update now (runs `{}`)", command.join(" ")),
        UpdateAction::Manual(_) => return None,
    };
    Some(TuiUpdatePrompt {
        current: current.to_string(),
        latest: latest.to_string(),
        release_notes: release_notes_url(latest),
        update_now,
    })
}

/// The screen on the terminal. One that cannot be shown is a Skip: the
/// interface starts as it would have.
pub(super) fn ask(prompt: TuiUpdatePrompt) -> TuiUpdateChoice {
    hanafubuki_agent_tui::update_prompt(prompt).unwrap_or_else(|error| {
        log::warn!("the update prompt was not shown: {error}");
        TuiUpdateChoice::Skip
    })
}

/// "Skip until next version": `latest` kept as the dismissed version, the
/// cache's other fields as the file holds them now. A failed write only
/// means the prompt comes again; it is logged, not shown.
pub(super) fn dismiss(data_dir: &Path, latest: &Version) {
    let written = UpdateCache::update(data_dir, |cache| {
        cache.dismissed_version = Some(latest.clone());
    });
    if let Err(error) = written {
        log::warn!("the update prompt's choice was not kept: {error}");
    }
}

#[cfg(test)]
mod tests;
