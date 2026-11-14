//! One atomic `rename` over the real path: a symlink pointing at it stays a
//! symlink, and the new file's own inode keeps macOS's code-signature cache
//! from judging it by the old one.

use std::fs;
use std::path::Path;

use hanafubuki_agent_foundation::paths::ResolvedPath;

use crate::error::UpdateError;

pub(super) fn replace(executable: &ResolvedPath, staged: &Path) -> Result<(), UpdateError> {
    fs::rename(staged, executable.system()).map_err(|error| UpdateError::Replace {
        executable: executable.native_text().to_string(),
        reason: error.to_string(),
    })
}
