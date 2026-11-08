//! The Skill catalog service.
//!
//! One authority owns the roots, the loaded catalog, and the managed user root.
//! Reads are served from a cached scan; anything that writes takes the mutation
//! lock, does its work, and reloads. There is no generation to agree on and no
//! capability to present: an operation names a Skill by its id, and the service
//! decides from the Skill's scope whether it may be changed.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use hanafubuki_agent_foundation::paths::PathKey;
use parking_lot::{Mutex, RwLock};

use crate::discovery::LoadedSkill;
use crate::fs_util::ensure_directory;
use crate::model::{
    SkillCatalogSnapshot, SkillError, SkillId, SkillLoadIssue, SkillRoots, SkillScope,
};

mod catalog;
mod mutations;
mod reads;
mod turn;

/// Bytes of a Skill document handed to the model in one turn.
///
/// This crate is the only authority for it. A consumer that re-derives its own
/// document budget and rejects what this produced turns a Skill that loaded
/// perfectly well into a hard session failure, which is exactly the bug this
/// single ownership exists to make impossible.
pub const MAX_TURN_DOCUMENT_BYTES: usize = 24 * 1024;
/// Bytes of one resource read through the catalog.
pub(crate) const MAX_RESOURCE_READ_BYTES: u64 = 512 * 1024;
/// Skills one catalog may hold.
pub(crate) const MAX_CATALOG_SKILLS: usize = 512;

#[derive(Clone)]
pub struct SkillService {
    inner: Arc<Inner>,
}

struct Inner {
    roots: SkillRoots,
    /// The repository roots of the Agent's current workspace, nearest first.
    repo: RwLock<Vec<PathBuf>>,
    /// Serializes everything that writes to a root or to the state file.
    mutation: Mutex<()>,
    catalog: RwLock<CatalogState>,
}

#[derive(Default)]
struct CatalogState {
    snapshot: Option<SkillCatalogSnapshot>,
    /// What the roots looked like when the snapshot was taken. `None` forces
    /// the next read back to disk.
    fingerprint: Option<Vec<(String, u64, u128)>>,
    skills: BTreeMap<SkillId, LoadedSkill>,
    /// Reported beside the catalog when the filesystem watcher cannot run, so
    /// the user learns their edits will not appear on their own.
    watcher_diagnostic: Option<SkillLoadIssue>,
}

impl SkillService {
    pub fn new(roots: SkillRoots) -> Result<Self, SkillError> {
        validate_roots(&roots)?;
        if roots.install_bundled_system_skills {
            crate::system::synchronize_system_skills(&roots.system)?;
        }
        ensure_directory(&roots.user, "the managed Skill root")?;
        if let Some(parent) = roots.state_file.parent() {
            ensure_directory(parent, "the Skill state directory")?;
        }
        let service = Self {
            inner: Arc::new(Inner {
                roots,
                repo: RwLock::new(Vec::new()),
                mutation: Mutex::new(()),
                catalog: RwLock::new(CatalogState::default()),
            }),
        };
        service.list()?;
        Ok(service)
    }

    pub fn roots(&self) -> &SkillRoots {
        &self.inner.roots
    }

    /// Follows the workspace: discovery reads `repo` (nearest first) before
    /// the user and System roots from now on. A changed set drops the cached
    /// scan, because the roots' stamp is relative to each root and can match
    /// across the change. A root that is the user or System root is skipped,
    /// so no Skill is found under two scopes.
    pub fn set_repo_roots(&self, repo: Vec<PathBuf>) -> Result<(), SkillError> {
        if repo.iter().any(|root| !root.is_absolute()) {
            return Err(SkillError::InvalidPath(
                "Skill roots must be resolved absolute paths".to_string(),
            ));
        }
        let fixed = [
            PathKey::of(&self.inner.roots.user),
            PathKey::of(&self.inner.roots.system),
        ];
        let repo = repo
            .into_iter()
            .filter(|root| !fixed.contains(&PathKey::of(root)))
            .collect::<Vec<_>>();
        let unchanged = self
            .inner
            .repo
            .read()
            .iter()
            .map(|root| PathKey::of(root))
            .eq(repo.iter().map(|root| PathKey::of(root)));
        if unchanged {
            return Ok(());
        }
        *self.inner.repo.write() = repo;
        self.inner.catalog.write().fingerprint = None;
        Ok(())
    }

    /// Roots in the order they are searched: nearest repository first, then the
    /// managed user root, then the bundled Skills.
    pub fn discovery_roots(&self) -> Vec<(PathBuf, SkillScope)> {
        let mut roots = self
            .inner
            .repo
            .read()
            .iter()
            .map(|root| (root.clone(), SkillScope::Repo))
            .collect::<Vec<_>>();
        roots.push((self.inner.roots.user.clone(), SkillScope::User));
        roots.push((self.inner.roots.system.clone(), SkillScope::System));
        roots
    }
}

fn validate_roots(roots: &SkillRoots) -> Result<(), SkillError> {
    for root in [&roots.system, &roots.user, &roots.state_file] {
        if root.as_os_str().is_empty() || !root.is_absolute() {
            return Err(SkillError::InvalidPath(
                "Skill roots must be resolved absolute paths".to_string(),
            ));
        }
    }
    if PathKey::of(&roots.user) == PathKey::of(&roots.system) {
        return Err(SkillError::InvalidPath(
            "Skill discovery roots must be distinct".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
