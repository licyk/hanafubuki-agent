//! The Skills bundled with the application.
//!
//! They live in the binary and are written into the System root whenever what
//! is on disk is not exactly what is embedded — on first run, after an upgrade,
//! and after someone edits them. Republishing is cheap and the root is not the
//! user's to keep, so the check is content equality rather than a version
//! number that could be wrong.

use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::fs_util::{
    collect_files, ensure_directory, native_text, next_temporary_token, replace_directory,
    write_new_file,
};
use crate::model::SkillError;

const BUNDLED_FILES: &[(&str, &[u8])] = &[
    (
        "skill-creator/SKILL.md",
        include_bytes!("../system-skills/skill-creator/SKILL.md"),
    ),
    (
        "skill-creator/agents/openai.yaml",
        include_bytes!("../system-skills/skill-creator/agents/openai.yaml"),
    ),
    (
        "skill-manager/SKILL.md",
        include_bytes!("../system-skills/skill-manager/SKILL.md"),
    ),
    (
        "skill-manager/agents/openai.yaml",
        include_bytes!("../system-skills/skill-manager/agents/openai.yaml"),
    ),
];

pub(crate) fn synchronize_system_skills(root: &Path) -> Result<(), SkillError> {
    let parent = root.parent().ok_or_else(|| {
        SkillError::InvalidPath("the System Skill root must have a parent".to_string())
    })?;
    ensure_directory(parent, "the System Skill parent")?;
    if matches_embedded(root)? {
        return Ok(());
    }
    let staging = parent.join(format!(
        ".system.staging.{}.{}",
        std::process::id(),
        next_temporary_token()
    ));
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    ensure_directory(&staging, "the System Skill staging root")?;
    let result = (|| {
        for (relative, contents) in BUNDLED_FILES {
            write_new_file(&staging.join(relative), contents)?;
        }
        replace_directory(&staging, root)
    })();
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result?;
    log::info!("published bundled System Skills at {}", native_text(root));
    Ok(())
}

fn matches_embedded(root: &Path) -> Result<bool, SkillError> {
    if !root.is_dir() {
        return Ok(false);
    }
    let mut expected = BUNDLED_FILES
        .iter()
        .map(|(path, bytes)| ((*path).to_string(), hex::encode(Sha256::digest(bytes))))
        .collect::<Vec<_>>();
    expected.sort();
    let mut actual = collect_files(root)?
        .into_iter()
        .map(|file| {
            let bytes = fs::read(&file.path).map_err(|error| {
                SkillError::Io(format!("read {}: {error}", native_text(&file.path)))
            })?;
            Ok((file.relative_path, hex::encode(Sha256::digest(bytes))))
        })
        .collect::<Result<Vec<_>, SkillError>>()?;
    actual.sort();
    Ok(actual == expected)
}

#[cfg(test)]
mod tests;
