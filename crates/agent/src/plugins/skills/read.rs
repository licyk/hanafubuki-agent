//! `skills_list` and `skills_read`: pages of the turn's frozen catalog, and a
//! Skill's own files as that catalog listed them.

use base64::Engine as _;
use hanafubuki_agent_contracts::{ToolContextFragment, ToolResult};
use hanafubuki_agent_skills::{SkillResourceKind, SkillScope, SkillTurnCatalog};

use super::admission::selected_skill_context;
use super::arguments::{SkillCatalogAuthority, SkillsListArguments, SkillsReadArguments};
use crate::attachments::normalize_image;
use crate::skills::AgentSkillApplication;

const PAGE_BYTES: usize = 8 * 1024;

pub(super) fn list_skills(
    catalog: &SkillTurnCatalog,
    call: &SkillsListArguments,
) -> Result<ToolResult, String> {
    let entries = catalog
        .entries
        .iter()
        .filter(|entry| {
            entry.allow_implicit_invocation
                && call.authority.as_ref().is_none_or(|authority| {
                    matches!(
                        (authority, entry.id.scope()),
                        (SkillCatalogAuthority::Repo, SkillScope::Repo)
                            | (SkillCatalogAuthority::User, SkillScope::User)
                            | (SkillCatalogAuthority::System, SkillScope::System)
                    )
                })
        })
        .collect::<Vec<_>>();
    let digest = hex_sha256(
        &serde_json::to_vec(&entries.iter().map(|entry| &entry.id).collect::<Vec<_>>())
            .map_err(|error| error.to_string())?,
    );
    let start = decode_cursor(call.cursor.as_deref(), &digest)?;
    if start > entries.len() {
        return Err("Skill catalog cursor is out of range".to_string());
    }
    let mut page = Vec::new();
    let mut encoded_bytes = 0usize;
    let mut next = start;
    for entry in entries.iter().skip(start) {
        let value = serde_json::json!({
            "name": entry.name,
            "description": entry.description,
            "skill": entry.id,
            "directory": entry.directory,
            "authority": entry.id.scope(),
            // Scripts are listed by the run tool, not offered as readable text.
            "resources": entry.resources.iter().filter(|resource| resource.kind != SkillResourceKind::Script).map(|resource| serde_json::json!({
                "path": resource.path,
                "kind": resource.kind,
                "size": resource.size,
            })).collect::<Vec<_>>(),
        });
        let value_bytes = serde_json::to_vec(&value)
            .map_err(|error| error.to_string())?
            .len();
        if !page.is_empty() && encoded_bytes.saturating_add(value_bytes) > PAGE_BYTES {
            break;
        }
        encoded_bytes = encoded_bytes.saturating_add(value_bytes);
        page.push(value);
        next += 1;
    }
    Ok(ToolResult::success(serde_json::json!({
        "entries": page,
        "next_cursor": (next < entries.len()).then(|| encode_cursor(&digest, next)),
    })))
}

