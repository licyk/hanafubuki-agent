//! Finding Skills on disk.
//!
//! A root is walked for directories containing `SKILL.md`; each one is a Skill,
//! named after its frontmatter or its folder. The walk does not descend into a
//! Skill it has already found, so a package that ships examples under its own
//! tree does not register them as separate Skills.
//!
//! A directory that fails to load is reported as an error against that
//! directory and skipped, and so is a directory below the root that cannot be
//! read. A Skill that loads with something ignored or shortened reports that as
//! a warning against its file. One malformed package never costs the user the
//! rest of their catalog.

use std::path::{Path, PathBuf};

use hanafubuki_agent_foundation::paths::PathKey;

use crate::fs_util::{
    collect_files, native_text, read_bounded, relative_from, DiscoveredFile, MAX_DIRECTORY_DEPTH,
};
use crate::interface::{parse_interface, MAX_INTERFACE_BYTES};
use crate::model::{
    SkillAvailability, SkillCatalogEntry, SkillError, SkillId, SkillLoadIssue,
    SkillResourceDescriptor, SkillResourceKind, SkillScope,
};
use crate::parser::parse_skill;

pub(crate) const SKILL_FILE: &str = "SKILL.md";
/// A `SKILL.md` far larger than this is not instructions any more.
pub(crate) const MAX_SKILL_MD_BYTES: u64 = 1024 * 1024;

/// A loaded Skill and everything the catalog needs to serve it.
#[derive(Clone, Debug)]
pub(crate) struct LoadedSkill {
    pub(crate) entry: SkillCatalogEntry,
    pub(crate) directory: PathBuf,
    /// The complete `SKILL.md`, frontmatter included.
    pub(crate) document: String,
    /// What loading ignored or shortened; [`scan_root`] moves these into the
    /// catalog's issues.
    pub(crate) issues: Vec<SkillLoadIssue>,
}

#[derive(Debug, Default)]
pub(crate) struct RootScan {
    pub(crate) skills: Vec<LoadedSkill>,
    pub(crate) issues: Vec<SkillLoadIssue>,
}

/// Load every Skill under one root.
pub(crate) fn scan_root(root: &Path, scope: SkillScope) -> RootScan {
    let mut scan = RootScan::default();
    if !root.is_dir() {
        return scan;
    }
    let mut directories = Vec::new();
    if let Err(error) = collect_skill_directories(root, 0, &mut directories, &mut scan.issues) {
        scan.issues
            .push(SkillLoadIssue::error(native_text(root), error.to_string()));
        return scan;
    }
    for directory in directories {
        let relative = match relative_from(root, &directory) {
            Ok(relative) => relative,
            Err(error) => {
                scan.issues.push(SkillLoadIssue::error(
                    native_text(&directory),
                    error.to_string(),
                ));
                continue;
            }
        };
        match load_skill(&directory, scope, &relative) {
            Ok(mut skill) => {
                scan.issues.append(&mut skill.issues);
                scan.skills.push(skill);
            }
            Err(error) => scan.issues.push(SkillLoadIssue::error(
                native_text(&directory),
                error.to_string(),
            )),
        }
    }
    scan
}

