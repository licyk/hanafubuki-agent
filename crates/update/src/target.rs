//! The release's targets and asset names, a contract with
//! `.github/workflows/release.yml` (`tests/release_contract.rs`): the
//! updater of one version reads the assets of the next.

use semver::Version;

/// The targets `release.yml` builds, as `about.toml` lists them.
pub const RELEASE_TARGETS: [&str; 4] = [
    "x86_64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
];

/// The release target this build is, or `None` when no release is built for
/// it. An x86_64 build under Rosetta is x86_64.
pub fn this_target() -> Option<&'static str> {
    if cfg!(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    )) {
        Some(RELEASE_TARGETS[0])
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some(RELEASE_TARGETS[1])
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some(RELEASE_TARGETS[2])
    } else if cfg!(all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc"
    )) {
        Some(RELEASE_TARGETS[3])
    } else {
        None
    }
}

/// The archive of `version` for `target`:
/// `hanafubuki-agent-<version>-<target>.tar.gz`, `.zip` for Windows.
pub fn asset_name(version: &Version, target: &str) -> String {
    let extension = if is_windows(target) { "zip" } else { "tar.gz" };
    format!("{}.{extension}", directory(version, target))
}

/// The executable's path inside that archive:
/// `hanafubuki-agent-<version>-<target>/hanafubuki-agent`, `.exe` for
/// Windows.
pub fn entry_path(version: &Version, target: &str) -> String {
    let suffix = if is_windows(target) { ".exe" } else { "" };
    format!("{}/hanafubuki-agent{suffix}", directory(version, target))
}

fn directory(version: &Version, target: &str) -> String {
    format!("hanafubuki-agent-{version}-{target}")
}

fn is_windows(target: &str) -> bool {
    target.contains("-windows-")
}
