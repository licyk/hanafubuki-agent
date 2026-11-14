//! What can go wrong, one sentence each, saying what was kept.

use std::fmt;

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
        }
    }
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
        }
    }
}

impl std::error::Error for UpdateError {}
