//! The model-visible specifications of the Skill tools.

use serde_json::{json, Value};

use super::SkillToolKind;

/// The model-visible description and input schema of each Skill tool.
pub(super) fn spec(kind: SkillToolKind) -> (String, Value) {
    match kind {
        SkillToolKind::SkillsList => (
            "List the Skills available for this turn. Each entry carries the id every other Skill tool takes, its name, its description, and its directory. Follow cursor until next_cursor is absent.".to_string(),
            json!({
                "type": "object",
                "properties": {
                    "authority": {"type":["string","null"],"enum":["repo","user","system",null]},
                    "cursor": {"type":["string","null"],"maxLength":256}
                },
                "additionalProperties": false
            }),
        ),
        SkillToolKind::SkillsRead => (
            "Read one Skill. Omit path to adopt the Skill for this turn: its SKILL.md is added to the conversation once, as a <skill> block. Otherwise pass a package-relative path the catalog listed and follow next_cursor until absent.".to_string(),
            json!({
                "type": "object",
                "properties": {
                    "skill": skill_id_schema(),
                    "path": {"oneOf":[skill_resource_path_schema(),{"type":"null"}]},
                    "cursor": {"type":["string","null"],"maxLength":256}
                },
                "required": ["skill"],
                "additionalProperties": false
            }),
        ),
        SkillToolKind::EnableSkill => (
            "Turn one Skill on for future user turns after explicit approval. This does not change the current turn.".to_string(),
            selection_schema(),
        ),
        SkillToolKind::DisableSkill => (
            "Turn one Skill off for future user turns after explicit approval. This does not rewrite the current or past turns.".to_string(),
            selection_schema(),
        ),
    }
}

fn selection_schema() -> Value {
    json!({
        "type":"object",
        "properties": {"skill": skill_id_schema()},
        "required":["skill"],
        "additionalProperties":false
    })
}

/// A Skill's identity: the scope it was discovered under and its directory path
/// under that scope's root, exactly as the catalog reports it.
fn skill_id_schema() -> Value {
    json!({
        "type":"string",
        "minLength":6,
        "maxLength":200,
        "pattern":"^(system|user|repo):[^/][^\\\\:]*$"
    })
}

/// A file inside a Skill, addressed by its package-relative path.
fn skill_resource_path_schema() -> Value {
    json!({"type":"string","minLength":1,"maxLength":1024})
}
