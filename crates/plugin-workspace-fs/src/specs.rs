//! Model-visible descriptions and schemas. Moving a tool between crates must
//! not change these; the Agent's model-request goldens pin them.

use hanafubuki_agent_tool_contracts::{
    FreeformToolFormat, FreeformToolSpec, ToolContractError, ToolName,
};
use serde_json::{json, Value};

use crate::tool::FsToolKind;

pub(crate) struct FsToolSpec {
    pub description: &'static str,
    pub input_schema: Value,
}

pub(crate) fn spec(kind: FsToolKind) -> FsToolSpec {
    match kind {
        FsToolKind::ListDirectory => object(
            "List a bounded directory tree after exact approval of its resolved root.",
            json!({
                "path": bounded_string(4096),
                "maxDepth": { "type": "integer", "minimum": 1, "maximum": 8 },
                "maxEntries": { "type": "integer", "minimum": 1, "maximum": 500 }
            }),
            &["path", "maxDepth", "maxEntries"],
        ),
        FsToolKind::SearchText => object(
            "Search workspace text with ripgrep-style Rust regular expressions, ignore-file and hidden-file filtering, and bounded traversal/output. Matching is case-sensitive unless the expression uses an inline flag such as (?i).",
            json!({
                "path": bounded_string(4096),
                "query": bounded_string(1024),
                "maxDepth": { "type": "integer", "minimum": 1, "maximum": 8 },
                "maxResults": { "type": "integer", "minimum": 1, "maximum": 200 },
                "maxBytes": { "type": "integer", "minimum": 1, "maximum": 4194304 }
            }),
            &["path", "query", "maxDepth", "maxResults", "maxBytes"],
        ),
        FsToolKind::ReadFile => object(
            "Read one bounded UTF-8 page from a file up to 128 MiB. Start with offset 0, then continue from nextOffset while truncated is true.",
            json!({
                "path": bounded_string(4096),
                "offset": { "type": "integer", "minimum": 0, "maximum": 134217728 },
                "maxBytes": { "type": "integer", "minimum": 4, "maximum": 262144 }
            }),
            &["path", "offset", "maxBytes"],
        ),
        FsToolKind::ApplyPatch => object(
            "Create, rewrite, rename, and delete files. This is the only tool that writes files; prefer it over shell commands that would write. The patch must use the *** Begin Patch / *** End Patch envelope and Add File, Delete File, or Update File sections. Add File with no '+' lines creates an empty file. Paths may be relative to the workspace or absolute; a path outside the workspace is written through the same approval flow as every other mutation.",
            json!({
                "patch": { "type": "string", "minLength": 1, "maxLength": 262144 }
            }),
            &["patch"],
        ),
        FsToolKind::MovePath => object(
            "Move one resolved file or directory to a non-existing target after exact approval.",
            json!({
                "source": bounded_string(4096),
                "destination": bounded_string(4096)
            }),
            &["source", "destination"],
        ),
        FsToolKind::DeletePath => object(
            "Delete one resolved file or directory after exact approval; recursive intent must be explicit.",
            json!({
                "path": bounded_string(4096),
                "recursive": { "type": "boolean" }
            }),
            &["path", "recursive"],
        ),
    }
}

/// The Lark grammar of `apply_patch` for providers with freeform tools.
pub(crate) fn apply_patch_freeform() -> Result<FreeformToolSpec, ToolContractError> {
    Ok(FreeformToolSpec {
        name: ToolName::parse("apply_patch")?,
        description: "Use this tool to edit files with a Codex patch. This is a FREEFORM tool, so provide the patch directly without JSON.".to_string(),
        format: FreeformToolFormat {
            format_type: "grammar".to_string(),
            syntax: "lark".to_string(),
            definition: include_str!("apply_patch.lark").to_string(),
        },
    })
}

fn object(description: &'static str, properties: Value, required: &[&str]) -> FsToolSpec {
    FsToolSpec {
        description,
        input_schema: json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }),
    }
}

fn bounded_string(maximum: usize) -> Value {
    json!({ "type": "string", "minLength": 1, "maxLength": maximum })
}
