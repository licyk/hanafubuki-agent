use std::collections::HashSet;

use hanafubuki_agent_tool_contracts::{
    canonical_digest, validate_json_schema, FunctionToolSpec, ToolGeneration, ToolId, ToolName,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::listing::McpListing;
use crate::{
    McpCatalogGeneration, McpConnectionGeneration, McpError, McpExposurePolicy, McpServerConfig,
    McpServerId,
};

pub const MAX_MCP_TOOLS_PER_SERVER: usize = 2048;
pub const MAX_MCP_TOOL_DESCRIPTION_BYTES: usize = 16 * 1024;
pub const MAX_MCP_CATALOG_ISSUE_NAME_BYTES: usize = 256;

/// A tool's annotations as the client uses them. They are hints from a
/// server the user configured, never a grant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolHints {
    /// `readOnlyHint` is true and `destructiveHint` is not: the tool may run
    /// beside other reads and in Plan mode.
    pub read_only: bool,
    /// `destructiveHint`, true unless the server says false; approval reads
    /// only this, so a read-only hint alone never skips a review.
    pub destructive: bool,
    pub idempotent: bool,
    pub open_world: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolBinding {
    pub server_id: McpServerId,
    pub connection_generation: McpConnectionGeneration,
    pub catalog_generation: McpCatalogGeneration,
    pub remote_name: String,
    pub tool_id: ToolId,
    pub tool_generation: ToolGeneration,
    pub specification: FunctionToolSpec,
    pub title: Option<String>,
    pub output_schema: Option<Value>,
    pub model_visible: bool,
    pub app_visible: bool,
    pub hints: McpToolHints,
    pub exposure: McpExposurePolicy,
}

/// One tool the catalog skipped or adjusted. It names the tool, never its
/// content: a server's bad tool costs only that tool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCatalogIssue {
    /// The remote tool name, bounded to [`MAX_MCP_CATALOG_ISSUE_NAME_BYTES`].
    pub remote_name: String,
    pub kind: McpCatalogIssueKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpCatalogIssueKind {
    /// Kept, with the description cut to [`MAX_MCP_TOOL_DESCRIPTION_BYTES`].
    DescriptionTruncated,
    /// Kept, without its invalid or oversized output schema.
    OutputSchemaDropped,
    /// Skipped: the input schema is invalid or oversized.
    InvalidInputSchema,
    /// Skipped: the name cannot become a tool identity.
    InvalidName,
    /// Skipped: an earlier tool in the listing has the same encoded name.
    DuplicateName,
    /// Skipped: the listed tool does not decode as an MCP tool.
    Undecodable,
}

impl McpCatalogIssueKind {
    pub fn skips_tool(self) -> bool {
        match self {
            Self::DescriptionTruncated | Self::OutputSchemaDropped => false,
            Self::InvalidInputSchema
            | Self::InvalidName
            | Self::DuplicateName
            | Self::Undecodable => true,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::DescriptionTruncated => "tool_description_truncated",
            Self::OutputSchemaDropped => "tool_output_schema_dropped",
            Self::InvalidInputSchema => "tool_skipped_invalid_input_schema",
            Self::InvalidName => "tool_skipped_invalid_name",
            Self::DuplicateName => "tool_skipped_duplicate_name",
            Self::Undecodable => "tool_skipped_undecodable",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::DescriptionTruncated => "MCP tool description was truncated to the size limit",
            Self::OutputSchemaDropped => {
                "MCP tool output schema is invalid or too large and was dropped"
            }
            Self::InvalidInputSchema => {
                "MCP tool was skipped: its input schema is invalid or too large"
            }
            Self::InvalidName => "MCP tool was skipped: its name cannot be encoded",
            Self::DuplicateName => {
                "MCP tool was skipped: an earlier tool has the same encoded name"
            }
            Self::Undecodable => "MCP tool was skipped: its definition could not be decoded",
        }
    }
}

/// The usable tools of one listing and what was skipped or adjusted to keep
/// them. Zero usable tools is a valid catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpToolCatalog {
    pub generation: McpCatalogGeneration,
    pub tools: Vec<McpToolBinding>,
    pub issues: Vec<McpCatalogIssue>,
}

