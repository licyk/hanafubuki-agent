//! Asking an executable its `--version` (D21): the staged one, which must
//! answer exactly `hanafubuki-agent <version>` before it is put in place
//! (this is also what catches a wrong architecture or a too-old glibc), and
//! the installed one, to notice an update another process already made.
//!
//! The child is the updater's own: stdin closed, its output read bounded,
//! killed and waited for at the time bound, no console window on Windows.

use std::io;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use semver::Version;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};

use crate::error::UpdateError;

/// What an executable prints as its version starts with this.
const PROGRAM: &str = "hanafubuki-agent";
/// The most read of each of the child's outputs.
const MAX_OUTPUT_BYTES: u64 = 1024;
/// A spawn that meets a file another thread's fork briefly holds open for
/// writing (`ETXTBSY`) is tried again this many times.
const BUSY_ATTEMPTS: u32 = 5;

/// The child's answer.
#[derive(Debug)]
pub(crate) struct Answer {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: String,
    /// The first line of its standard error.
    pub(crate) stderr: String,
}

/// Why there is no answer.
#[derive(Debug)]
pub(crate) enum NoAnswer {
    Start(io::Error),
    TimedOut,
}

/// Requires the staged executable at `path` to answer exactly
/// `hanafubuki-agent <expected>` within `timeout`.
pub(crate) async fn check_staged(
    path: &Path,
    expected: &Version,
    timeout: Duration,
) -> Result<(), UpdateError> {
    let not_runnable = |reason: String| UpdateError::NotRunnable {
        reason,
        current: crate::current_version(),
    };
    let answer = match ask_version(path, timeout).await {
        Ok(answer) => answer,
        Err(NoAnswer::Start(error)) => return Err(not_runnable(error.to_string())),
        Err(NoAnswer::TimedOut) => {
            return Err(UpdateError::VersionTimeout {
                seconds: timeout.as_secs(),
            })
        }
    };
    if !answer.status.success() {
        let reason = if answer.stderr.is_empty() {
            answer.status.to_string()
        } else {
            answer.stderr
        };
        return Err(not_runnable(reason));
    }
    if reported_version(&answer.stdout) == Some(expected.to_string()) {
        return Ok(());
    }
    Err(UpdateError::WrongVersion {
        reported: quoted(answer.stdout.trim()),
        expected: expected.clone(),
    })
}

/// The version the executable at `path` reports, or `None` when it does
/// not answer as this program does.
pub(crate) async fn installed_version(path: &Path, timeout: Duration) -> Option<Version> {
    let answer = ask_version(path, timeout).await.ok()?;
    if !answer.status.success() {
        return None;
    }
    Version::parse(&reported_version(&answer.stdout)?).ok()
}

/// `--version`'s answer, read bounded, within `timeout`; the child is
/// killed and waited for when it does not end in time.
pub(crate) async fn ask_version(path: &Path, timeout: Duration) -> Result<Answer, NoAnswer> {
    let mut child = spawn(path).await.map_err(NoAnswer::Start)?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(NoAnswer::Start(io::Error::other("its output is not piped")));
    };
    let answered = tokio::time::timeout(timeout, async {
        let (stdout, stderr) = tokio::join!(read_bounded(stdout), read_bounded(stderr));
        let status = child.wait().await?;
        Ok::<_, io::Error>((status, stdout, stderr))
    })
    .await;
    match answered {
        Ok(Ok((status, stdout, stderr))) => Ok(Answer {
            status,
            stdout,
            stderr: first_line(&stderr),
        }),
        Ok(Err(error)) => Err(NoAnswer::Start(error)),
        Err(_) => {
            // On Windows a live child keeps its file from being removed.
            let _ = child.kill().await;
            Err(NoAnswer::TimedOut)
        }
    }
}

async fn spawn(path: &Path) -> io::Result<Child> {
    let mut attempt = 1;
    loop {
        let mut command = Command::new(path);
        command
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);
        match command.spawn() {
            Err(error)
                if error.kind() == io::ErrorKind::ExecutableFileBusy && attempt < BUSY_ATTEMPTS =>
            {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            spawned => return spawned,
        }
    }
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

async fn read_bounded(output: impl AsyncRead + Unpin) -> String {
    let mut bytes = Vec::new();
    let _ = output.take(MAX_OUTPUT_BYTES).read_to_end(&mut bytes).await;
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The version in `hanafubuki-agent <version>`, the output's whole first
/// line but for surrounding whitespace.
fn reported_version(stdout: &str) -> Option<String> {
    let line = stdout.trim();
    let version = line.strip_prefix(PROGRAM)?.strip_prefix(' ')?;
    (!version.is_empty() && !version.contains(char::is_whitespace)).then(|| version.to_string())
}

/// The first line of `text`, its control characters escaped.
fn first_line(text: &str) -> String {
    escaped(text.lines().next().unwrap_or_default().trim())
}

/// `text` in quotes, its control characters escaped, cut short.
fn quoted(text: &str) -> String {
    format!("\"{}\"", escaped(text))
}

fn escaped(text: &str) -> String {
    const MAX_CHARS: usize = 200;
    let mut escaped = String::new();
    for character in text.chars().take(MAX_CHARS) {
        if character.is_control() {
            escaped.extend(character.escape_default());
        } else {
            escaped.push(character);
        }
    }
    if text.chars().nth(MAX_CHARS).is_some() {
        escaped.push('…');
    }
    escaped
}

#[cfg(test)]
mod tests;
