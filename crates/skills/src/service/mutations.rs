//! Turning Skills on and off, and everything that writes to the managed root.
//!
//! The managed user root is the only place this service writes. A Skill's scope
//! decides what may be done to it: a System or Repo Skill can be turned off but
//! never edited, and a User Skill is the user's to change or remove. That is the
//! whole authority model — there is no token to present and no receipt to match.

use std::fs;
use std::path::{Path, PathBuf};

use crate::discovery::{load_skill, MAX_SKILL_MD_BYTES, SKILL_FILE};
use crate::fs_util::{
    copy_tree, ensure_below, ensure_directory, native_text, next_temporary_token,
    normalize_relative, read_bounded, replace_directory, sync_directory, write_new_file,
};
use crate::model::{
    SkillCatalogEntry, SkillCatalogSnapshot, SkillCreateRequest, SkillDeleteFileRequest,
    SkillDuplicateRequest, SkillError, SkillId, SkillInstallRequest, SkillScope,
    SkillSourceInspection, SkillWriteFileRequest, MAX_SKILL_FILE_BYTES,
};
use crate::parser::slug_from_name;
use crate::state::{bump_revision, load_state, save_state};

use super::SkillService;

const MAX_DESCRIPTION_CHARS: usize = 1024;

impl SkillService {
    pub fn enable(&self, id: &SkillId) -> Result<SkillCatalogSnapshot, SkillError> {
        self.set_enabled(id, true)
    }

    pub fn disable(&self, id: &SkillId) -> Result<SkillCatalogSnapshot, SkillError> {
        self.set_enabled(id, false)
    }

    fn set_enabled(&self, id: &SkillId, enabled: bool) -> Result<SkillCatalogSnapshot, SkillError> {
        let _mutation = self.inner.mutation.lock();
        let current = self.reload()?;
        if current.entry(id).is_none() {
            return Err(SkillError::NotFound(id.to_string()));
        }
        let mut state = load_state(&self.inner.roots.state_file)?;
        if state.set_disabled(id, !enabled) {
            bump_revision(&mut state);
            save_state(&self.inner.roots.state_file, &state)?;
            log::info!(
                "skill {} id={id}",
                if enabled { "enabled" } else { "disabled" }
            );
        }
        self.reload()
    }

    /// What a candidate directory would install as, without installing it.
    pub fn inspect_source(&self, source: &Path) -> Result<SkillSourceInspection, SkillError> {
        let directory = skill_directory_of(source)?;
        let folder_name = source_folder_name(&directory)?;
        let skill = load_skill(&directory, SkillScope::User, &folder_name)?;
        Ok(SkillSourceInspection {
            name: skill.entry.name,
            description: skill.entry.description,
            conflicts: self.inner.roots.user.join(&folder_name).exists(),
            resources: skill.entry.resources,
            folder_name,
        })
    }

    /// Copy a Skill directory into the managed root.
    ///
    /// The source may be the Skill directory itself or a wrapper holding
    /// exactly one — a downloaded archive or a cloned repository usually is
    /// one, and making the user find the inner folder is a step with no purpose.
    pub fn install(&self, request: SkillInstallRequest) -> Result<SkillCatalogEntry, SkillError> {
        let _mutation = self.inner.mutation.lock();
        let directory = skill_directory_of(&request.source_directory)?;
        let folder_name = source_folder_name(&directory)?;
        // Load it before anything is written: an unreadable source must fail
        // here rather than half-way through a copy.
        load_skill(&directory, SkillScope::User, &folder_name)?;
        let destination = self.managed_path(&folder_name)?;
        if destination.exists() && !request.overwrite {
            return Err(SkillError::AlreadyExists(folder_name));
        }
        self.publish(&folder_name, |staging| copy_tree(&directory, staging))?;
        log::info!(
            "skill installed folder={folder_name} source={}",
            native_text(&directory)
        );
        self.managed_entry(&folder_name)
    }

    /// Author a new Skill in the managed root from a name and a description.
    pub fn create(&self, request: SkillCreateRequest) -> Result<SkillCatalogEntry, SkillError> {
        let _mutation = self.inner.mutation.lock();
        let name = validated_name(&request.name)?;
        let description = validated_description(&request.description)?;
        let folder_name = folder_name_for(&name)?;
        if self.managed_path(&folder_name)?.exists() {
            return Err(SkillError::AlreadyExists(folder_name));
        }
        let document = new_skill_document(&name, &description);
        self.publish(&folder_name, |staging| {
            write_new_file(&staging.join(SKILL_FILE), document.as_bytes())
        })?;
        log::info!("skill created folder={folder_name}");
        self.managed_entry(&folder_name)
    }

