//! `cargo xtask version <patch|minor|major>`: bumps the workspace version in
//! every file that names it.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use toml_edit::{value, DocumentMut, Item};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Patch,
    Minor,
    Major,
}

#[derive(Debug)]
pub struct Options {
    pub level: Level,
    pub dry_run: bool,
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut level = None;
        let mut dry_run = false;
        for arg in args {
            match arg.as_str() {
                "patch" => level = Some(Level::Patch),
                "minor" => level = Some(Level::Minor),
                "major" => level = Some(Level::Major),
                "--dry-run" => dry_run = true,
                other => {
                    return Err(format!(
                        "unexpected argument `{other}`: version <patch|minor|major> [--dry-run]"
                    ))
                }
            }
        }
        let level = level.ok_or("which increment: version <patch|minor|major>")?;
        Ok(Self { level, dry_run })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    /// A plain `major.minor.patch`; pre-releases and build metadata are refused.
    pub fn parse(text: &str) -> Result<Self, String> {
        let parts: Vec<u64> = text
            .split('.')
            .map(|part| part.parse().ok())
            .collect::<Option<_>>()
            .unwrap_or_default();
        match parts[..] {
            [major, minor, patch] => Ok(Self {
                major,
                minor,
                patch,
            }),
            _ => Err(format!("`{text}` is not a plain major.minor.patch version")),
        }
    }

    pub fn bump(self, level: Level) -> Self {
        match level {
            Level::Patch => Self {
                patch: self.patch + 1,
                ..self
            },
            Level::Minor => Self {
                minor: self.minor + 1,
                patch: 0,
                ..self
            },
            Level::Major => Self {
                major: self.major + 1,
                minor: 0,
                patch: 0,
            },
        }
    }

