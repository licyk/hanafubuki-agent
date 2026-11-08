//! The Skill tools' typed arguments and their validation.

use hanafubuki_agent_contracts::tools::arguments::valid_bounded_text;
use hanafubuki_agent_contracts::{decode_arguments, object_arguments, ToolCallValidationError};
use hanafubuki_agent_skills::SkillId;
use serde::Deserialize;
use serde_json::Value;

use super::SkillToolKind;

// The argument types ignore unknown fields: the model's arguments are read
// tolerantly, and an error names the field it is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum SkillCatalogAuthority {
    Repo,
    User,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillsListArguments {
    #[serde(default)]
    pub(super) authority: Option<SkillCatalogAuthority>,
    #[serde(default)]
    pub(super) cursor: Option<String>,
}

/// Read a Skill's `SKILL.md`, or one of the files it ships. `path` is the
/// package-relative path the catalog listed, never a host path.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillsReadArguments {
    pub(super) skill: SkillId,
    #[serde(default)]
    pub(super) path: Option<String>,
    #[serde(default)]
    pub(super) cursor: Option<String>,
}

/// Turn one Skill on or off for future turns. Naming the Skill is the whole
/// request: the catalog resolves the id when the call runs, so there is no
/// revision for the model to have guessed right.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillSelectionArguments {
    skill: SkillId,
}

/// One validated Skill tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SkillCall {
    List(SkillsListArguments),
    Read(SkillsReadArguments),
    Enable(SkillId),
    Disable(SkillId),
}

pub(super) fn parse(
    kind: SkillToolKind,
    arguments: &Value,
) -> Result<SkillCall, ToolCallValidationError> {
    let arguments = object_arguments(arguments)?;
    match kind {
        SkillToolKind::SkillsList => {
            let call: SkillsListArguments = decode_arguments(arguments)?;
            validate_skill_cursor(call.cursor.as_deref())?;
            Ok(SkillCall::List(call))
        }
        SkillToolKind::SkillsRead => {
            let call: SkillsReadArguments = decode_arguments(arguments)?;
            if let Some(path) = call.path.as_deref() {
                validate_resource_path(path)?;
            }
            validate_skill_cursor(call.cursor.as_deref())?;
            Ok(SkillCall::Read(call))
        }
        SkillToolKind::EnableSkill => {
            let call: SkillSelectionArguments = decode_arguments(arguments)?;
            Ok(SkillCall::Enable(call.skill))
        }
        SkillToolKind::DisableSkill => {
            let call: SkillSelectionArguments = decode_arguments(arguments)?;
            Ok(SkillCall::Disable(call.skill))
        }
    }
}

/// A package-relative path. Anything absolute, escaping, or hidden is refused
/// here so the catalog never has to interpret a hostile path.
fn validate_resource_path(path: &str) -> Result<(), ToolCallValidationError> {
    let valid = valid_bounded_text(path, 1024, false)
        && !std::path::Path::new(path).is_absolute()
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && path
            .split(['/', '\\'])
            .filter(|segment| !segment.is_empty() && *segment != ".")
            .all(|segment| segment != ".." && !segment.starts_with('.'));
    if !valid {
        return Err(ToolCallValidationError::InvalidArgument("path"));
    }
    Ok(())
}

fn validate_skill_cursor(cursor: Option<&str>) -> Result<(), ToolCallValidationError> {
    if cursor.is_some_and(|cursor| {
        cursor.is_empty()
            || cursor.len() > 256
            || !cursor
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    }) {
        return Err(ToolCallValidationError::InvalidArgument("cursor"));
    }
    Ok(())
}
