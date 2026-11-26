use std::num::NonZeroUsize;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::{deps, tag, version};

#[derive(Debug, Parser)]
#[command(
    name = "cargo xtask",
    bin_name = "cargo xtask",
    about = "Repository checks, asset generation and release preparation"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub task: Task,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Task {
    /// Run every repository check.
    Check,
    /// Check the workspace's dependency direction and layers.
    CheckDependencies,
    /// Check the Rust module and test layout.
    CheckSourceLayout,
    /// Check file and function sizes against their limits.
    CheckModuleSize,
    /// Check that deny_unknown_fields is only on listed types.
    CheckStrictDecoding,
    /// Check that hanafubuki-agent-model-api names no context section kind.
    CheckModelApiContext,
    /// Check that paths resolve only through the foundation's authority.
    CheckPathBoundaries,
    /// Check that production code uses no std Mutex or RwLock.
    CheckLocks,
    /// Check that plugin production code does not unwrap or expect.
    CheckPluginPanics,
    /// Check the third-party attribution of derived files and NOTICE.
    CheckNotices,
    /// Check that every setting has its row in the configuration reference.
    CheckConfigReference,
    /// Print the largest files and functions.
    ModuleSizeReport {
        /// Number of files and functions to show.
        #[arg(long, default_value = "20", value_name = "N")]
        top: NonZeroUsize,
    },
    /// Generate welcome animation frames and audit artifacts.
    WelcomeAssets {
        /// Source artwork directory.
        #[arg(long, value_name = "DIR")]
        source: PathBuf,
        /// Atlas destination (default: crates/tui/assets/welcome in the workspace).
        #[arg(long, value_name = "DIR")]
        output: Option<PathBuf>,
    },
    /// Render exported terminal cells as a colored SVG.
    WelcomePreview {
        /// Exported terminal cells.
        #[arg(long, value_name = "JSON")]
        input: PathBuf,
        /// SVG destination.
        #[arg(long, value_name = "SVG")]
        output: PathBuf,
    },
    /// Bump the workspace version everywhere it is named.
    Version(version::Options),
    /// Create an annotated tag on HEAD (default: v<workspace version>).
    Tag(tag::Options),
    /// Upgrade Cargo.lock to the newest versions the manifests allow.
    Update(deps::Options),
    /// Rewrite Cargo.lock from the manifests without upgrading locked versions.
    Lock(deps::Options),
}

#[cfg(test)]
mod tests;