/// Read one Skill directory. `relative` is its path under the scope root and
/// becomes the tail of its id; its last segment is the default name.
pub(crate) fn load_skill(
    directory: &Path,
    scope: SkillScope,
    relative: &str,
) -> Result<LoadedSkill, SkillError> {
    let id = SkillId::new(scope, relative)?;
    let files = collect_files(directory)?;
    let parsed = parse_skill(
        &read_bounded(&directory.join(SKILL_FILE), MAX_SKILL_MD_BYTES)?,
        id.folder_name(),
    )?;
    let interface_path = directory.join("agents").join("openai.yaml");
    let interface = interface_path
        .is_file()
        .then(|| read_bounded(&interface_path, MAX_INTERFACE_BYTES).ok())
        .flatten()
        .map(|bytes| parse_interface(&bytes))
        .unwrap_or_default();
    let skill_file = native_text(&directory.join(SKILL_FILE));
    let interface_file = native_text(&interface_path);
    let issues = parsed
        .issues
        .into_iter()
        .map(|message| (&skill_file, message))
        .chain(
            interface
                .issues
                .into_iter()
                .map(|message| (&interface_file, message)),
        )
        .map(|(path, message)| SkillLoadIssue::warning(path.clone(), message))
        .collect();

    Ok(LoadedSkill {
        entry: SkillCatalogEntry {
            scope,
            name: parsed.name,
            description: parsed.description,
            short_description: parsed.short_description,
            license: parsed.license,
            version: parsed.version,
            allow_implicit_invocation: interface.allow_implicit_invocation,
            interface: interface.interface,
            dependencies: interface.dependencies,
            directory: native_text(directory),
            editable: scope == SkillScope::User,
            enabled: true,
            availability: SkillAvailability::default(),
            resources: resources(&files),
            id,
        },
        directory: directory.to_path_buf(),
        document: parsed.document,
        issues,
    })
}

/// Every file except `SKILL.md` itself, classified by where it sits.
fn resources(files: &[DiscoveredFile]) -> Vec<SkillResourceDescriptor> {
    files
        .iter()
        .filter(|file| file.relative_path != SKILL_FILE)
        .map(|file| SkillResourceDescriptor {
            kind: resource_kind(&file.relative_path),
            path: file.relative_path.clone(),
            size: file.size,
        })
        .collect()
}

pub(crate) fn resource_kind(path: &str) -> SkillResourceKind {
    if path.starts_with("scripts/") {
        SkillResourceKind::Script
    } else if path.starts_with("references/") || path.to_ascii_lowercase().ends_with(".md") {
        SkillResourceKind::Reference
    } else {
        SkillResourceKind::Asset
    }
}

/// Directories under `root` that hold a `SKILL.md`, nearest first. A Skill's
/// own subtree is not searched again. The root that cannot be read fails the
/// walk; a directory below it that cannot be read is an issue and is skipped.
fn collect_skill_directories(
    directory: &Path,
    depth: usize,
    output: &mut Vec<PathBuf>,
    issues: &mut Vec<SkillLoadIssue>,
) -> Result<(), SkillError> {
    if depth > MAX_DIRECTORY_DEPTH {
        return Ok(());
    }
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if depth > 0 => {
            let path = native_text(directory);
            log::warn!("skipping an unreadable directory under a Skill root path={path}: {error}");
            issues.push(SkillLoadIssue::error(
                path,
                format!("skipped a directory that could not be read: {error}"),
            ));
            return Ok(());
        }
        Err(error) => {
            return Err(SkillError::Io(format!(
                "read {}: {error}",
                native_text(directory)
            )))
        }
    };
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        // A symlinked Skill directory is followed on purpose: a checkout linked
        // into the user root is a normal way to develop one. Symlinks *inside*
        // a Skill are still skipped when its files are collected.
        if !path.is_dir() {
            continue;
        }
        let _ = metadata;
        if path.join(SKILL_FILE).is_file() {
            output.push(path);
            continue;
        }
        collect_skill_directories(&path, depth.saturating_add(1), output, issues)?;
    }
    Ok(())
}

/// The repository Skill roots of a workspace: `.agents/skills` in each
/// workspace root and in every directory above it up to the root of its Git
/// repository (none above it outside one), nearest first and each once.
pub fn repository_skill_roots(workspace_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for root in workspace_roots {
        let boundary = root
            .ancestors()
            .find(|directory| directory.join(".git").exists());
        for directory in root.ancestors() {
            let candidate = directory.join(".agents").join("skills");
            // Workspace roots can share ancestors, so the same repository
            // root is reachable twice.
            let key = PathKey::of(&candidate);
            if !roots.iter().any(|existing| PathKey::of(existing) == key) {
                roots.push(candidate);
            }
            if boundary.is_none_or(|boundary| PathKey::of(directory) == PathKey::of(boundary)) {
                break;
            }
        }
    }
    roots
}

#[cfg(test)]
mod tests;
