//! What can go wrong, one sentence each, saying what was kept.

use std::fmt;

use semver::Version;

/// A failure of the update. `Display` is the whole sentence after `error: `;
/// [`UpdateError::kind`] is a label for a log line, which carries nothing of
/// the answer, the request or a token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateError {
    /// The release API could not be reached, or did not answer in time.
    Unreachable { host: String, timed_out: bool },
    /// 403 or 429: GitHub's rate limit.
    RateLimited { status: u16 },
    /// 401: the token was rejected.
    TokenRejected,
    /// 404: no published release.
    NoRelease,
    /// Any other answer but a success.
    Status { status: u16 },
    /// The answer is not a stable, published release with a version tag.
    NotUnderstood { reason: String },
    /// A debug build without `HANAFUBUKI_AGENT_TEST_RELEASE_URL`.
    DebugBuild,
    /// `HANAFUBUKI_AGENT_TEST_RELEASE_URL` is not a URL the lookup takes.
    TestUrl { reason: String },
    /// `GITHUB_TOKEN` cannot be sent as a header.
    InvalidToken,
    /// The proxy cannot be used.
    Proxy,
    /// The HTTP client could not be set up.
    Client,
    /// The lookup's own runtime could not be started.
    Runtime { reason: String },
    /// The running executable is not where it was started from.
    ExecutableNotFound,
    /// The running executable's path could not be resolved.
    Executable { reason: String },
    /// This build is for a platform no release is built for.
    NoReleaseForPlatform,
    /// `--force` against a release older than this build.
    Older { current: Version, latest: Version },
    /// The release has no archive for this target (yet).
    NoArchive {
        version: Version,
        target: &'static str,
        archive: String,
    },
    /// The archive's asset gives no SHA-256 digest.
    NoDigest { version: Version, archive: String },
    /// Another update of the same executable holds its lock.
    LockHeld { executable: String, lock: String },
    /// A file could not be written beside the executable: `what` is the
    /// download, the update lock or the new executable.
    Write {
        what: &'static str,
        directory: String,
        reason: String,
    },
    /// The archive's download failed, before its last byte.
    Download { archive: String, reason: String },
    /// The download ended before the size the release gives.
    Truncated {
        archive: String,
        received: u64,
        expected: u64,
    },
    /// The download went on past the size the release gives.
    Overlong { archive: String },
    /// The download's SHA-256 is not the release's digest.
    DigestMismatch { archive: String },
    /// The archive could not be read.
    Archive { archive: String, reason: String },
    /// The archive has no regular file at the entry's path, or one larger
    /// than the bound.
    NoEntry { archive: String, entry: String },
    /// The new executable could not be started, or failed.
    NotRunnable { reason: String, current: Version },
    /// The new executable printed something else than its version.
    WrongVersion { reported: String, expected: Version },
    /// The new executable did not answer `--version` in time.
    VersionTimeout { seconds: u64 },
    /// The new executable could not be renamed over the old (Unix).
    Replace { executable: String, reason: String },
    /// Windows: the running executable could not be moved aside.
    MoveAside {
        executable: String,
        reason: String,
        held: bool,
    },
    /// Windows: the new executable could not be put in place, and the
    /// previous one was put back.
    Restored { executable: String, reason: String },
    /// Windows: neither the new executable nor the previous one could be
    /// put in place.
    NotRestored {
        executable: String,
        backup: String,
        reason: String,
        rollback: String,
    },
    /// No `cargo` on `PATH`.
    CargoNotFound { command: String },
    /// `cargo` could not be started.
    CargoStart { reason: String },
    /// `cargo install` failed.
    CargoFailed {
        command: String,
        status: String,
        current: Version,
    },
    /// `cargo install` succeeded, but the executable is not a newer
    /// version: cargo installed it elsewhere, or it does not answer.
    CargoElsewhere {
        executable: String,
        reported: Option<String>,
        current: Version,
    },
}

