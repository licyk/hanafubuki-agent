//! Loading and serving the catalog.

use std::collections::BTreeMap;
use std::path::Path;

use crate::discovery::{scan_root, LoadedSkill};
use crate::model::{
    SkillCatalogListRequest, SkillCatalogSnapshot, SkillError, SkillId, SkillLoadIssue,
};
use crate::state::{catalog_revision, load_state};

use super::{CatalogState, SkillService, MAX_CATALOG_SKILLS};

impl SkillService {
    pub fn list(&self) -> Result<SkillCatalogSnapshot, SkillError> {
        self.list_with_request(SkillCatalogListRequest::default())
    }

    pub fn list_with_request(
        &self,
        request: SkillCatalogListRequest,
    ) -> Result<SkillCatalogSnapshot, SkillError> {
        // A cached snapshot is served only while the roots look untouched. The
        // watcher normally invalidates first, but it is a best-effort adapter:
        // a user who edits a Skill in an editor must not have to know that.
        if !request.force_reload {
            let fingerprint = self.roots_fingerprint();
            let catalog = self.inner.catalog.read();
            if catalog.fingerprint.as_ref() == Some(&fingerprint) {
                if let Some(snapshot) = catalog.snapshot.clone() {
                    return Ok(snapshot);
                }
            }
        }
        let _mutation = self.inner.mutation.lock();
        self.reload()
    }

    /// A cheap stamp of every discovery root: entry names, sizes, and modified
    /// times. Never file contents — this decides whether to look again, and
    /// reading every Skill to answer that would defeat the point.
    fn roots_fingerprint(&self) -> Vec<(String, u64, u128)> {
        let mut entries = Vec::new();
        for (root, _) in self.discovery_roots() {
            stamp_directory(&root, &root, 0, &mut entries);
        }
        entries.sort();
        entries
    }

    /// Drop the cached scan so the next read goes back to disk. The watcher
    /// calls this for the root it saw change; the whole catalog is cheap enough
    /// to reload that tracking which root it was would only add a way to be
    /// wrong.
    pub fn invalidate_root(&self, root: &Path) -> Result<(), SkillError> {
        if !root.is_absolute() {
            return Err(SkillError::InvalidPath(
                "Skill invalidation root must be absolute".to_string(),
            ));
        }
        self.inner.catalog.write().fingerprint = None;
        Ok(())
    }

    pub fn set_watcher_diagnostic(&self, message: Option<String>) {
        let mut catalog = self.inner.catalog.write();
        catalog.watcher_diagnostic =
            message.map(|message| SkillLoadIssue::warning(String::new(), message));
        catalog.fingerprint = None;
    }

    /// Rescan every root and republish the snapshot. Callers hold the mutation
    /// lock.
    pub(super) fn reload(&self) -> Result<SkillCatalogSnapshot, SkillError> {
        let mut skills: BTreeMap<SkillId, LoadedSkill> = BTreeMap::new();
        let mut issues = Vec::new();
        for (root, scope, plugin) in self.scanned_roots() {
            let scan = scan_root(&root, scope, plugin.as_ref());
            issues.extend(scan.issues);
            for skill in scan.skills {
                // Repo roots can nest, so the same relative path is reachable
                // from two of them. Nearest-first ordering means the first one
                // seen is the one the user meant.
                match skills.entry(skill.entry.id.clone()) {
                    std::collections::btree_map::Entry::Vacant(slot) => {
                        slot.insert(skill);
                    }
                    std::collections::btree_map::Entry::Occupied(existing) => {
                        issues.push(SkillLoadIssue::warning(
                            skill.entry.directory.clone(),
                            format!(
                                "{} is already provided by {}",
                                skill.entry.id,
                                existing.get().entry.directory
                            ),
                        ));
                    }
                }
            }
            if skills.len() > MAX_CATALOG_SKILLS {
                return Err(SkillError::LimitExceeded(format!(
                    "the catalog supports at most {MAX_CATALOG_SKILLS} Skills"
                )));
            }
        }

        let state = load_state(&self.inner.roots.state_file)?;
        for skill in skills.values_mut() {
            skill.entry.enabled = !state.is_disabled(&skill.entry.id);
        }
        let mut entries = skills
            .values()
            .map(|skill| skill.entry.clone())
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            left.display_name()
                .to_lowercase()
                .cmp(&right.display_name().to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });

        // Stamped after the scan, so a change made while it ran is seen next
        // time rather than cached away.
        let fingerprint = self.roots_fingerprint();
        let mut catalog = self.inner.catalog.write();
        if let Some(diagnostic) = catalog.watcher_diagnostic.clone() {
            issues.push(diagnostic);
        }
        let snapshot = SkillCatalogSnapshot {
            revision: catalog_revision(&entries, state.revision),
            entries,
            issues,
        };
        catalog.snapshot = Some(snapshot.clone());
        catalog.fingerprint = Some(fingerprint);
        catalog.skills = skills;
        Ok(snapshot)
    }

    /// Resolve one Skill from the loaded catalog, reloading first if the cache
    /// was invalidated.
    pub(super) fn loaded(&self, id: &SkillId) -> Result<LoadedSkill, SkillError> {
        self.list()?;
        self.inner
            .catalog
            .read()
            .skills
            .get(id)
            .cloned()
            .ok_or_else(|| SkillError::NotFound(id.to_string()))
    }

    pub(super) fn with_catalog<T>(&self, read: impl FnOnce(&CatalogState) -> T) -> T {
        read(&self.inner.catalog.read())
    }
}

fn stamp_directory(
    root: &Path,
    directory: &Path,
    depth: usize,
    entries: &mut Vec<(String, u64, u128)>,
) {
    if depth > crate::fs_util::MAX_DIRECTORY_DEPTH || entries.len() > MAX_FINGERPRINT_ENTRIES {
        return;
    }
    let Ok(read) = std::fs::read_dir(directory) else {
        // A root that is not there yet stamps as absent and reappears the
        // moment it exists.
        return;
    };
    for entry in read.filter_map(Result::ok) {
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let modified = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |value| value.as_nanos());
        entries.push((
            path.strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned(),
            metadata.len(),
            modified,
        ));
        if path.is_dir() {
            stamp_directory(root, &path, depth.saturating_add(1), entries);
        }
    }
}

const MAX_FINGERPRINT_ENTRIES: usize = 20_000;
