//! What an interface shows of a finished tool call, beside what the model
//! reads. Not model-visible: a presenter computes it from the call's
//! arguments and committed output whenever an interface asks.

use hanafubuki_agent_foundation::terminal_text::{StyleSpan, TextColor, TextStyle, TextStyles};
use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The colours and attributes of a command's plain output (`output` of
/// [`AgentToolDisplay::Command`]): runs of its bytes with their SGR style.
pub type AgentStyledText = TextStyles;
/// One run of an [`AgentStyledText`].
pub type AgentStyleSpan = StyleSpan;
/// A run's colours and attributes.
pub type AgentTextStyle = TextStyle;
/// A palette index (0–15: the terminal's 16 colours) or 24-bit colour.
pub type AgentTextColor = TextColor;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentToolDisplay {
    /// A command and what it printed (`exec_command`, `write_stdin`).
    Command {
        command: String,
        cwd: Option<String>,
        output: String,
        exit_code: Option<i32>,
        duration_millis: Option<u64>,
        truncated: bool,
        /// The styles `output` was printed in, when it had any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        styled: Option<AgentStyledText>,
    },
    Patch {
        preview: AgentPatchPreview,
    },
    FileRead {
        path: String,
        /// The first line's number, when the reader knows it.
        start_line: Option<u64>,
        text: String,
        truncated: bool,
    },
    Listing {
        path: String,
        entries: Vec<AgentListingEntry>,
        truncated: bool,
    },
    Search {
        query: String,
        path: Option<String>,
        matches: Vec<AgentSearchMatch>,
        truncated: bool,
    },
    WebSearch {
        query: String,
        results: Vec<AgentWebResult>,
    },
    WebPage {
        url: String,
        title: Option<String>,
        text: String,
        truncated: bool,
    },
    /// Paths moved (from, to) or deleted.
    Mutation {
        kind: AgentMutationKind,
        paths: Vec<String>,
    },
    Text {
        text: String,
    },
    /// A structured output no presenter read: its top-level keys.
    Fields {
        rows: Vec<(String, String)>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMutationKind {
    Moved,
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentListingEntryKind {
    File,
    Directory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentListingEntry {
    pub path: String,
    pub kind: AgentListingEntryKind,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSearchMatch {
    pub path: String,
    pub line: u64,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWebResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// A patch read as per-file diffs (`apply_patch`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPatchPreview {
    pub files: Vec<AgentPatchFile>,
    /// Why the patch could not be read as a diff; `files` is then empty.
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPatchFile {
    pub operation: AgentPatchOperation,
    /// The path as the patch wrote it.
    pub path: String,
    pub moved_to: Option<String>,
    pub lines: Vec<AgentPatchLine>,
    pub added: usize,
    pub removed: usize,
    /// Lines left out past the preview's cap.
    pub omitted_lines: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPatchOperation {
    Add,
    Update,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPatchLine {
    pub kind: AgentPatchLineKind,
    /// Without its diff marker.
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPatchLineKind {
    Added,
    Removed,
    Context,
    /// A `@@` chunk header, with any context text it carries.
    Hunk,
    /// A structural note from the patch, such as end-of-file.
    Marker,
}

/// What a handler gives an interface's display of its output beyond what
/// the model reads. The kernel commits it beside the output and never
/// sends it to the model; a reader of a committed one decodes it
/// tolerantly, a field it cannot read being absent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolDisplayMetadata {
    /// The styles of a command's output, for [`AgentToolDisplay::Command`].
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "tolerant"
    )]
    pub styled_output: Option<AgentStyledText>,
}

impl AgentToolDisplayMetadata {
    pub fn is_empty(&self) -> bool {
        self.styled_output.is_none()
    }
}

/// A committed field that does not decode is absent.
fn tolerant<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

impl AgentToolDisplay {
    /// `self` with what `metadata` adds: a command's styles, when they are
    /// the styles of its output (a bounded or otherwise changed output
    /// shows plain).
    pub fn with_metadata(mut self, metadata: &AgentToolDisplayMetadata) -> Self {
        if let Self::Command { output, styled, .. } = &mut self {
            if let Some(styles) = metadata
                .styled_output
                .as_ref()
                .filter(|styles| styles.fits(output))
            {
                *styled = Some(styles.clone());
            }
        }
        self
    }

    /// What a call without a presenter shows: its structured output's
    /// top-level keys, else its text.
    pub fn fallback(output: &ToolOutput) -> Self {
        match structured_output(output) {
            Some(Value::Object(object)) => Self::Fields {
                rows: object
                    .iter()
                    .map(|(key, value)| (key.clone(), field_value(value)))
                    .collect(),
            },
            Some(value) => Self::Text {
                text: field_value(value),
            },
            None => Self::Text {
                text: output_text(output),
            },
        }
    }
}

/// The structured value of `output`, which a presenter decodes: its
/// structured content, else its first structured item. Text items after it
/// (the argument notes) are not part of it.
pub fn structured_output(output: &ToolOutput) -> Option<&Value> {
    output.structured_content.as_ref().or_else(|| {
        output.content.iter().find_map(|item| match item {
            ToolOutputContent::Structured { value } => Some(value),
            _ => None,
        })
    })
}

/// The text items of `output`, one after another.
pub fn output_text(output: &ToolOutput) -> String {
    output
        .content
        .iter()
        .filter_map(|item| match item {
            ToolOutputContent::Text { text } => Some(text.as_str()),
            ToolOutputContent::EmbeddedResource {
                text: Some(text), ..
            } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn field_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(_) | Value::Number(_) => value.to_string(),
        Value::Array(items) => format!("{} items", items.len()),
        Value::Object(_) => "…".to_string(),
    }
}