impl UpdateError {
    /// A short label for a log line (`update check: <kind>`).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Unreachable {
                timed_out: true, ..
            } => "timed out",
            Self::Unreachable { .. } => "unreachable",
            Self::RateLimited { .. } => "rate limited",
            Self::TokenRejected => "token rejected",
            Self::NoRelease => "no release",
            Self::Status { .. } => "status",
            Self::NotUnderstood { .. } => "not understood",
            Self::DebugBuild => "debug build",
            Self::TestUrl { .. } => "test url",
            Self::InvalidToken => "invalid token",
            Self::Proxy => "proxy",
            Self::Client => "client",
            Self::Runtime { .. } => "runtime",
            Self::ExecutableNotFound => "executable not found",
            Self::Executable { .. } => "executable",
            Self::NoReleaseForPlatform => "no release for this platform",
            Self::Older { .. } => "older release",
            Self::NoArchive { .. } => "no archive",
            Self::NoDigest { .. } => "no digest",
            Self::LockHeld { .. } => "lock held",
            Self::Write { .. } => "write",
            Self::Download { .. } => "download",
            Self::Truncated { .. } => "truncated",
            Self::Overlong { .. } => "overlong",
            Self::DigestMismatch { .. } => "digest mismatch",
            Self::Archive { .. } => "archive",
            Self::NoEntry { .. } => "no entry",
            Self::NotRunnable { .. } => "not runnable",
            Self::WrongVersion { .. } => "wrong version",
            Self::VersionTimeout { .. } => "version timeout",
            Self::Replace { .. } => "replace",
            Self::MoveAside { .. } => "move aside",
            Self::Restored { .. } => "restored",
            Self::NotRestored { .. } => "not restored",
            Self::CargoNotFound { .. } => "cargo not found",
            Self::CargoStart { .. } => "cargo start",
            Self::CargoFailed { .. } => "cargo failed",
            Self::CargoElsewhere { .. } => "cargo elsewhere",
        }
    }

    /// The sentence of a failure to install, after the lookup.
    fn fmt_install(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoReleaseForPlatform => formatter.write_str(
                "there is no released binary for this platform; update it the way you installed it.",
            ),
            Self::Older { current, latest } => write!(
                formatter,
                "refusing to replace {current} with the older release {latest}; \
                 --force only reinstalls the same version."
            ),
            Self::NoArchive {
                version,
                target,
                archive,
            } => write!(
                formatter,
                "release {version} has no archive for {target} ({archive}); if it was just \
                 published, try again in a few minutes."
            ),
            Self::NoDigest { version, archive } => write!(
                formatter,
                "release {version} gives no SHA-256 digest for {archive}; nothing was changed."
            ),
            Self::LockHeld { executable, lock } => write!(
                formatter,
                "another update of {executable} is running ({lock}); try again when it ends."
            ),
            Self::Write {
                what,
                directory,
                reason,
            } => write!(
                formatter,
                "could not write {what} beside the executable ({directory}): {reason}; \
                 nothing was changed."
            ),
            Self::Download { archive, reason } => write!(
                formatter,
                "could not download {archive} ({reason}); nothing was changed. Try again."
            ),
            Self::Truncated {
                archive,
                received,
                expected,
            } => write!(
                formatter,
                "the download of {archive} ended after {} of {} MiB; nothing was changed. \
                 Try again.",
                mebibytes(*received),
                mebibytes(*expected)
            ),
            Self::Overlong { archive } => write!(
                formatter,
                "the download of {archive} is longer than the release says; nothing was \
                 changed. Try again."
            ),
            Self::DigestMismatch { archive } => write!(
                formatter,
                "the download of {archive} does not match its SHA-256 digest; nothing was \
                 changed. Try again."
            ),
            Self::Archive { archive, reason } => write!(
                formatter,
                "{archive} cannot be read ({reason}); nothing was changed."
            ),
            Self::NoEntry { archive, entry } => write!(
                formatter,
                "{archive} does not contain {entry} as a file; nothing was changed."
            ),
            Self::NotRunnable { reason, current } => {
                write!(
                    formatter,
                    "the new executable does not run here ({reason}); hanafubuki-agent \
                     {current} is unchanged."
                )?;
                if cfg!(target_os = "linux") {
                    formatter.write_str(" The Linux build needs glibc 2.35 or newer.")?;
                }
                Ok(())
            }
            Self::WrongVersion { reported, expected } => write!(
                formatter,
                "the new executable reports {reported}, not hanafubuki-agent {expected}; \
                 nothing was changed."
            ),
            Self::VersionTimeout { seconds } => write!(
                formatter,
                "the new executable did not answer --version within {seconds} s; nothing was \
                 changed."
            ),
            Self::Replace { executable, reason } => write!(
                formatter,
                "cannot put the new executable at {executable} ({reason}); nothing was changed."
            ),
            Self::MoveAside {
                executable,
                reason,
                held: true,
            } => write!(
                formatter,
                "cannot move {executable} aside: another program holds it open ({reason}); \
                 close it and try again. Nothing was changed."
            ),
            Self::MoveAside {
                executable, reason, ..
            } => write!(
                formatter,
                "cannot move {executable} aside ({reason}); nothing was changed."
            ),
            Self::Restored { executable, reason } => write!(
                formatter,
                "cannot put the new executable at {executable} ({reason}); the previous one \
                 was put back."
            ),
            Self::NotRestored {
                executable,
                backup,
                reason,
                rollback,
            } => write!(
                formatter,
                "cannot put the new executable at {executable} ({reason}), and the previous one \
                 could not be put back ({rollback}). Rename {backup} to {executable} by hand."
            ),
            Self::CargoNotFound { command } => write!(
                formatter,
                "cannot find cargo on PATH; run `{command}` yourself. Nothing was changed."
            ),
            Self::CargoStart { reason } => write!(
                formatter,
                "could not start cargo ({reason}); nothing was changed."
            ),
            Self::CargoFailed {
                command,
                status,
                current,
            } => write!(
                formatter,
                "{command} failed ({status}); hanafubuki-agent {current} is unchanged."
            ),
            Self::CargoElsewhere {
                executable,
                reported: Some(reported),
                current,
            } => write!(
                formatter,
                "cargo install finished, but {executable} still reports {reported}: cargo \
                 installed it elsewhere (CARGO_INSTALL_ROOT or install.root in cargo's \
                 configuration); hanafubuki-agent {current} is unchanged here."
            ),
            Self::CargoElsewhere { executable, .. } => write!(
                formatter,
                "cargo install finished, but {executable} does not answer --version; run it to \
                 see what cargo left there."
            ),
            // The lookup's and the installation's are written by `fmt`.
            _ => formatter.write_str(self.kind()),
        }
    }
}

