//! The `hanafubuki-agent` binary's self-update, for the command line: the
//! release lookup on GitHub, the version rules, the release's asset names,
//! and the kind of installation the running executable is, with what can be
//! done to update it.
//!
//! Only the CLI uses it: the facade never depends on it (`cargo xtask
//! check-dependencies`), so a host embedding the Agent has none of it. A
//! debug build never contacts GitHub: it looks releases up only at
//! `HANAFUBUKI_AGENT_TEST_RELEASE_URL`, which a release build compiles out.

mod action;
mod client;
mod error;
mod install;
mod release;
mod target;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
mod version;

pub use action::{ManualReason, UpdateAction};
pub use client::{ReleaseClient, UpdateProxy};
pub use error::UpdateError;
pub use install::{InstallKind, Installation};
pub use release::{release_notes_url, Release, ReleaseAsset, RELEASES_PAGE};
pub use semver::Version;
pub use target::{asset_name, entry_path, this_target, RELEASE_TARGETS};
pub use version::{current_version, is_newer};
