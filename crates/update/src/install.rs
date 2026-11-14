//! The running executable and what kind of installation it is, decided
//! once from its real path: a build from a checkout, a `cargo install` from
//! a registry, or else a released binary.
//!
//! Resolved once (at `update`'s start, at the interface's launch) and then
//! passed along: a Linux process whose executable was replaced reads its
//! own path as `<path> (deleted)`, so a later resolution would fail exactly
//! when another process has just updated it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::{PathError, ResolvedPath};
use serde::Deserialize;
use serde_json::Value;

use crate::error::UpdateError;

/// The crate `cargo install` installs the binary from.
pub(crate) const CLI_CRATE: &str = "hanafubuki-agent-cli";
/// The name of the file `writable` creates and drops beside the executable.
const PROBE_PREFIX: &str = ".hanafubuki-agent-update-probe-";

/// The running executable, resolved, and its kind.
#[derive(Clone, Debug)]
pub struct Installation {
    executable: ResolvedPath,
    kind: InstallKind,
}

/// How the executable was installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallKind {
    /// A released binary (or anything not recognised as the others).
    Release,
    /// `cargo install` from a registry, into `root` (`<root>/bin` holds the
    /// executable); `default_root` when that is `$CARGO_HOME`, else
    /// `~/.cargo`.
    Cargo { root: PathBuf, default_root: bool },
    /// A build in a cargo profile directory, or a `cargo install --path` or
    /// `--git`.
    Checkout,
}

impl Installation {
    /// The running executable (`current_exe`), resolved.
    pub fn current() -> Result<Self, UpdateError> {
        let executable = std::env::current_exe().map_err(|error| UpdateError::Executable {
            reason: error.to_string(),
        })?;
        Self::at(&executable)
    }

    /// The executable at `path`, resolved, as if it were running.
    pub fn at(path: &Path) -> Result<Self, UpdateError> {
        let executable = ResolvedPath::resolve(path).map_err(executable_error)?;
        let kind = detect_kind(&executable, default_cargo_root().as_ref());
        Ok(Self { executable, kind })
    }

    /// The executable's real path (symlinks resolved).
    pub fn executable(&self) -> &ResolvedPath {
        &self.executable
    }

    pub fn kind(&self) -> &InstallKind {
        &self.kind
    }

    /// Whether a file can be created beside the executable: it creates and
    /// drops one.
    pub fn writable(&self) -> bool {
        let Some(directory) = self.executable.system().parent() else {
            return false;
        };
        tempfile::Builder::new()
            .prefix(PROBE_PREFIX)
            .tempfile_in(directory)
            .is_ok()
    }
}

fn executable_error(error: PathError) -> UpdateError {
    match error {
        PathError::NotFound(_) => UpdateError::ExecutableNotFound,
        error => UpdateError::Executable {
            reason: error.to_string(),
        },
    }
}

/// `$CARGO_HOME` when set, else `~/.cargo`, resolved; `None` when it does
/// not exist.
fn default_cargo_root() -> Option<ResolvedPath> {
    let root = std::env::var_os("CARGO_HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join(".cargo")))?;
    ResolvedPath::resolve(root).ok()
}

/// The kind of `executable`, `default_cargo_root` being cargo's default
/// installation root.
pub(crate) fn detect_kind(
    executable: &ResolvedPath,
    default_cargo_root: Option<&ResolvedPath>,
) -> InstallKind {
    let Some(directory) = executable.system().parent() else {
        return InstallKind::Release;
    };
    if directory.join("deps").is_dir() && directory.join(".fingerprint").is_dir() {
        return InstallKind::Checkout;
    }
    let root = executable.native().parent().and_then(cargo_root);
    let (Some(file_name), Some(root)) = (executable.file_name(), root) else {
        return InstallKind::Release;
    };
    match cargo_source(&root, &file_name.to_string_lossy()) {
        Some(CargoSource::Registry) => InstallKind::Cargo {
            default_root: default_cargo_root.is_some_and(|default| default.key() == root.key()),
            root: root.native().to_path_buf(),
        },
        Some(CargoSource::Local) => InstallKind::Checkout,
        None => InstallKind::Release,
    }
}

/// The parent of `directory` (a native path, to resolve again) when
/// `directory` is a `bin`.
fn cargo_root(directory: &Path) -> Option<ResolvedPath> {
    if directory.file_name()? != "bin" {
        return None;
    }
    ResolvedPath::resolve(directory.parent()?).ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CargoSource {
    /// `registry+…` or `sparse+…`.
    Registry,
    /// `path+…` or `git+…`.
    Local,
}

/// cargo's record of what it installed into a root, read tolerantly:
/// anything that does not read means cargo did not install it.
#[derive(Deserialize)]
struct Crates2 {
    #[serde(default)]
    installs: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct Crates2Install {
    #[serde(default)]
    bins: Vec<String>,
}

/// Where cargo installed the CLI's binary `file_name` into `root` from, by
/// `<root>/.crates2.json`.
fn cargo_source(root: &ResolvedPath, file_name: &str) -> Option<CargoSource> {
    let text = fs::read(root.system().join(".crates2.json")).ok()?;
    let record: Crates2 = serde_json::from_slice(&text).ok()?;
    let source = record.installs.into_iter().find_map(|(key, install)| {
        let source = cli_install_source(&key)?.to_string();
        let install: Crates2Install = serde_json::from_value(install).ok()?;
        install
            .bins
            .iter()
            .any(|bin| bin == file_name)
            .then_some(source)
    })?;
    if source.starts_with("registry+") || source.starts_with("sparse+") {
        Some(CargoSource::Registry)
    } else if source.starts_with("path+") || source.starts_with("git+") {
        Some(CargoSource::Local)
    } else {
        None
    }
}

/// The source of an `installs` key of the CLI's crate,
/// `hanafubuki-agent-cli <version> (<source>)`.
fn cli_install_source(key: &str) -> Option<&str> {
    let (name, rest) = key.split_once(' ')?;
    if name != CLI_CRATE {
        return None;
    }
    rest.split_once(" (")?.1.strip_suffix(')')
}

#[cfg(test)]
mod tests;
