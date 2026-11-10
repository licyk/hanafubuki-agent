// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugin-common/src/installed.rs` (the version directory's
// name and the active version among those installed) at commit
// `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the remote install metadata is not ported.

use std::cmp::Ordering;
use std::fs;
use std::path::Path;

/// The version directory of a package whose manifest names no version.
pub const LOCAL_VERSION: &str = "local";

pub fn validate_version_segment(version: &str) -> Result<(), String> {
    if version.is_empty() {
        return Err("invalid plugin version: must not be empty".to_string());
    }
    if matches!(version, "." | "..") {
        return Err("invalid plugin version: path traversal is not allowed".to_string());
    }
    if !version
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '+'))
    {
        return Err(
            "invalid plugin version: only ASCII letters, digits, `.`, `+`, `_`, and `-` are allowed"
                .to_string(),
        );
    }
    Ok(())
}

/// Semantic versions compare semantically; anything else as text.
pub fn compare_versions(left: &str, right: &str) -> Ordering {
    match (semver::Version::parse(left), semver::Version::parse(right)) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

/// The version of the package under `base` that is in use: `local` when it
/// is there, else the highest.
pub fn active_version(base: &Path) -> Option<String> {
    let mut versions = fs::read_dir(base)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|version| validate_version_segment(version).is_ok())
        .collect::<Vec<_>>();
    if versions.iter().any(|version| version == LOCAL_VERSION) {
        return Some(LOCAL_VERSION.to_string());
    }
    versions.sort_unstable_by(|left, right| compare_versions(left, right));
    versions.pop()
}
