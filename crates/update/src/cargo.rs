//! The update of a `cargo install` (D7): cargo rebuilds it from crates.io.
//! It runs with the person's environment and its output on the terminal,
//! in a directory of its own, so no project's `.cargo/config.toml` or
//! `rust-toolchain.toml` applies. Then the installed file is asked its
//! version: cargo may have installed it elsewhere (`CARGO_INSTALL_ROOT`,
//! `install.root`) and left this one as it was.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use semver::Version;

use crate::error::UpdateError;
use crate::install::Installation;
use crate::updater::Bounds;
use crate::verify;
use crate::version::current_version;

/// Runs `command` (the action's, `cargo` first; `--force` added with
/// `force`, which reinstalls the same version), waits for it, and returns
/// the version the installed executable then reports: newer than this
/// build, or with `force` any.
pub fn run_cargo(
    installation: &Installation,
    command: &[String],
    force: bool,
) -> Result<Version, UpdateError> {
    run_cargo_on(
        installation,
        command,
        force,
        std::env::var_os("PATH").as_deref(),
    )
}

pub(crate) fn run_cargo_on(
    installation: &Installation,
    command: &[String],
    force: bool,
    path: Option<&OsStr>,
) -> Result<Version, UpdateError> {
    let shown = command.join(" ");
    let cargo = path
        .and_then(find_cargo)
        .ok_or_else(|| UpdateError::CargoNotFound {
            command: shown.clone(),
        })?;
    let start_failed = |error: std::io::Error| UpdateError::CargoStart {
        reason: error.to_string(),
    };
    let directory = tempfile::tempdir().map_err(start_failed)?;
    let mut cargo = Command::new(cargo);
    cargo
        .args(command.iter().skip(1))
        .current_dir(directory.path())
        .stdin(Stdio::inherit());
    if force {
        cargo.arg("--force");
    }
    let status = cargo.status().map_err(start_failed)?;
    if !status.success() {
        return Err(UpdateError::CargoFailed {
            command: shown,
            status: status.to_string(),
            current: current_version(),
        });
    }
    installed_after(installation, force)
}

/// The version the installed executable reports after cargo succeeded.
fn installed_after(installation: &Installation, force: bool) -> Result<Version, UpdateError> {
    let executable = installation.executable();
    let reported = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| UpdateError::Runtime {
            reason: error.to_string(),
        })?
        .block_on(verify::installed_version(
            executable.system(),
            Bounds::DEFAULT.version,
        ));
    let current = current_version();
    match reported {
        Some(version) if force || crate::is_newer(&version, &current) => Ok(version),
        reported => Err(UpdateError::CargoElsewhere {
            executable: executable.native_text().to_string(),
            reported: reported.map(|version| format!("hanafubuki-agent {version}")),
            current,
        }),
    }
}

/// `cargo` in the absolute entries of `path`.
fn find_cargo(path: &OsStr) -> Option<PathBuf> {
    let name = format!("cargo{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(path)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join(&name))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests;