    /// Copy an existing Skill of any scope into the managed root under a new
    /// name, so a bundled Skill can be used as the starting point for one of
    /// the user's own.
    pub fn duplicate(
        &self,
        request: SkillDuplicateRequest,
    ) -> Result<SkillCatalogEntry, SkillError> {
        let source = self.loaded(&request.source)?;
        let _mutation = self.inner.mutation.lock();
        let name = validated_name(&request.name)?;
        let folder_name = folder_name_for(&name)?;
        if self.managed_path(&folder_name)?.exists() {
            return Err(SkillError::AlreadyExists(folder_name));
        }
        let document = rename_document(&source.document, &name);
        self.publish(&folder_name, |staging| {
            copy_tree(&source.directory, staging)?;
            let skill_file = staging.join(SKILL_FILE);
            let _ = fs::remove_file(&skill_file);
            write_new_file(&skill_file, document.as_bytes())
        })?;
        log::info!(
            "skill duplicated source={} folder={folder_name}",
            request.source
        );
        self.managed_entry(&folder_name)
    }

    /// Write one file inside a managed Skill, creating it if it is new.
    pub fn write_file(
        &self,
        request: SkillWriteFileRequest,
    ) -> Result<SkillCatalogEntry, SkillError> {
        let directory = self.editable_directory(&request.id)?;
        let _mutation = self.inner.mutation.lock();
        let path = normalize_relative(&request.path)?;
        if request.content.len() as u64 > MAX_SKILL_FILE_BYTES {
            return Err(SkillError::LimitExceeded(format!(
                "a Skill file supports at most {MAX_SKILL_FILE_BYTES} bytes"
            )));
        }
        let target = directory.join(&path);
        ensure_below(&directory, &target)?;
        if let Some(parent) = target.parent() {
            ensure_directory(parent, "the Skill directory")?;
        }
        crate::fs_util::atomic_write(&target, &request.content, "the Skill file")?;
        self.invalidate();
        self.managed_entry(request.id.relative_path())
    }

    /// Remove one file from a managed Skill. `SKILL.md` is what makes the
    /// directory a Skill at all, so deleting it means deleting the Skill.
    pub fn delete_file(
        &self,
        request: SkillDeleteFileRequest,
    ) -> Result<SkillCatalogEntry, SkillError> {
        let directory = self.editable_directory(&request.id)?;
        let _mutation = self.inner.mutation.lock();
        let path = normalize_relative(&request.path)?;
        if path == SKILL_FILE {
            return Err(SkillError::InvalidPath(
                "SKILL.md cannot be removed; delete the Skill instead".to_string(),
            ));
        }
        let target = directory.join(&path);
        ensure_below(&directory, &target)?;
        let metadata = fs::symlink_metadata(&target)
            .map_err(|_| SkillError::ResourceNotFound(path.clone()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(SkillError::ResourceNotFound(path));
        }
        fs::remove_file(&target)
            .map_err(|error| SkillError::Io(format!("delete the Skill file: {error}")))?;
        self.invalidate();
        self.managed_entry(request.id.relative_path())
    }

    /// Delete a managed Skill outright.
    pub fn delete(&self, id: &SkillId) -> Result<SkillCatalogSnapshot, SkillError> {
        let directory = self.editable_directory(id)?;
        let _mutation = self.inner.mutation.lock();
        fs::remove_dir_all(&directory)
            .map_err(|error| SkillError::Io(format!("delete the Skill: {error}")))?;
        sync_directory(&self.inner.roots.user)?;
        let mut state = load_state(&self.inner.roots.state_file)?;
        if state.set_disabled(id, false) {
            bump_revision(&mut state);
            save_state(&self.inner.roots.state_file, &state)?;
        }
        log::info!("skill deleted id={id}");
        self.reload()
    }

    /// The directory of a Skill this service is allowed to change.
    fn editable_directory(&self, id: &SkillId) -> Result<PathBuf, SkillError> {
        let skill = self.loaded(id)?;
        if !skill.entry.editable {
            return Err(SkillError::ReadOnly(id.to_string()));
        }
        let directory = skill.directory;
        ensure_below(&self.inner.roots.user, &directory)?;
        Ok(directory)
    }

    fn managed_path(&self, relative: &str) -> Result<PathBuf, SkillError> {
        let path = self.inner.roots.user.join(normalize_relative(relative)?);
        ensure_below(&self.inner.roots.user, &path)?;
        Ok(path)
    }

    /// Build the Skill in a sibling staging directory and move it into place
    /// only once it is complete, so a failed write never leaves a half-formed
    /// Skill in the catalog.
    fn publish(
        &self,
        folder_name: &str,
        build: impl FnOnce(&Path) -> Result<(), SkillError>,
    ) -> Result<(), SkillError> {
        ensure_directory(&self.inner.roots.user, "the managed Skill root")?;
        let destination = self.managed_path(folder_name)?;
        let staging = self.inner.roots.user.join(format!(
            ".{folder_name}.staging.{}.{}",
            std::process::id(),
            next_temporary_token()
        ));
        if staging.exists() {
            let _ = fs::remove_dir_all(&staging);
        }
        ensure_directory(&staging, "the Skill staging directory")?;
        let result = build(&staging)
            .and_then(|()| {
                if staging.join(SKILL_FILE).is_file() {
                    Ok(())
                } else {
                    Err(SkillError::InvalidManifest(
                        "the source directory has no SKILL.md".to_string(),
                    ))
                }
            })
            .and_then(|()| replace_directory(&staging, &destination));
        if staging.exists() {
            let _ = fs::remove_dir_all(&staging);
        }
        result?;
        self.invalidate();
        Ok(())
    }

    fn invalidate(&self) {
        self.inner.catalog.write().fingerprint = None;
    }

    /// The catalog entry for a managed Skill, after the write that produced it.
    fn managed_entry(&self, relative: &str) -> Result<SkillCatalogEntry, SkillError> {
        let id = SkillId::new(SkillScope::User, relative)?;
        self.reload()?
            .entry(&id)
            .cloned()
            .ok_or_else(|| SkillError::NotFound(id.to_string()))
    }
}

/// The Skill directory inside a source the user pointed at: the directory
/// itself if it holds `SKILL.md`, otherwise its single Skill-bearing child.
fn skill_directory_of(source: &Path) -> Result<PathBuf, SkillError> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| SkillError::Io(format!("inspect {}: {error}", native_text(source))))?;
    if !metadata.is_dir() && !metadata.file_type().is_symlink() {
        return Err(SkillError::InvalidPath(format!(
            "{} is not a directory",
            native_text(source)
        )));
    }
    if source.join(SKILL_FILE).is_file() {
        return Ok(source.to_path_buf());
    }
    let mut candidates = fs::read_dir(source)
        .map_err(|error| SkillError::Io(format!("read {}: {error}", native_text(source))))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join(SKILL_FILE).is_file())
        .collect::<Vec<_>>();
    candidates.sort();
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err(SkillError::InvalidManifest(format!(
            "{} contains no SKILL.md",
            native_text(source)
        ))),
        count => Err(SkillError::InvalidManifest(format!(
            "{} contains {count} Skills; point at the one to install",
            native_text(source)
        ))),
    }
}

