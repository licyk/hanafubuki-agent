use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::ResolvedPath;
use serde::{Deserialize, Serialize};

use crate::McpError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResolvedProgram {
    pub configured_program: String,
    pub resolved_path: PathBuf,
    pub searched_path_entries: usize,
}

pub fn validate_stdio_program_shape(program: &str) -> Result<(), McpError> {
    if program.is_empty() || program.len() > crate::MAX_MCP_FIELD_BYTES || program.contains('\0') {
        return Err(McpError::InvalidConfig("stdio program is invalid".into()));
    }
    let path = Path::new(program);
    if path.is_absolute() {
        return Ok(());
    }
    if path.components().count() != 1
        || program.contains('/')
        || program.contains('\\')
        || !program
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'+'))
        || matches!(program, "." | "..")
    {
        return Err(McpError::InvalidConfig(
            "stdio program must be an absolute executable path or a PATH command name".into(),
        ));
    }
    Ok(())
}

pub fn resolve_stdio_program(
    program: &str,
    path_value: Option<&OsStr>,
) -> Result<McpResolvedProgram, McpError> {
    validate_stdio_program_shape(program)?;
    let path = Path::new(program);
    if path.is_absolute() {
        return resolved(program, path, 0);
    }
    let path_value = path_value.ok_or_else(|| {
        McpError::ResolveProgram("PATH is unavailable in the server launch environment".into())
    })?;
    let entries = std::env::split_paths(path_value).collect::<Vec<_>>();
    let extensions = platform_executable_extensions();
    for directory in &entries {
        for extension in &extensions {
            let mut candidate = directory.join(program);
            if !extension.is_empty() {
                candidate.set_extension(extension);
            }
            if let Ok(resolved) = resolved(program, &candidate, entries.len()) {
                return Ok(resolved);
            }
        }
    }
    Err(McpError::ResolveProgram(format!(
        "command {program:?} was not found in {} PATH entries",
        entries.len()
    )))
}

fn resolved(
    configured_program: &str,
    candidate: &Path,
    searched_path_entries: usize,
) -> Result<McpResolvedProgram, McpError> {
    let metadata = std::fs::metadata(candidate).map_err(|_| {
        McpError::ResolveProgram(format!("program {configured_program:?} is not available"))
    })?;
    if !metadata.is_file() {
        return Err(McpError::ResolveProgram(format!(
            "program {configured_program:?} is not a regular file"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(McpError::ResolveProgram(format!(
                "program {configured_program:?} is not executable"
            )));
        }
    }
    // The native form: this becomes `argv[0]` for the stdio server and appears
    // in diagnostics, and neither reliably accepts a verbatim path.
    let resolved_path = ResolvedPath::resolve(candidate)
        .map_err(|error| {
            McpError::ResolveProgram(format!("resolve program {configured_program:?}: {error}"))
        })?
        .native()
        .to_path_buf();
    Ok(McpResolvedProgram {
        configured_program: configured_program.to_string(),
        resolved_path,
        searched_path_entries,
    })
}

fn platform_executable_extensions() -> Vec<OsString> {
    #[cfg(windows)]
    {
        let mut values = vec![OsString::new()];
        values.extend(
            std::env::var_os("PATHEXT")
                .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into())
                .to_string_lossy()
                .split(';')
                .filter(|value| !value.is_empty())
                .map(|value| OsString::from(value.trim_start_matches('.').to_ascii_lowercase())),
        );
        values
    }
    #[cfg(not(windows))]
    {
        vec![OsString::new()]
    }
}

#[cfg(test)]
mod tests;
