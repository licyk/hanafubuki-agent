//! Reading a Skill and the files it ships.

use crate::discovery::{resource_kind, SKILL_FILE};
use crate::fs_util::{ensure_below, normalize_relative, read_bounded};
use crate::model::{
    SkillAssetRead, SkillError, SkillFileRead, SkillId, SkillRead, SkillResourceKind,
    SkillResourceRead,
};

use super::{SkillService, MAX_RESOURCE_READ_BYTES};

impl SkillService {
    /// A Skill and its complete `SKILL.md`.
    pub fn read(&self, id: &SkillId) -> Result<SkillRead, SkillError> {
        let skill = self.loaded(id)?;
        Ok(SkillRead {
            entry: skill.entry,
            instructions: skill.document,
        })
    }

    /// Any text file inside a Skill, `SKILL.md` included. This is the authoring
    /// read: it does not care whether the Skill is turned on, because editing a
    /// Skill you have switched off is a perfectly ordinary thing to do.
    pub fn read_file(&self, id: &SkillId, path: &str) -> Result<SkillFileRead, SkillError> {
        let (path, bytes) = self.resource_bytes(id, path)?;
        Ok(SkillFileRead {
            id: id.clone(),
            content: text(&bytes, &path)?,
            path,
        })
    }

    /// A resource read on the model's behalf. Scripts are never disclosed here:
    /// they run through the approved script tool or not at all.
    pub fn read_enabled_resource(
        &self,
        id: &SkillId,
        path: &str,
    ) -> Result<SkillResourceRead, SkillError> {
        self.ensure_enabled(id)?;
        let (path, bytes) = self.resource_bytes(id, path)?;
        let kind = resource_kind(&path);
        if kind == SkillResourceKind::Script {
            return Err(SkillError::UnsupportedResource(path));
        }
        Ok(SkillResourceRead {
            id: id.clone(),
            content: text(&bytes, &path)?,
            path,
            kind,
        })
    }

    /// Bytes of a packaged asset, for the surfaces that render one.
    pub fn read_enabled_asset(
        &self,
        id: &SkillId,
        path: &str,
    ) -> Result<SkillAssetRead, SkillError> {
        self.ensure_enabled(id)?;
        let (path, bytes) = self.resource_bytes(id, path)?;
        if resource_kind(&path) == SkillResourceKind::Script {
            return Err(SkillError::UnsupportedResource(path));
        }
        Ok(SkillAssetRead {
            id: id.clone(),
            path,
            bytes,
        })
    }

    fn ensure_enabled(&self, id: &SkillId) -> Result<(), SkillError> {
        if self.loaded(id)?.entry.enabled {
            return Ok(());
        }
        Err(SkillError::Disabled(id.to_string()))
    }

    /// Read one file inside a Skill, refusing anything that is not a bounded
    /// regular file within its directory.
    fn resource_bytes(&self, id: &SkillId, path: &str) -> Result<(String, Vec<u8>), SkillError> {
        let skill = self.loaded(id)?;
        let path = normalize_relative(path)?;
        let known = path == SKILL_FILE
            || skill
                .entry
                .resources
                .iter()
                .any(|resource| resource.path == path);
        if !known {
            return Err(SkillError::ResourceNotFound(path));
        }
        let absolute = skill.directory.join(&path);
        ensure_below(&skill.directory, &absolute)?;
        let bytes = read_bounded(&absolute, MAX_RESOURCE_READ_BYTES)?;
        Ok((path, bytes))
    }
}

fn text(bytes: &[u8], path: &str) -> Result<String, SkillError> {
    String::from_utf8(bytes.to_vec()).map_err(|_| SkillError::UnsupportedResource(path.to_string()))
}