/// Builds the catalog tool by tool: a listed tool that did not decode or
/// cannot be represented is skipped, an over-long description is truncated,
/// and of two tools with the same encoded name the first in listing order is
/// kept. The listing it is built from is already bounded to
/// [`MAX_MCP_TOOLS_PER_SERVER`] entries while it is paged
/// (`listing::list_all`).
pub(crate) fn build_tool_catalog(
    config: &McpServerConfig,
    connection_generation: McpConnectionGeneration,
    listing: McpListing<rmcp::model::Tool>,
) -> Result<McpToolCatalog, McpError> {
    let namespace = vec!["mcp".to_string(), safe_identifier(config.id.as_str())];
    let mut bindings = Vec::new();
    let mut issues = listing
        .undecodable
        .into_iter()
        .filter(|remote_name| is_selected(config, remote_name))
        .map(|remote_name| McpCatalogIssue {
            remote_name,
            kind: McpCatalogIssueKind::Undecodable,
        })
        .collect::<Vec<_>>();
    let mut names = HashSet::new();
    for tool in listing.items {
        let remote_name = tool.name.to_string();
        if !is_selected(config, &remote_name) {
            continue;
        }
        let mut issue = |kind| {
            issues.push(McpCatalogIssue {
                remote_name: bounded_name(&remote_name),
                kind,
            })
        };
        match build_binding(config, connection_generation, &namespace, tool, &mut issue) {
            Ok(binding) if names.insert(binding.specification.name.qualified()) => {
                bindings.push(binding)
            }
            Ok(_) => issue(McpCatalogIssueKind::DuplicateName),
            Err(kind) => issue(kind),
        }
    }
    bindings.sort_by(|left, right| {
        left.specification
            .name
            .qualified()
            .cmp(&right.specification.name.qualified())
    });
    let generation =
        McpCatalogGeneration(canonical_digest(&bindings).map_err(|_| McpError::LimitExceeded)?);
    for binding in &mut bindings {
        binding.catalog_generation = generation;
    }
    Ok(McpToolCatalog {
        generation,
        tools: bindings,
        issues,
    })
}

/// Whether the configuration's tool filters leave the tool in the catalog.
fn is_selected(config: &McpServerConfig, remote_name: &str) -> bool {
    (config.enabled_tools.is_empty() || config.enabled_tools.contains(remote_name))
        && !config.disabled_tools.contains(remote_name)
}