fn source_folder_name(directory: &Path) -> Result<String, SkillError> {
    let name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| SkillError::InvalidPath(native_text(directory)))?;
    // The frontmatter name is a better folder name than a checkout called
    // `main` or `skill-abc123`, but only when it slugifies to something usable.
    let from_frontmatter = read_bounded(&directory.join(SKILL_FILE), MAX_SKILL_MD_BYTES)
        .ok()
        .and_then(|bytes| crate::parser::parse_skill(&bytes, name).ok())
        .map(|parsed| slug_from_name(&parsed.name))
        .filter(|slug| !slug.is_empty());
    let candidate = normalize_relative(name)
        .ok()
        .filter(|name| !name.contains('/'))
        .or(from_frontmatter.clone())
        .ok_or_else(|| SkillError::InvalidPath(native_text(directory)))?;
    Ok(candidate)
}

fn folder_name_for(name: &str) -> Result<String, SkillError> {
    let slug = slug_from_name(name);
    if slug.is_empty() {
        return Err(SkillError::InvalidName(
            "the name must contain letters or digits".to_string(),
        ));
    }
    Ok(slug)
}

fn validated_name(name: &str) -> Result<String, SkillError> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || name.chars().count() > 96 {
        return Err(SkillError::InvalidName(
            "the name must be 1 to 96 characters".to_string(),
        ));
    }
    Ok(name)
}

fn validated_description(description: &str) -> Result<String, SkillError> {
    let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
    if description.is_empty() || description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(SkillError::InvalidName(format!(
            "the description must be 1 to {MAX_DESCRIPTION_CHARS} characters"
        )));
    }
    Ok(description)
}

/// The starting document for a Skill the user just created: valid frontmatter
/// and a skeleton that says what belongs where.
fn new_skill_document(name: &str, description: &str) -> String {
    format!(
        "---\nname: {}\ndescription: {}\n---\n\n# {name}\n\n\
         ## When to use this\n\n{description}\n\n\
         ## How to do it\n\n1. Describe the first step.\n2. Describe the next step.\n\n\
         ## Notes\n\nPut long supporting material in `references/` and runnable helpers in `scripts/`.\n",
        yaml_scalar(name),
        yaml_scalar(description),
    )
}

/// Rewrite the `name:` line of a copied document, or add one when the original
/// relied on its folder name.
fn rename_document(document: &str, name: &str) -> String {
    let mut lines = document.lines().collect::<Vec<_>>();
    let scalar = yaml_scalar(name);
    let closing = lines
        .iter()
        .skip(1)
        .position(|line| line.trim() == "---")
        .map(|index| index + 1);
    let Some(closing) = closing.filter(|_| lines.first().is_some_and(|line| line.trim() == "---"))
    else {
        return document.to_string();
    };
    let replacement = format!("name: {scalar}");
    match lines[1..closing]
        .iter()
        .position(|line| line.trim_start().starts_with("name:"))
    {
        Some(index) => lines[index + 1] = replacement.as_str(),
        None => lines.insert(1, replacement.as_str()),
    }
    let mut document = lines.join("\n");
    document.push('\n');
    document
}

/// Quote a value so it survives as one YAML scalar whatever it contains.
fn yaml_scalar(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{value}\""))
}
