//! The version rules: semver precedence (build metadata ignored), stable
//! releases only, never down.

use std::cmp::Ordering;

use semver::Version;

/// This build's version, `[workspace.package].version`.
pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("cargo requires a semver package version")
}

/// Whether `latest` is a stable release newer than `current` by precedence.
/// A pre-release is never newer; equal precedence (`0.2.0+local` against
/// `0.2.0`) is up to date.
pub fn is_newer(latest: &Version, current: &Version) -> bool {
    latest.pre.is_empty() && latest.cmp_precedence(current) == Ordering::Greater
}

/// The latest release against this build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    /// The release is newer: it can be installed.
    Available,
    /// The same version by precedence: `--force` reinstalls it.
    UpToDate,
    /// This build is newer than the latest release: nothing is installed.
    Ahead,
}

/// How `latest`, a stable release, compares with `current` (D8).
pub fn compare(latest: &Version, current: &Version) -> Comparison {
    if is_newer(latest, current) {
        Comparison::Available
    } else if latest.cmp_precedence(current) == Ordering::Equal {
        Comparison::UpToDate
    } else {
        Comparison::Ahead
    }
}

/// Why a release tag is not one this version understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TagProblem {
    /// Not `v<semver>`.
    NotAVersion,
    /// `v<semver>` with a pre-release part.
    PreRelease,
}

/// The version a release tag names: `v<semver>` without a pre-release part,
/// surrounding whitespace ignored.
pub(crate) fn release_version(tag: &str) -> Result<Version, TagProblem> {
    let version = tag
        .trim()
        .strip_prefix('v')
        .and_then(|version| Version::parse(version).ok())
        .ok_or(TagProblem::NotAVersion)?;
    if version.pre.is_empty() {
        Ok(version)
    } else {
        Err(TagProblem::PreRelease)
    }
}

#[cfg(test)]
mod tests;
