// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/marketplace_add/source.rs` (what `marketplace
// add` accepts: a local directory, a git URL with a `#ref`, a GitHub
// `owner/repo[@ref]`) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: a local directory resolves through the
// foundation's paths (its native form is recorded); a `file://` URL is a
// git source, so a repository on disk can be added as one.

use std::path::Path;

use hanafubuki_agent_foundation::paths::ResolvedPath;

use super::entry::{github_shorthand, github_url};
use crate::PackageError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AddSource {
    Git {
        url: String,
        ref_name: Option<String>,
    },
    Local {
        path: ResolvedPath,
    },
}

pub(crate) fn parse_add_source(
    source: &str,
    explicit_ref: Option<String>,
) -> Result<AddSource, PackageError> {
    let invalid = |message: &str| Err(PackageError::Invalid(message.to_string()));
    let source = source.trim();
    if source.is_empty() {
        return invalid("marketplace source must not be empty");
    }
    let (base, parsed_ref) = split_ref(source);
    let ref_name = explicit_ref.or(parsed_ref);
    if looks_like_local_path(&base) {
        if ref_name.is_some() {
            return invalid("--ref is only supported for git marketplace sources");
        }
        let path = ResolvedPath::resolve(expand_home(&base)).map_err(|error| {
            PackageError::Invalid(format!(
                "failed to resolve local marketplace source path: {error}"
            ))
        })?;
        if !path.system().is_dir() {
            return invalid("local marketplace source must be a directory, not a file");
        }
        return Ok(AddSource::Local { path });
    }
    if is_ssh_url(&base) || base.starts_with("file://") {
        return Ok(AddSource::Git {
            url: base.trim_end_matches('/').to_string(),
            ref_name,
        });
    }
    if base.starts_with("http://") || base.starts_with("https://") {
        return Ok(AddSource::Git {
            url: github_url(&base),
            ref_name,
        });
    }
    match github_shorthand(&base) {
        Some(url) => Ok(AddSource::Git { url, ref_name }),
        None => invalid(
            "invalid marketplace source format; expected owner/repo, a git URL, or a local marketplace path",
        ),
    }
}

/// `url#ref`, or `owner/repo@ref` for what is neither a path nor a URL.
fn split_ref(source: &str) -> (String, Option<String>) {
    let non_empty = |name: &str| Some(name.trim().to_string()).filter(|name| !name.is_empty());
    if let Some((base, ref_name)) = source.rsplit_once('#') {
        return (base.to_string(), non_empty(ref_name));
    }
    if !looks_like_local_path(source) && !source.contains("://") && !is_ssh_url(source) {
        if let Some((base, ref_name)) = source.rsplit_once('@') {
            return (base.to_string(), non_empty(ref_name));
        }
    }
    (source.to_string(), None)
}

fn is_ssh_url(source: &str) -> bool {
    source.starts_with("ssh://") || (source.starts_with("git@") && source.contains(':'))
}

pub(crate) fn looks_like_local_path(source: &str) -> bool {
    let bytes = source.as_bytes();
    let windows_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    Path::new(source).is_absolute()
        || windows_absolute
        || source.starts_with(r"\\")
        || ["./", ".\\", "../", "..\\", "~/"]
            .iter()
            .any(|prefix| source.starts_with(prefix))
        || source == "."
        || source == ".."
}

fn expand_home(source: &str) -> std::path::PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    match (source.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => Path::new(&home).join(rest),
        _ => source.into(),
    }
}

#[cfg(test)]
mod tests;
