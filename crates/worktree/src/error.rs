use std::fmt;
use std::path::PathBuf;

/// Why a worktree operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorktreeError {
    /// `git` did not start or failed; its message.
    Git(String),
    /// The path is not a worktree directly under the managed directory.
    NotManaged(PathBuf),
    /// The worktree already belongs to another session.
    Owned(String),
    /// The worktree is (or contains) the directory the request came from.
    InUse(PathBuf),
    Io(String),
}

impl fmt::Display for WorktreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(message) => write!(formatter, "git failed: {message}"),
            Self::NotManaged(path) => {
                write!(formatter, "{} is not an Agent worktree", path.display())
            }
            Self::Owned(owner) => write!(formatter, "the worktree belongs to session {owner}"),
            Self::InUse(path) => write!(
                formatter,
                "{} is the working directory; change to another directory first",
                path.display()
            ),
            Self::Io(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for WorktreeError {}
