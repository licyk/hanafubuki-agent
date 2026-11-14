//! Immutable workspace captured when a session admits a turn.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnWorkspace {
    pub roots: Vec<PathBuf>,
}