/// One tool's binding; `adjusted` receives what was changed to keep it.
fn build_binding(
    config: &McpServerConfig,
    connection_generation: McpConnectionGeneration,
    namespace: &[String],
    tool: rmcp::model::Tool,
    adjusted: &mut impl FnMut(McpCatalogIssueKind),
) -> Result<McpToolBinding, McpCatalogIssueKind> {
    let remote_name = tool.name.into_owned();
    let input_schema = Value::Object((*tool.input_schema).clone());
    validate_json_schema(&input_schema).map_err(|_| McpCatalogIssueKind::InvalidInputSchema)?;
    let safe_remote_name = safe_identifier(&remote_name);
    let model_name = ToolName::namespaced(namespace.to_vec(), safe_remote_name.clone())
        .map_err(|_| McpCatalogIssueKind::InvalidName)?;
    let tool_id = ToolId::parse(safe_remote_name).map_err(|_| McpCatalogIssueKind::InvalidName)?;
    let mut description = tool
        .description
        .map(|value| value.into_owned())
        .unwrap_or_default();
    if description.len() > MAX_MCP_TOOL_DESCRIPTION_BYTES {
        description.truncate(utf8_boundary(&description, MAX_MCP_TOOL_DESCRIPTION_BYTES));
        adjusted(McpCatalogIssueKind::DescriptionTruncated);
    }
    let output_schema = tool
        .output_schema
        .map(|schema| Value::Object((*schema).clone()))
        .filter(|schema| {
            let usable = validate_json_schema(schema).is_ok()
                && serde_json::to_vec(schema)
                    .is_ok_and(|bytes| bytes.len() <= MAX_MCP_TOOL_DESCRIPTION_BYTES);
            if !usable {
                adjusted(McpCatalogIssueKind::OutputSchemaDropped);
            }
            usable
        });
    let title = tool.title.map(|title| {
        if title.len() <= 256 {
            title
        } else {
            title.chars().take(256).collect()
        }
    });
    let (model_visible, app_visible) = tool_visibility(tool.meta.as_ref());
    let annotations = tool.annotations.unwrap_or_default();
    let hints = McpToolHints {
        read_only: annotations.read_only_hint == Some(true)
            && annotations.destructive_hint != Some(true),
        destructive: annotations.destructive_hint.unwrap_or(true),
        idempotent: annotations.idempotent_hint.unwrap_or(false),
        open_world: annotations.open_world_hint.unwrap_or(true),
    };
    let exposure = if model_visible {
        match (app_visible, config.exposure_policy_for(&remote_name)) {
            (false, McpExposurePolicy::Direct) => McpExposurePolicy::DirectModelOnly,
            (_, exposure) => exposure,
        }
    } else {
        McpExposurePolicy::Hidden
    };
    let approval = config.approval_policy_for(&remote_name);
    let specification = FunctionToolSpec {
        name: model_name,
        description,
        input_schema,
        strict: false,
        deferred: exposure == McpExposurePolicy::Deferred,
    };
    let generation = ToolGeneration(
        canonical_digest(&(
            config.id.as_str(),
            &connection_generation,
            &remote_name,
            &specification,
            hints,
            &title,
            &output_schema,
            model_visible,
            app_visible,
            approval,
            exposure,
        ))
        .map_err(|_| McpCatalogIssueKind::InvalidInputSchema)?,
    );
    Ok(McpToolBinding {
        server_id: config.id.clone(),
        connection_generation,
        catalog_generation: McpCatalogGeneration([0; 32]),
        remote_name,
        tool_id,
        tool_generation: generation,
        specification,
        title,
        output_schema,
        model_visible,
        app_visible,
        hints,
        exposure,
    })
}

fn utf8_boundary(value: &str, maximum: usize) -> usize {
    let mut boundary = maximum.min(value.len());
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

pub(crate) fn bounded_name(value: &str) -> String {
    value[..utf8_boundary(value, MAX_MCP_CATALOG_ISSUE_NAME_BYTES)].to_string()
}

fn tool_visibility(meta: Option<&rmcp::model::MetaObject>) -> (bool, bool) {
    let Some(visibility) = meta
        .and_then(|meta| meta.0.get("ui"))
        .and_then(Value::as_object)
        .and_then(|ui| ui.get("visibility"))
        .and_then(Value::as_array)
    else {
        return (true, true);
    };
    (
        visibility
            .iter()
            .any(|target| target.as_str() == Some("model")),
        visibility
            .iter()
            .any(|target| target.as_str() == Some("app")),
    )
}

/// One name segment in the model tool identifier charset: each character
/// outside `[A-Za-z0-9_-]` (a `.`, which `McpServerId` and remote names
/// allow, or anything else a server names a tool with) becomes `_`, so the
/// name stays readable. A segment still over the identifier limit keeps a
/// prefix and ends with `_` and 12 hex digits of a hash of the whole name.
fn safe_identifier(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        sanitized.push('_');
    }
    let maximum = hanafubuki_agent_tool_contracts::MAX_TOOL_IDENTIFIER_BYTES;
    if sanitized.len() > maximum {
        let digest = canonical_digest(&value).expect("strings always have a canonical digest");
        sanitized.truncate(maximum - 13);
        sanitized.push('_');
        for byte in &digest[..6] {
            sanitized.push_str(&format!("{byte:02x}"));
        }
    }
    sanitized
}
