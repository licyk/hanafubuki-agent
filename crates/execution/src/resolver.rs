use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use hanafubuki_agent_approval::ApprovalOrigin;
use hanafubuki_agent_foundation::paths::ResolvedPath;

use crate::model::AgentExecutionError;

pub(crate) const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 10 * 60 * 1_000;
pub(crate) const DEFAULT_COMMAND_OUTPUT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_OUTPUT_READ_BYTES: usize = 256 * 1024;
pub(crate) const MAX_WAIT_MS: u64 = 30_000;

pub(crate) fn validate_execution_origin(
    origin: &ApprovalOrigin,
    actual_source_digest: Option<&str>,
) -> Result<(), AgentExecutionError> {
    let ApprovalOrigin::Skill {
        skill,
        content_digest,
    } = origin
    else {
        return Ok(());
    };
    let valid_digest = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    if skill.is_empty()
        || skill.len() > 200
        || !valid_digest(content_digest)
        || actual_source_digest != Some(content_digest.as_str())
    {
        return Err(AgentExecutionError::InvalidRequest(
            "Skill execution origin must match the exact verified script bytes".to_string(),
        ));
    }
    Ok(())
}

/// Resolve a working directory to its interchange form.
///
/// The native spelling is what belongs in a plan: it is logged, shown on the
/// approval card, returned to the model, and joined into command arguments. The
/// verbatim form is re-applied by `hanafubuki-agent-process` at the spawn boundary, which is
/// the only place a Win32 call actually happens.
pub(crate) fn canonical_directory(path: &Path) -> Result<PathBuf, AgentExecutionError> {
    let resolved = ResolvedPath::resolve(path)
        .map_err(|error| AgentExecutionError::Resolution(format!("working directory: {error}")))?;
    if !resolved.system().is_dir() {
        return Err(AgentExecutionError::Resolution(
            "working directory is not a directory".to_string(),
        ));
    }
    Ok(resolved.native().to_path_buf())
}

pub(crate) fn resolve_executable(
    executable: &str,
    cwd: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<PathBuf, AgentExecutionError> {
    let requested = Path::new(executable);
    if requested.components().count() > 1 || requested.is_absolute() {
        let candidate = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            cwd.join(requested)
        };
        return canonical_executable(&candidate);
    }
    let path = environment_value(environment, "PATH").unwrap_or_default();
    for directory in std::env::split_paths(path) {
        for candidate in executable_candidates(&directory.join(requested), environment) {
            if let Ok(resolved) = canonical_executable(&candidate) {
                return Ok(resolved);
            }
        }
    }
    Err(AgentExecutionError::Resolution(format!(
        "executable was not found in the clean PATH: {executable}"
    )))
}

/// Resolve a program to its interchange form.
///
/// `argv[0]` is both a Win32 argument and a string the model reads back, so it
/// must be the native spelling; a verbatim `argv[0]` is not reliably accepted by
/// the programs being invoked.
fn canonical_executable(path: &Path) -> Result<PathBuf, AgentExecutionError> {
    let resolved = ResolvedPath::resolve(path)
        .map_err(|error| AgentExecutionError::Resolution(format!("executable: {error}")))?;
    let canonical = resolved.native().to_path_buf();
    let metadata = fs::metadata(resolved.system())
        .map_err(|error| AgentExecutionError::Resolution(format!("executable: {error}")))?;
    if !metadata.is_file() {
        return Err(AgentExecutionError::Resolution(
            "resolved executable is not a file".to_string(),
        ));
    }
    #[cfg(windows)]
    if canonical.extension().is_some_and(|extension| {
        let extension = extension.to_string_lossy();
        extension.eq_ignore_ascii_case("bat") || extension.eq_ignore_ascii_case("cmd")
    }) {
        return Err(AgentExecutionError::Resolution(
            "batch files require the separately approved typed Shell tool".to_string(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(AgentExecutionError::Resolution(
                "resolved executable is not executable".to_string(),
            ));
        }
    }
    utf8_path(&canonical, "resolved executable")?;
    Ok(canonical)
}

#[cfg(windows)]
fn executable_candidates(base: &Path, environment: &BTreeMap<String, String>) -> Vec<PathBuf> {
    if base.extension().is_some() {
        return vec![base.to_path_buf()];
    }
    let extensions = environment_value(environment, "PATHEXT")
        .unwrap_or(".COM;.EXE")
        .split(';')
        .filter(|extension| {
            let extension = extension.trim_start_matches('.');
            extension.eq_ignore_ascii_case("COM") || extension.eq_ignore_ascii_case("EXE")
        })
        .map(|extension| base.with_extension(extension.trim_start_matches('.')))
        .collect::<Vec<_>>();
    if extensions.is_empty() {
        vec![base.to_path_buf()]
    } else {
        extensions
    }
}

#[cfg(not(windows))]
fn executable_candidates(base: &Path, _environment: &BTreeMap<String, String>) -> Vec<PathBuf> {
    vec![base.to_path_buf()]
}

fn environment_value<'a>(environment: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    environment
        .iter()
        .find(|(key, _)| {
            if cfg!(windows) {
                key.eq_ignore_ascii_case(name)
            } else {
                key.as_str() == name
            }
        })
        .map(|(_, value)| value.as_str())
}

pub(crate) fn utf8_path<'a>(path: &'a Path, label: &str) -> Result<&'a str, AgentExecutionError> {
    path.to_str()
        .ok_or_else(|| AgentExecutionError::Resolution(format!("{label} is not valid UTF-8")))
}
