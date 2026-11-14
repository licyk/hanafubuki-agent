//! The update of a released binary (§3.7): the release's archive for this
//! target, downloaded beside the executable, verified, its one entry taken
//! and asked its version, then put in place. Everything before the last
//! step leaves the installed file untouched, and the staging directory is
//! removed on every path.

use std::path::{Path, PathBuf};
use std::time::Duration;

use semver::Version;

use crate::archive::Extraction;
use crate::cleanup::{self, STAGING_PREFIX};
use crate::client::ReleaseClient;
use crate::download::Download;
use crate::error::UpdateError;
use crate::install::Installation;
use crate::lock::UpdateLock;
use crate::release::Release;
use crate::target::{asset_name, entry_path, this_target};
use crate::version::{compare, current_version, Comparison};
use crate::{replace, verify};

/// A step of the update, as it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// `done` of `total` bytes of the archive, at most every 100 ms, from 0
    /// to `total`.
    Downloading { done: u64, total: u64 },
    /// Comparing the archive's SHA-256 with the release's digest.
    Verifying,
    /// Asking the new executable its version.
    Checking,
    /// Putting the new executable in place.
    Installing,
}

/// What an update did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// The version now installed: the release's, or with `already` the
    /// one the installed executable reports.
    pub version: Version,
    /// Windows: where the previous executable was moved, until no process
    /// runs it.
    pub backup: Option<PathBuf>,
    /// The installed executable was already this version or a newer one
    /// (another process updated it, or this build is the release): nothing
    /// was downloaded.
    pub already: bool,
}

/// The time bounds of an update; the tests shorten them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bounds {
    /// A download without a byte for this long fails.
    pub(crate) stall: Duration,
    /// An executable that does not answer `--version` within this fails.
    pub(crate) version: Duration,
    pub(crate) progress_every: Duration,
}

impl Bounds {
    pub(crate) const DEFAULT: Self = Self {
        stall: Duration::from_secs(60),
        version: Duration::from_secs(10),
        progress_every: Duration::from_millis(100),
    };
}

/// An entry is at most this many times its archive's size: a release
/// binary is about three times its archive.
const MAX_EXPANSION: u64 = 16;

/// Replaces `installation`'s executable with `release`'s for this target.
/// `release` must be newer than this build, or the same version with
/// `force` (which reinstalls it); an older one is refused. Under the update
/// lock, an installed executable that already reports the release's
/// version or a newer one is left alone (`already`) unless `force`.
pub async fn install(
    client: &ReleaseClient,
    installation: &Installation,
    release: &Release,
    force: bool,
    progress: &mut dyn FnMut(Progress),
) -> Result<Installed, UpdateError> {
    install_within(
        client,
        installation,
        release,
        force,
        progress,
        Bounds::DEFAULT,
    )
    .await
}

/// [`install`] on a runtime of its own, for a thread that is not in one.
pub fn install_blocking(
    client: &ReleaseClient,
    installation: &Installation,
    release: &Release,
    force: bool,
    progress: &mut dyn FnMut(Progress),
) -> Result<Installed, UpdateError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| UpdateError::Runtime {
            reason: error.to_string(),
        })?
        .block_on(install(client, installation, release, force, progress))
}

pub(crate) async fn install_within(
    client: &ReleaseClient,
    installation: &Installation,
    release: &Release,
    force: bool,
    progress: &mut dyn FnMut(Progress),
    bounds: Bounds,
) -> Result<Installed, UpdateError> {
    let version = &release.version;
    let installed = |already, backup| Installed {
        version: version.clone(),
        backup,
        already,
    };
    match compare(version, &current_version()) {
        Comparison::Available => {}
        Comparison::UpToDate if force => {}
        Comparison::UpToDate => return Ok(installed(true, None)),
        Comparison::Ahead => {
            return Err(UpdateError::Older {
                current: current_version(),
                latest: version.clone(),
            })
        }
    }
    let target = this_target().ok_or(UpdateError::NoReleaseForPlatform)?;
    let archive = asset_name(version, target);
    let asset = release
        .asset(&archive)
        .ok_or_else(|| UpdateError::NoArchive {
            version: version.clone(),
            target,
            archive: archive.clone(),
        })?;
    let sha256 = asset.sha256.ok_or_else(|| UpdateError::NoDigest {
        version: version.clone(),
        archive: archive.clone(),
    })?;

    let executable = installation.executable();
    let (directory, shown_directory) = directories(installation)?;
    let _lock = UpdateLock::acquire(executable)?;
    cleanup::remove_stale_staging(directory);
    cleanup::sweep_outdated(installation);
    if !force {
        let reported = verify::installed_version(executable.system(), bounds.version).await;
        // Another process installed this release, or a newer build: never
        // replaced by an older one.
        if let Some(reported) = reported.filter(|reported| reported.cmp_precedence(version).is_ge())
        {
            return Ok(Installed {
                version: reported,
                backup: None,
                already: true,
            });
        }
    }

    let staging = tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
        .tempdir_in(directory)
        .map_err(|error| UpdateError::Write {
            what: "the download",
            directory: shown_directory.clone(),
            reason: error.to_string(),
        })?;
    let archive_path = staging.path().join(&archive);
    Download {
        http: client.http(),
        asset,
        sha256,
        destination: &archive_path,
        directory: &shown_directory,
        stall: bounds.stall,
        progress_every: bounds.progress_every,
    }
    .run(progress)
    .await?;
    let staged = staging.path().join(staged_name(target));
    Extraction {
        archive: &archive_path,
        archive_name: &archive,
        entry: &entry_path(version, target),
        max_bytes: asset.size.saturating_mul(MAX_EXPANSION),
        destination: &staged,
        directory: &shown_directory,
    }
    .run()?;
    keep_mode(executable.system(), &staged, &shown_directory)?;
    progress(Progress::Checking);
    verify::check_staged(&staged, version, bounds.version).await?;
    progress(Progress::Installing);
    let backup = replace::replace(executable, &staged)?;
    Ok(installed(false, backup))
}

/// The executable's directory, for the file operations and as shown.
fn directories(installation: &Installation) -> Result<(&Path, String), UpdateError> {
    let executable = installation.executable();
    match (executable.system().parent(), executable.native().parent()) {
        (Some(system), Some(native)) => Ok((system, native.display().to_string())),
        _ => Err(UpdateError::Executable {
            reason: "it has no directory".to_string(),
        }),
    }
}

/// The staged executable's name: the name inside the archive.
fn staged_name(target: &str) -> &'static str {
    if target.contains("-windows-") {
        "hanafubuki-agent.exe"
    } else {
        "hanafubuki-agent"
    }
}

/// Unix: the staged file takes the installed one's mode.
fn keep_mode(installed: &Path, staged: &Path, directory: &str) -> Result<(), UpdateError> {
    #[cfg(unix)]
    {
        let failed = |error: std::io::Error| UpdateError::Write {
            what: "the new executable",
            directory: directory.to_string(),
            reason: error.to_string(),
        };
        let permissions = std::fs::metadata(installed).map_err(failed)?.permissions();
        std::fs::set_permissions(staged, permissions).map_err(failed)?;
    }
    #[cfg(not(unix))]
    let _ = (installed, staged, directory);
    Ok(())
}

#[cfg(test)]
mod tests;
