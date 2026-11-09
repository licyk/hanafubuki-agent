//! Git worktrees the Agent makes for a session to work in apart from the
//! user's checkout: each a detached checkout of a repository's `HEAD`,
//! directly under one directory (the data directory's `worktrees/`), named
//! after its repository, with the session it belongs to recorded in its
//! Git metadata. Every `git` runs through the Agent's `ExecutorManager`.

mod error;
mod git;
mod manager;
mod owner;

pub use error::WorktreeError;
pub use manager::{Worktree, WorktreeManager};
