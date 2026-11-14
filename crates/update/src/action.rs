//! What can be done to update an installation: the notice's wording, the
//! prompt's first option and `update` all read it.

use crate::install::{InstallKind, Installation, CLI_CRATE};
use crate::target::this_target;

/// How this installation is updated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateAction {
    /// Replace the executable with the release's.
    Replace,
    /// Run this command (`cargo install --locked hanafubuki-agent-cli`,
    /// with `--root <root>` off cargo's default root), the program first.
    Cargo { command: Vec<String> },
    /// Nothing the updater can do: the person updates it by hand.
    Manual(ManualReason),
}

/// Why an installation is updated by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManualReason {
    /// A build from a checkout: rebuild it.
    Checkout,
    /// A released binary in a directory this process cannot write.
    Unwritable,
    /// A build for a platform no release is built for.
    NoReleaseForPlatform,
    /// A `cargo install` on Windows, where cargo cannot replace the running
    /// executable: the command is named, not run.
    CargoOnWindows,
}

impl InstallKind {
    /// For a cargo installation, the command that updates it: `cargo
    /// install --locked hanafubuki-agent-cli`, with `--root <root>` off
    /// cargo's default root, the program first. On Windows it is only
    /// named ([`ManualReason::CargoOnWindows`]).
    pub fn cargo_command(&self) -> Option<Vec<String>> {
        let Self::Cargo { root, default_root } = self else {
            return None;
        };
        let mut command: Vec<String> = ["cargo", "install", "--locked", CLI_CRATE]
            .map(String::from)
            .into();
        if !default_root {
            command.push("--root".to_string());
            command.push(root.to_string_lossy().into_owned());
        }
        Some(command)
    }
}

impl Installation {
    /// What can be done for this installation; for a released binary it
    /// probes the directory ([`Installation::writable`]).
    pub fn action(&self) -> UpdateAction {
        action_for(self.kind(), this_target(), cfg!(windows), || {
            self.writable()
        })
    }
}

/// The action for `kind` on a build for `target` (`windows` when it runs
/// there), `writable` probing a released binary's directory.
pub(crate) fn action_for(
    kind: &InstallKind,
    target: Option<&str>,
    windows: bool,
    writable: impl FnOnce() -> bool,
) -> UpdateAction {
    match kind {
        InstallKind::Checkout => UpdateAction::Manual(ManualReason::Checkout),
        InstallKind::Cargo { .. } if windows => UpdateAction::Manual(ManualReason::CargoOnWindows),
        InstallKind::Cargo { .. } => UpdateAction::Cargo {
            command: kind.cargo_command().unwrap_or_default(),
        },
        InstallKind::Release if target.is_none() => {
            UpdateAction::Manual(ManualReason::NoReleaseForPlatform)
        }
        InstallKind::Release if !writable() => UpdateAction::Manual(ManualReason::Unwritable),
        InstallKind::Release => UpdateAction::Replace,
    }
}

#[cfg(test)]
mod tests;
