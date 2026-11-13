// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The external editor Ctrl+G opens the draft in (Codex's
//! `external_editor.rs`). Its command is `VISUAL`'s, else `EDITOR`'s, split
//! into a shell's words on Unix and by the Windows command line's rules on
//! Windows, where the program is looked up on `PATH` with `PATHEXT` (so
//! `code` finds `code.cmd`). The draft is written to a temporary `.md`
//! file, the editor runs on it in the foreground with the terminal's input
//! and output, and the file is read back once the editor exits
//! successfully.
//!
//! The editor is the person's own program, which the interface waits for
//! with the terminal handed over, as `/diff` waits for `git`: it is not a
//! process of the Agent's `ExecutorManager`. Codex's temporary directory
//! that the sandbox cannot write is not ported; the file is in the
//! system's temporary directory, and removed once read.

use std::fmt;
use std::fs;
use std::io;
use std::process::{Command, ExitStatus, Stdio};

/// `VISUAL` and `EDITOR`, read when the interface starts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EditorVariables {
    pub(crate) visual: Option<String>,
    pub(crate) editor: Option<String>,
}

/// The editor's program and its arguments, before the file's path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditorCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
}

/// Why the draft was not edited; it stays as it was.
#[derive(Debug)]
pub(crate) enum EditorError {
    /// Neither variable names an editor.
    Unset,
    /// The variable's value is not a command line (an unclosed quote).
    Unsplittable(&'static str),
    NotFound(String),
    Start(io::Error),
    TemporaryFile(io::Error),
    Exited(ExitStatus),
    /// The edited file is gone or not UTF-8.
    Unreadable(io::Error),
}

impl EditorVariables {
    pub(crate) fn from_env() -> Self {
        Self {
            visual: std::env::var("VISUAL").ok(),
            editor: std::env::var("EDITOR").ok(),
        }
    }

    /// `VISUAL`'s command, else `EDITOR`'s; a variable set to nothing but
    /// blanks counts as unset.
    pub(crate) fn command(&self) -> Result<EditorCommand, EditorError> {
        let (name, value) = [("VISUAL", &self.visual), ("EDITOR", &self.editor)]
            .into_iter()
            .find_map(|(name, value)| {
                let value = value.as_deref().filter(|value| !value.trim().is_empty())?;
                Some((name, value))
            })
            .ok_or(EditorError::Unset)?;
        let mut words = split(value).ok_or(EditorError::Unsplittable(name))?;
        if words.is_empty() {
            return Err(EditorError::Unsplittable(name));
        }
        let program = words.remove(0);
        Ok(EditorCommand {
            program,
            args: words,
        })
    }
}

/// Edits `seed` in the editor and returns what it saved.
pub(crate) fn run_editor(command: &EditorCommand, seed: &str) -> Result<String, EditorError> {
    // A path, with no handle left open: an editor on Windows could not
    // replace a file this process holds.
    let path = tempfile::Builder::new()
        .prefix("hanafubuki-agent-")
        .suffix(".md")
        .tempfile()
        .map(tempfile::NamedTempFile::into_temp_path)
        .map_err(EditorError::TemporaryFile)?;
    fs::write(&path, seed).map_err(EditorError::TemporaryFile)?;
    let status = Command::new(program(&command.program))
        .args(&command.args)
        .arg(&path)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => EditorError::NotFound(command.program.clone()),
            _ => EditorError::Start(error),
        })?;
    if !status.success() {
        return Err(EditorError::Exited(status));
    }
    fs::read_to_string(&path).map_err(EditorError::Unreadable)
}

impl fmt::Display for EditorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditorError::Unset => {
                write!(f, "Set VISUAL or EDITOR to edit the message in an editor.")
            }
            EditorError::Unsplittable(name) => write!(
                f,
                "The editor in {name} is not a command line (a quote is not closed)."
            ),
            EditorError::NotFound(program) => write!(
                f,
                "The editor `{program}` was not found; set VISUAL or EDITOR to an installed one."
            ),
            EditorError::Start(error) => write!(f, "The editor could not be started: {error}."),
            EditorError::TemporaryFile(error) => write!(
                f,
                "The draft could not be written to a temporary file for the editor: {error}."
            ),
            EditorError::Exited(status) => write!(
                f,
                "The editor exited with {status}; the draft is unchanged."
            ),
            EditorError::Unreadable(error) => write!(
                f,
                "The edited message could not be read ({error}); the draft is unchanged."
            ),
        }
    }
}

#[cfg(not(windows))]
fn split(value: &str) -> Option<Vec<String>> {
    shlex::split(value)
}

#[cfg(windows)]
fn split(value: &str) -> Option<Vec<String>> {
    Some(winsplit::split(value))
}

#[cfg(not(windows))]
fn program(name: &str) -> &str {
    name
}

/// A spawn by name finds only `.exe` files; `which` reads `PATHEXT` too.
#[cfg(windows)]
fn program(name: &str) -> std::path::PathBuf {
    program_in(
        name,
        std::env::var_os("PATH"),
        std::env::current_dir().unwrap_or_default(),
    )
}

#[cfg(windows)]
fn program_in(
    name: &str,
    path: Option<std::ffi::OsString>,
    cwd: std::path::PathBuf,
) -> std::path::PathBuf {
    which::which_in(name, path, cwd).unwrap_or_else(|_| std::path::PathBuf::from(name))
}

#[cfg(test)]
mod tests;
