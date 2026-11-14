//! The `hanafubuki-agent` binary's self-update, for the command line: the
//! release lookup on GitHub, the version rules, the release's asset names,
//! the kind of installation the running executable is, with what can be
//! done to update it, and the update itself: the archive downloaded beside
//! the executable, verified, its executable asked its version and put in
//! place, or `cargo install` run for a cargo installation.
//!
//! Only the CLI uses it: the facade never depends on it (`cargo xtask
//! check-dependencies`), so a host embedding the Agent has none of it. A
//! debug build never contacts GitHub: it looks releases up only at
//! `HANAFUBUKI_AGENT_TEST_RELEASE_URL`, which a release build compiles out.

mod action;
mod archive;
mod cargo;
mod cleanup;
mod client;
mod download;
mod error;
mod install;
mod lock;
mod release;
mod replace;
mod target;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
mod updater;
mod verify;
mod version;

pub use action::{ManualReason, UpdateAction};
pub use cargo::run_cargo;
pub use cleanup::sweep_outdated;
pub use client::{ReleaseClient, UpdateProxy};
pub use error::UpdateError;
pub use install::{InstallKind, Installation};
pub use release::{release_notes_url, Release, ReleaseAsset, RELEASES_PAGE};
pub use semver::Version;
pub use target::{asset_name, entry_path, this_target, RELEASE_TARGETS};
pub use updater::{install, install_blocking, Installed, Progress};
pub use version::{compare, current_version, is_newer, Comparison};
