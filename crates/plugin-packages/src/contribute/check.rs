//! What the registration of a package's contributions would leave out,
//! found without starting its guest: the rules its proxies apply when they
//! register, for `plugins validate`.

use std::collections::BTreeSet;

use hanafubuki_agent_packages::{PackageContributes, PluginId};
use hanafubuki_agent_tool_contracts::{
    validate_description, validate_json_schema, ToolId, ToolName,
};
use serde_json::Value;

use super::context::{guest_kind, MAXIMUM_GUEST_KINDS};
use super::observers::EVENTS;
use super::prompt::{prompt_kind, slot};

/// The declarations of the package `id` that would be left out or changed
/// when its guest's proxies register, each as an issue.
pub(crate) fn check_contributes(id: &PluginId, contributes: &PackageContributes) -> Vec<String> {
    let mut issues = Vec::new();
    let mut tools = BTreeSet::new();
    for tool in &contributes.tools {
        // The tool source names a tool `<namespace>__<name>` with a
        // namespace always valid, so the name alone decides.
        if ToolName::parse(tool.name.clone()).is_err() || ToolId::parse(tool.name.clone()).is_err()
        {
            issues.push(format!(
                "the tool `{}` would be left out: its name is not a tool name \
                 (ASCII letters, digits, `_` and `-`, at most 128 bytes)",
                tool.name
            ));
        }
        if !tools.insert(tool.name.as_str()) {
            issues.push(format!("the tool `{}` is declared twice", tool.name));
        }
        if validate_json_schema(&Value::Object(tool.input_schema.clone())).is_err() {
            issues.push(format!(
                "the tool `{}` would be left out: its inputSchema is too large or too deep",
                tool.name
            ));
        }
        if validate_description(&tool.description).is_err() {
            issues.push(format!(
                "the tool `{}` would be left out: its description is too long",
                tool.name
            ));
        }
    }
    let sections = &contributes.context_sections;
    if sections.len() > MAXIMUM_GUEST_KINDS {
        issues.push(format!(
            "only its first {MAXIMUM_GUEST_KINDS} context sections would be registered, of {}",
            sections.len()
        ));
    }
    let mut kinds = BTreeSet::new();
    for section in sections {
        if guest_kind(&section.kind).is_none() {
            issues.push(format!(
                "the context section `{}` would be left out: its kind is not a plugin's kind \
                 (lowercase letters, digits and `_`, not one of the Agent's own)",
                section.kind
            ));
        }
        if !kinds.insert(section.kind.as_str()) {
            issues.push(format!(
                "the context section `{}` is declared twice",
                section.kind
            ));
        }
    }
    if let Some(declared) = &contributes.prompt_slot {
        if slot(declared).is_none() {
            issues.push(format!(
                "its prompt's slot `{declared}` is not one of instructions, worldState, \
                 capabilities or memory; capabilities would be used"
            ));
        }
        if guest_kind(&prompt_kind(id)).is_none() {
            issues.push("its prompt would be left out: its name gives no prompt kind".to_string());
        }
    }
    let unknown = contributes
        .observers
        .iter()
        .filter(|event| !EVENTS.contains(&event.as_str()))
        .map(|event| format!("`{event}`"))
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        issues.push(format!(
            "it observes no events named {} (the events are {})",
            unknown.join(", "),
            EVENTS.join(", ")
        ));
    }
    issues
}