pub(super) fn read_skill(
    skills: &AgentSkillApplication,
    catalog: &SkillTurnCatalog,
    call: &SkillsReadArguments,
) -> Result<ToolResult, String> {
    let Some(entry) = catalog.entries.iter().find(|entry| entry.id == call.skill) else {
        return Err(
            match catalog
                .unavailable
                .iter()
                .find(|skill| skill.id == call.skill)
            {
                Some(skill) => format!("Skill is unavailable this turn: {}", skill.reason),
                None => "Skill is not in the current turn catalog".to_string(),
            },
        );
    };
    let Some(path) = call.path.as_deref() else {
        if call.cursor.is_some() {
            return Err("SKILL.md is read whole; a cursor pages a resource path".to_string());
        }
        // The context is the one delivery of the instructions: the kernel
        // commits it once per turn and Skill, so this output does not repeat
        // them and a Skill the turn already has is not delivered twice.
        return Ok(ToolResult::success(serde_json::json!({
            "skill": entry.id,
            "path": "SKILL.md",
            "directory": entry.directory,
            "instructions": "in the conversation as this skill's <skill> block",
        }))
        .with_context(ToolContextFragment {
            source: entry.id.as_str().to_string(),
            name: entry.name.clone(),
            content: selected_skill_context(entry),
        }));
    };
    let resource = entry
        .resources
        .iter()
        .find(|resource| resource.path == path)
        .ok_or_else(|| "Skill resource is not in the current turn catalog".to_string())?;
    match resource.kind {
        SkillResourceKind::Script => {
            Err("Skill scripts cannot be disclosed by skills_read".to_string())
        }
        SkillResourceKind::Reference => {
            let read = skills.read_enabled_resource(&entry.id, path)?;
            paged_skill_text(
                entry.id.as_str(),
                &resource.path,
                &read.content,
                call.cursor.as_deref(),
            )
        }
        SkillResourceKind::Asset => {
            if call.cursor.is_some() {
                return Err("binary Skill assets do not accept a cursor".to_string());
            }
            let read = skills.read_enabled_asset(&entry.id, path)?;
            if image::guess_format(&read.bytes).is_ok() {
                return normalize_image(
                    read.bytes,
                    read.path,
                    hanafubuki_agent_kernel::AgentImageDetail::High,
                )
                .map(ToolResult::image);
            }
            let encoded = base64::engine::general_purpose::STANDARD.encode(read.bytes);
            let output = hanafubuki_agent_tool_contracts::ToolOutput {
                content: vec![
                    hanafubuki_agent_tool_contracts::ToolOutputContent::EmbeddedResource {
                        uri: format!("skill-resource:{}/{}", entry.id, read.path),
                        mime_type: Some(skill_asset_mime_type(&read.path).to_string()),
                        text: None,
                        data: Some(encoded),
                    },
                ],
                structured_content: Some(serde_json::json!({
                    "skill": entry.id,
                    "path": read.path,
                })),
                is_error: false,
            };
            output.validate().map_err(|error| error.to_string())?;
            Ok(output.into())
        }
    }
}

fn paged_skill_text(
    skill: &str,
    resource: &str,
    content: &str,
    cursor: Option<&str>,
) -> Result<ToolResult, String> {
    let digest = hex_sha256(content.as_bytes());
    let start = decode_cursor(cursor, &digest)?;
    if start > content.len() || !content.is_char_boundary(start) {
        return Err("Skill read cursor is out of range".to_string());
    }
    let mut end = start.saturating_add(PAGE_BYTES).min(content.len());
    while end > start && !content.is_char_boundary(end) {
        end -= 1;
    }
    Ok(ToolResult::success(serde_json::json!({
        "skill": skill,
        "path": resource,
        "contents": &content[start..end],
        "next_cursor": (end < content.len()).then(|| encode_cursor(&digest, end)),
    })))
}

fn hex_sha256(value: &[u8]) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(value))
}

fn encode_cursor(digest: &str, offset: usize) -> String {
    format!("v1.{digest}.{offset}")
}

fn decode_cursor(cursor: Option<&str>, expected_digest: &str) -> Result<usize, String> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    let mut parts = cursor.split('.');
    let valid = parts.next() == Some("v1")
        && parts.next() == Some(expected_digest)
        && parts.clone().count() == 1;
    if !valid {
        return Err("Skill cursor does not match the current turn snapshot".to_string());
    }
    parts
        .next()
        .and_then(|offset| offset.parse::<usize>().ok())
        .ok_or_else(|| "Skill cursor is invalid".to_string())
}

fn skill_asset_mime_type(relative_path: &str) -> &'static str {
    match std::path::Path::new(relative_path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        Some("pdf") => "application/pdf",
        Some("txt") | Some("md") => "text/plain",
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        _ => "application/octet-stream",
    }
}
