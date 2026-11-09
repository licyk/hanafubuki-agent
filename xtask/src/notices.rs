//! Keeps the attribution of third-party code where its licence asks for it
//! (NOTICE; site/content/docs/contributing.en.mdx, "Licence").
//!
//! Apache-2.0 asks every modified file to say that it was changed and the
//! NOTICE to carry the original's notices; MIT asks for its header to stay
//! on the files derived from it. Nothing else catches a ported file that
//! lost its line or a new file under the port that never got one, so the
//! derived sources are listed in `architecture/notices-policy.toml` and each
//! is checked: the marker in its leading comment block, the MIT header where
//! one is kept, and NOTICE naming it.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::source_layout::relative;

const POLICY: &str = include_str!("../../architecture/notices-policy.toml");
const MIT_TITLE: &str = "The MIT License (MIT)";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoticesPolicy {
    version: u32,
    #[serde(default)]
    derived: Vec<Derived>,
    #[serde(default)]
    mit: Vec<Mit>,
    #[serde(default)]
    original: Vec<Original>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Derived {
    /// A repository-relative file or directory, with `/` separators.
    path: String,
    /// Each file carries one of them: one per upstream revision ported.
    markers: Vec<String>,
    /// What NOTICE names for it.
    notice: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mit {
    path: String,
    holder: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Original {
    path: String,
    reason: String,
}

pub fn run(root: &Path) -> Result<(), String> {
    let policy = parse(POLICY)?;
    let notice =
        fs::read_to_string(root.join("NOTICE")).map_err(|error| format!("read NOTICE: {error}"))?;
    let violations = check(root, &policy, &notice);
    if violations.is_empty() {
        println!("Notices check passed.");
        return Ok(());
    }
    Err(format!(
        "third-party notices:\n\n{}\n\n\
         A file derived from another project says so in its first comment\n\
         block and NOTICE names it; a file that is not derived is listed as\n\
         original in architecture/notices-policy.toml with why.\n",
        violations.join("\n")
    ))
}

pub(crate) fn parse(text: &str) -> Result<NoticesPolicy, String> {
    let policy: NoticesPolicy =
        toml::from_str(text).map_err(|error| format!("parse notices-policy.toml: {error}"))?;
    if policy.version != 1 {
        return Err(format!(
            "notices-policy.toml version {} is not 1",
            policy.version
        ));
    }
    Ok(policy)
}

/// Every way the sources under `root` fall short of `policy`, given the
/// text of NOTICE.
pub(crate) fn check(root: &Path, policy: &NoticesPolicy, notice: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let original = original_paths(root, policy, &mut violations);
    for derived in &policy.derived {
        if !notice.contains(&format!("`{}`", derived.notice)) {
            violations.push(format!(
                "NOTICE does not name `{}` ({})",
                derived.notice, derived.path
            ));
        }
        let files = rust_files(&root.join(&derived.path));
        if files.is_empty() {
            violations.push(format!("{}: no Rust file there", derived.path));
        }
        for file in files {
            let path = relative(root, &file);
            if original.contains(&path) {
                continue;
            }
            match fs::read_to_string(&file) {
                Ok(source) if has_marker(&header(&source), &derived.markers) => {}
                Ok(_) => violations.push(format!(
                    "{path}: its leading comments do not say \"{}\"",
                    derived.markers.join("\" or \"")
                )),
                Err(error) => violations.push(format!("{path}: {error}")),
            }
        }
    }
    for mit in &policy.mit {
        if !notice.contains(&format!("`{}`", mit.path)) {
            violations.push(format!("NOTICE does not name `{}`", mit.path));
        }
        match fs::read_to_string(root.join(&mit.path)) {
            Ok(source) => {
                let header = header(&source);
                if !header.contains(MIT_TITLE) || !header.contains(&mit.holder) {
                    violations.push(format!("{}: the MIT header is gone", mit.path));
                }
            }
            Err(error) => violations.push(format!("{}: {error}", mit.path)),
        }
    }
    violations
}

/// The files listed as original; an entry for a file that is gone, or one
/// without a reason, is a violation.
fn original_paths(
    root: &Path,
    policy: &NoticesPolicy,
    violations: &mut Vec<String>,
) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for original in &policy.original {
        if original.reason.trim().is_empty() {
            violations.push(format!(
                "{}: an original file states why it is not derived",
                original.path
            ));
        }
        if !root.join(&original.path).is_file() {
            violations.push(format!(
                "{}: listed as original but no longer exists; remove the entry",
                original.path
            ));
        }
        paths.insert(original.path.clone());
    }
    paths
}

fn has_marker(header: &str, markers: &[String]) -> bool {
    markers
        .iter()
        .any(|marker| header.contains(marker.as_str()))
}

/// The comment lines a file starts with, blank lines among them included.
pub(crate) fn header(source: &str) -> String {
    source
        .lines()
        .take_while(|line| {
            let line = line.trim_start();
            line.is_empty() || line.starts_with("//")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn rust_files(path: &Path) -> Vec<PathBuf> {
    if path.is_file() {
        return vec![path.to_path_buf()];
    }
    let mut files = Vec::new();
    let mut pending = vec![path.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests;
