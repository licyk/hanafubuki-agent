//! Keeps path resolution behind its single authority,
//! `hanafubuki-agent-foundation::paths` (AGENTS.md, "Paths").
//!
//! New code kept regressing to a raw `fs::canonicalize` because nothing made
//! one visible. This check is what makes it visible.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

const PATH_POLICY: &str = include_str!("../../architecture/path-policy.toml");

/// Ways a path becomes text.
const RENDERINGS: &[&str] = &[
    ".display()",
    ".to_string_lossy()",
    "format!",
    "write!",
    "log::",
];

#[derive(Debug, Deserialize)]
struct PathPolicy {
    #[serde(default)]
    allow: Vec<AllowEntry>,
}

#[derive(Debug, Deserialize)]
struct AllowEntry {
    path: String,
    reason: String,
}

fn policy(root: &Path) -> Result<BTreeSet<String>, String> {
    let parsed: PathPolicy =
        toml::from_str(PATH_POLICY).map_err(|error| format!("parse path-policy.toml: {error}"))?;
    let mut allowed = BTreeSet::new();
    for entry in parsed.allow {
        if entry.reason.trim().is_empty() {
            return Err(format!(
                "path-policy.toml entry {} must state why the raw call is correct",
                entry.path
            ));
        }
        if !root.join(&entry.path).exists() {
            return Err(format!(
                "path-policy.toml names a file that no longer exists: {}\n\
                 Remove the entry rather than leaving a stale exemption.",
                entry.path
            ));
        }
        allowed.insert(entry.path);
    }
    Ok(allowed)
}

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if name != "target" && name != "gen" && name != "generated" {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Lines that are comments or doc comments. The rules are about code, and the
/// documentation explaining them naturally names what it forbids.
fn is_prose(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("//") || trimmed.starts_with("*") || trimmed.starts_with("#[doc")
}

pub fn run(root: &Path) -> Result<(), String> {
    let allowed = policy(root)?;
    let mut violations = Vec::new();

    for file in rust_sources(&root.join("crates")) {
        let relative = relative(root, &file);
        let Ok(contents) = fs::read_to_string(&file) else {
            continue;
        };
        let exempt = allowed.contains(&relative);

        for (index, line) in contents.lines().enumerate() {
            let number = index + 1;
            if is_prose(line) {
                continue;
            }
            if !exempt && line.contains("canonicalize(") {
                violations.push(format!(
                    "{relative}:{number}: `canonicalize` outside hanafubuki-agent-foundation::paths.\n\
                     Resolve through `ResolvedPath::resolve` and choose a projection:\n\
                     system() for OS calls, native() for anything read or recorded,\n\
                     key() for comparison. Add an audited entry to\n\
                     architecture/path-policy.toml only if this really is a\n\
                     platform boundary."
                ));
            }
            // Now that `canonicalize` is confined to the authority, `system()`
            // is the only value that still carries a verbatim prefix. Rendering
            // it is therefore the one remaining way to leak `\\?\` outward.
            if line.contains(".system()")
                && RENDERINGS.iter().any(|rendering| line.contains(rendering))
            {
                violations.push(format!(
                    "{relative}:{number}: the system form is being rendered. It carries the\n\
                     Windows verbatim prefix and exists only for operating system calls.\n\
                     Use `native_text()` for anything a person, a model, a log line, or a\n\
                     durable record will read."
                ));
            }
        }
    }

    if violations.is_empty() {
        println!("Path resolution boundary check passed.");
        return Ok(());
    }
    Err(format!(
        "path resolution boundary violations:\n\n{}\n",
        violations.join("\n\n")
    ))
}
