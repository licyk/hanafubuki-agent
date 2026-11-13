//! How an interface shows a tool call of the history: its subject and its
//! display, from the tool's presenter. Computed when asked, never stored
//! or sent as an event: a display can be as large as the output.

use hanafubuki_agent_contracts::{AgentToolDisplay, RegistryService};
use hanafubuki_agent_tool_contracts::ToolOutput;
use serde_json::Value;

use super::tool_execution::user_shell_command_display;
use super::*;

/// The history items a [`AgentService::tool_display`] read takes at once.
const TOOL_DISPLAY_PAGE: usize = 64;

/// What an interface shows of one tool call.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolCallDisplay {
    /// From the tool's presenter; `None` without one.
    pub subject: Option<String>,
    /// `None` while the call has no committed output.
    pub display: Option<AgentToolDisplay>,
}

impl AgentService {
    /// How the call `call_id` of the session's history is shown, read back
    /// from the session's latest items; `None` when the history has no
    /// such call.
    pub fn tool_display(
        &self,
        session_id: AgentSessionId,
        call_id: &str,
    ) -> Result<Option<AgentToolCallDisplay>, AgentError> {
        let mut output = None;
        let mut request = AgentTranscriptPageRequest {
            session_id,
            page_size: TOOL_DISPLAY_PAGE,
            cursor: None,
            direction: AgentHistorySortDirection::Descending,
            through_revision: None,
        };
        loop {
            let page = self.read_transcript_page(request.clone())?;
            for item in page.items {
                if let Some(display) = user_shell_command_display(&item.content, call_id) {
                    return Ok(Some(display));
                }
                match item.content {
                    ConversationItem::ToolOutput {
                        call_id: id,
                        output: committed,
                        ..
                    } if id == call_id => output = Some(committed),
                    ConversationItem::ToolCall {
                        call_id: id,
                        namespace,
                        name,
                        arguments,
                        ..
                    } if id == call_id => {
                        return Ok(Some(self.present_tool_call(
                            namespace.as_deref(),
                            &name,
                            &arguments,
                            output.as_ref(),
                        )));
                    }
                    _ => {}
                }
            }
            let Some(cursor) = page.older_cursor else {
                return Ok(None);
            };
            request.cursor = Some(cursor);
            request.through_revision = Some(page.through_revision);
        }
    }

    /// How a call with the model's `arguments` and its committed `output`
    /// is shown, the call named as history records it (`namespace`,
    /// `name`). The presenter reads the arguments normalized against the
    /// schema of the tool registered under that name now, as a call of it
    /// would receive them, and the model's own when none is.
    pub fn present_tool_call(
        &self,
        namespace: Option<&str>,
        name: &str,
        arguments: &Value,
        output: Option<&ToolOutput>,
    ) -> AgentToolCallDisplay {
        let tool = hanafubuki_agent_model_api::flattened_tool_call_name(namespace, name);
        let arguments = self.presented_arguments(&tool, arguments);
        let presenters = &self.inner.registries.tool_presenters;
        AgentToolCallDisplay {
            subject: presenters.subject(&tool, &arguments),
            display: output.map(|output| presenters.display(&tool, &arguments, output)),
        }
    }

    fn presented_arguments(&self, tool: &str, arguments: &Value) -> Value {
        let snapshot = self.inner.registries.tools.registry().snapshot();
        let schema = snapshot
            .items()
            .map(|handler| handler.descriptor())
            .find(|descriptor| descriptor.name.flattened_alias() == tool)
            .map(|descriptor| &descriptor.input_schema);
        match schema {
            Some(schema) if arguments.is_object() => {
                crate::tool_catalog::normalize_tool_arguments(schema, arguments)
            }
            _ => arguments.clone(),
        }
    }
}