    /// The requirement a user writes for this version (`hanafubuki-agent = "0.1"`):
    /// before 1.0 a minor release may break, after it only a major one.
    pub fn requirement(self) -> String {
        if self.major == 0 {
            format!("0.{}", self.minor)
        } else {
            self.major.to_string()
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A file and its new contents.
#[derive(Debug)]
pub struct Change {
    pub path: PathBuf,
    pub contents: String,
}

pub fn run(root: &Path, options: &Options) -> Result<(), String> {
    let (old, new, changes) = plan(root, options.level)?;
    println!("{old} -> {new}");
    for change in &changes {
        println!(
            "  {}",
            change
                .path
                .strip_prefix(root)
                .unwrap_or(&change.path)
                .display()
        );
    }
    if options.dry_run {
        println!("  Cargo.lock (cargo update --workspace)");
        return Ok(());
    }
    for change in &changes {
        std::fs::write(&change.path, &change.contents)
            .map_err(|error| format!("{}: {error}", change.path.display()))?;
    }
    update_lockfile(root)?;
    println!("  Cargo.lock");
    Ok(())
}

/// The workspace version, its bump, and the files that change.
pub fn plan(root: &Path, level: Level) -> Result<(Version, Version, Vec<Change>), String> {
    let manifest_path = root.join("Cargo.toml");
    let manifest = read(&manifest_path)?;
    let old = workspace_version(&manifest)?;
    let new = old.bump(level);
    let members = member_names(root, &manifest)?;
    let mut changes = vec![Change {
        path: manifest_path,
        contents: bump_manifest(&manifest, new, &members)?,
    }];
    if old.requirement() != new.requirement() {
        for path in snippet_files(root) {
            let text = read(&path)?;
            let updated = bump_snippets(&text, &old.requirement(), &new.requirement(), &members);
            if updated != text {
                changes.push(Change {
                    path,
                    contents: updated,
                });
            }
        }
    }
    Ok((old, new, changes))
}

pub fn workspace_version(manifest: &str) -> Result<Version, String> {
    let document = parse("Cargo.toml", manifest)?;
    let version = document
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("version"))
        .and_then(|version| version.as_str())
        .ok_or("Cargo.toml has no [workspace.package] version")?;
    Version::parse(version)
}

/// The root manifest with the new version: `[workspace.package] version` and
/// the `version` of every `[workspace.dependencies]` entry on a member.
pub fn bump_manifest(
    manifest: &str,
    new: Version,
    members: &BTreeSet<String>,
) -> Result<String, String> {
    let mut document = parse("Cargo.toml", manifest)?;
    let new = new.to_string();
    set_keeping_decor(&mut document["workspace"]["package"]["version"], &new);
    let dependencies = document
        .get_mut("workspace")
        .and_then(|workspace| workspace.get_mut("dependencies"))
        .and_then(Item::as_table_like_mut);
    if let Some(dependencies) = dependencies {
        for (name, entry) in dependencies.iter_mut() {
            let Some(table) = entry.as_table_like_mut() else {
                continue;
            };
            let package = table
                .get("package")
                .and_then(Item::as_str)
                .unwrap_or(name.get());
            if !members.contains(package) {
                continue;
            }
            if let Some(version) = table.get_mut("version") {
                set_keeping_decor(version, &new);
            }
        }
    }
    Ok(document.to_string())
}

/// Sets a string value, keeping the spaces and comment around the old one.
fn set_keeping_decor(item: &mut Item, new: &str) {
    let decor = item.as_value().map(|old| old.decor().clone());
    *item = value(new);
    if let (Some(decor), Some(value)) = (decor, item.as_value_mut()) {
        *value.decor_mut() = decor;
    }
}

/// `text` with every `<member> = "<old>"` dependency requirement set to `new`.
pub fn bump_snippets(text: &str, old: &str, new: &str, members: &BTreeSet<String>) -> String {
    let needle = format!(" = \"{old}\"");
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find(&needle) {
        let (before, after) = rest.split_at(index);
        let crate_name = before
            .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .next();
        result.push_str(before);
        if crate_name.is_some_and(|name| members.contains(name)) {
            result.push_str(&format!(" = \"{new}\""));
        } else {
            result.push_str(&needle);
        }
        rest = &after[needle.len()..];
    }
    result.push_str(rest);
    result
}

/// The package names of the workspace's members. A member is a directory, or
/// `<dir>/*` for every directory below `<dir>` that holds a Cargo.toml.
fn member_names(root: &Path, manifest: &str) -> Result<BTreeSet<String>, String> {
    let document = parse("Cargo.toml", manifest)?;
    let patterns = document
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(Item::as_array)
        .map(|members| {
            members
                .iter()
                .filter_map(|member| member.as_str())
                .collect()
        })
        .unwrap_or_else(Vec::new);
    let mut directories = Vec::new();
    for pattern in patterns {
        match pattern.strip_suffix("/*") {
            Some(parent) => {
                if let Ok(entries) = std::fs::read_dir(root.join(parent)) {
                    directories.extend(entries.flatten().map(|entry| entry.path()));
                }
            }
            None => directories.push(root.join(pattern)),
        }
    }
    let mut names = BTreeSet::new();
    for directory in directories {
        let path = directory.join("Cargo.toml");
        if !path.is_file() {
            continue;
        }
        let member = parse(&path.display().to_string(), &read(&path)?)?;
        if let Some(name) = member
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(Item::as_str)
        {
            names.insert(name.to_string());
        }
    }
    Ok(names)
}

/// Files that show users a dependency on a member: the README and the
/// Markdown pages under docs/.
fn snippet_files(root: &Path) -> Vec<PathBuf> {
    let mut files = vec![root.join("README.md")];
    if let Ok(entries) = std::fs::read_dir(root.join("docs")) {
        let mut pages: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
            .collect();
        pages.sort();
        files.extend(pages);
    }
    files.retain(|path| path.is_file());
    files
}

fn update_lockfile(root: &Path) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(root)
        .args(["update", "--workspace", "--offline", "--quiet"])
        .status()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("`cargo update --workspace` failed".into())
    }
}

fn parse(name: &str, manifest: &str) -> Result<DocumentMut, String> {
    manifest.parse().map_err(|error| format!("{name}: {error}"))
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests;