/// `bytes` in MiB, to one decimal.
fn mebibytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / (1024.0 * 1024.0))
}

impl fmt::Display for UpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable { host, timed_out } => {
                let why = if *timed_out {
                    "timed out"
                } else {
                    "the connection failed"
                };
                write!(
                    formatter,
                    "could not reach {host} ({why}); check the network and network.proxy. \
                     Nothing was changed."
                )
            }
            Self::RateLimited { status } => write!(
                formatter,
                "GitHub refused the release lookup ({status}): its rate limit was probably \
                 reached. Try again later, or set GITHUB_TOKEN."
            ),
            Self::TokenRejected => formatter
                .write_str("GitHub rejected GITHUB_TOKEN (401); unset it or use a valid token."),
            Self::NoRelease => {
                formatter.write_str("the repository has no published release yet (404).")
            }
            Self::Status { status } => write!(
                formatter,
                "GitHub answered {status} to the release lookup; try again later."
            ),
            Self::NotUnderstood { reason } => write!(
                formatter,
                "GitHub's answer is not a release this version understands ({reason}); \
                 nothing was changed."
            ),
            Self::DebugBuild => formatter.write_str(
                "a debug build looks up releases only at HANAFUBUKI_AGENT_TEST_RELEASE_URL, \
                 which is not set; nothing was changed.",
            ),
            Self::TestUrl { reason } => write!(
                formatter,
                "HANAFUBUKI_AGENT_TEST_RELEASE_URL is not usable ({reason}); nothing was changed."
            ),
            Self::InvalidToken => formatter.write_str(
                "GITHUB_TOKEN is not a valid HTTP header value; unset it or use a valid token.",
            ),
            Self::Proxy => formatter
                .write_str("the proxy in network.proxy is not usable; nothing was changed."),
            Self::Client => formatter.write_str(
                "could not set up the HTTP client for the release lookup; nothing was changed.",
            ),
            Self::Runtime { reason } => write!(
                formatter,
                "could not start the release lookup ({reason}); nothing was changed."
            ),
            Self::ExecutableNotFound => formatter.write_str(
                "cannot find the running executable (it was moved or deleted); nothing was changed.",
            ),
            Self::Executable { reason } => write!(
                formatter,
                "cannot resolve the running executable ({reason}); nothing was changed."
            ),
            install => install.fmt_install(formatter),
        }
    }
}

impl std::error::Error for UpdateError {}
