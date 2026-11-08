//! The optional `agents/openai.yaml` interface file.
//!
//! Best-effort by design. This file decorates a Skill with a display name, a
//! short description, icons, and the MCP servers it wants; none of that is
//! required for the Skill to work, so nothing here returns an error. Unknown
//! keys, unknown transports, and malformed URLs are simply ignored; a field of
//! the wrong type is ignored on its own, and a file that is not YAML at all is
//! ignored whole, each with an issue beside the catalog. The Skill loads from
//! its `SKILL.md` regardless.
//! Rejecting a package because its decoration disagreed with a schema is
//! exactly the failure this design exists to avoid.

use serde_yaml::{Mapping, Value};

use crate::model::{SkillInterface, SkillMcpTransport, SkillToolDependency};
use crate::yaml_fields::{
    ignored, kind, optional_bool, optional_mapping, optional_sequence, optional_string,
};

pub(crate) const MAX_INTERFACE_BYTES: u64 = 64 * 1024;

const MAX_DEPENDENCIES: usize = 32;
/// A display name is a title, not prose. Codex bounds one at 64 characters and
/// nothing real comes close.
const MAX_NAME_BYTES: usize = 256;
/// Matches the description bound Codex applies to the same fields.
const MAX_TEXT_BYTES: usize = 4096;
const MAX_SERVER_ID_BYTES: usize = 128;

/// One `dependencies.tools` entry as written, before it is judged.
struct OpenAiToolDependency {
    dependency_type: Option<String>,
    value: Option<String>,
    description: Option<String>,
    transport: Option<String>,
    url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParsedInterface {
    pub(crate) interface: SkillInterface,
    pub(crate) allow_implicit_invocation: bool,
    pub(crate) dependencies: Vec<SkillToolDependency>,
    /// Fields of the wrong type that were ignored, and a file that could not
    /// be read at all, for the catalog's issues.
    pub(crate) issues: Vec<String>,
}

impl Default for ParsedInterface {
    fn default() -> Self {
        Self {
            interface: SkillInterface::default(),
            allow_implicit_invocation: true,
            dependencies: Vec::new(),
            issues: Vec::new(),
        }
    }
}

/// Read what the interface file offers. A file this cannot understand yields
/// the defaults; a field of the wrong type yields its default and leaves the
/// other fields as written.
pub(crate) fn parse_interface(bytes: &[u8]) -> ParsedInterface {
    let unreadable = |message: String| ParsedInterface {
        issues: vec![message],
        ..ParsedInterface::default()
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return unreadable("ignored the file: it is not UTF-8".to_string());
    };
    let manifest = match serde_yaml::from_str::<Value>(&text.replace("\r\n", "\n")) {
        Ok(Value::Mapping(manifest)) => manifest,
        Ok(Value::Null) => return ParsedInterface::default(),
        Ok(other) => {
            return unreadable(format!(
                "ignored the file: expected a mapping, found {}",
                kind(&other)
            ))
        }
        Err(_) => return unreadable("ignored the file: it is not valid YAML".to_string()),
    };
    let mut issues = Vec::new();
    let interface = optional_mapping(&manifest, "interface", "interface", &mut issues);
    let text = |key: &str, issues: &mut Vec<String>| {
        interface.and_then(|interface| {
            optional_string(interface, &[key], &format!("interface.{key}"), issues)
        })
    };
    let interface = SkillInterface {
        display_name: bounded_to(text("display_name", &mut issues), MAX_NAME_BYTES),
        short_description: bounded(text("short_description", &mut issues)),
        icon_small: relative_resource(text("icon_small", &mut issues)),
        icon_large: relative_resource(text("icon_large", &mut issues)),
        brand_color: brand_color(text("brand_color", &mut issues)),
        default_prompt: bounded(text("default_prompt", &mut issues)),
    };
    let allow_implicit_invocation = optional_mapping(&manifest, "policy", "policy", &mut issues)
        .and_then(|policy| {
            optional_bool(
                policy,
                "allow_implicit_invocation",
                "policy.allow_implicit_invocation",
                &mut issues,
            )
        })
        .unwrap_or(true);
    let dependencies = optional_mapping(&manifest, "dependencies", "dependencies", &mut issues)
        .and_then(|dependencies| {
            optional_sequence(dependencies, "tools", "dependencies.tools", &mut issues)
        })
        .unwrap_or_default()
        .iter()
        .enumerate()
        .filter_map(|(index, value)| tool_dependency(index, value, &mut issues))
        .filter_map(dependency)
        .take(MAX_DEPENDENCIES)
        .collect();
    ParsedInterface {
        interface,
        allow_implicit_invocation,
        dependencies,
        issues,
    }
}

fn tool_dependency(
    index: usize,
    value: &Value,
    issues: &mut Vec<String>,
) -> Option<OpenAiToolDependency> {
    let label = format!("dependencies.tools[{index}]");
    let Value::Mapping(entry) = value else {
        issues.push(ignored(&label, "a mapping", value));
        return None;
    };
    let mut text = |key: &str| text_field(entry, key, &label, issues);
    Some(OpenAiToolDependency {
        dependency_type: text("type"),
        value: text("value"),
        description: text("description"),
        transport: text("transport"),
        url: text("url"),
    })
}

fn text_field(entry: &Mapping, key: &str, label: &str, issues: &mut Vec<String>) -> Option<String> {
    optional_string(entry, &[key], &format!("{label}.{key}"), issues)
}

fn dependency(raw: OpenAiToolDependency) -> Option<SkillToolDependency> {
    // Only MCP dependencies mean anything here; a future kind is ignored, not
    // an error.
    if raw
        .dependency_type
        .as_deref()
        .is_some_and(|kind| kind != "mcp")
    {
        return None;
    }
    let server_id = raw.value?;
    let valid = !server_id.is_empty()
        && server_id.len() <= MAX_SERVER_ID_BYTES
        && server_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if !valid {
        return None;
    }
    Some(SkillToolDependency {
        description: bounded(raw.description).unwrap_or_default(),
        transport: match raw.transport.as_deref() {
            Some("stdio") => Some(SkillMcpTransport::Stdio),
            Some("streamable_http") => Some(SkillMcpTransport::StreamableHttp),
            Some("sse") => Some(SkillMcpTransport::Sse),
            _ => None,
        },
        url: raw.url.filter(|url| {
            url::Url::parse(url).is_ok_and(|parsed| matches!(parsed.scheme(), "http" | "https"))
        }),
        server_id,
    })
}

fn bounded(value: Option<String>) -> Option<String> {
    bounded_to(value, MAX_TEXT_BYTES)
}

fn bounded_to(value: Option<String>, maximum: usize) -> Option<String> {
    value
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|value| !value.is_empty() && value.len() <= maximum)
}

/// A package-relative resource path. An absolute or escaping path is dropped;
/// whether the file exists is settled when the directory is scanned.
fn relative_resource(value: Option<String>) -> Option<String> {
    value.filter(|value| {
        !value.is_empty()
            && value.len() <= 1024
            && !value.starts_with('/')
            && !value.starts_with('\\')
            && !value.split(['/', '\\']).any(|segment| segment == "..")
            && !value.contains(':')
    })
}

fn brand_color(value: Option<String>) -> Option<String> {
    value.filter(|color| {
        color.len() == 7
            && color.starts_with('#')
            && color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

#[cfg(test)]
mod tests;
