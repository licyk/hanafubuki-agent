// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A call shown by its name: while it runs, when it has no display, and
//! for text or fields (an MCP or plugin tool's, the fallback's), after
//! Codex's MCP cell: `Called` and the tool, then the first lines; expanded,
//! all of them. A `compress` call is folded: its ranges and topics in the
//! header, the first line of its receipt below; expanded, the rest of the
//! receipt and the summaries.

use ratatui::style::Stylize;
use ratatui::text::{Line, Span};

use super::{clipped, wrapped, Extent, ToolCall, View, COMPRESS, EXEC_COMMAND, PREVIEW_LINES};
use crate::history_cell::ActivityDisclosure;
use crate::render::highlight::highlight_code_to_lines;

/// `Running` and the command (its clock follows, see `frame`), or the
/// tool and its subject.
pub(super) fn running(call: &ToolCall) -> View {
    if let Some(title) = operation_title(call, true) {
        return View::title(title, false);
    }
    if call.name == EXEC_COMMAND && call.subject.is_some() {
        return command_title(call, "Running", false);
    }
    let mut title = vec!["Running".bold(), " ".into()];
    title.extend(named(call));
    View::title(title, false)
}

/// `Called` (or `Failed`) and the tool, with nothing more to show; a
/// command `Ran` (the person's, `You ran`) and its command line, as it ran.
pub(super) fn called(call: &ToolCall, failed: bool) -> View {
    if call.name == EXEC_COMMAND && call.subject.is_some() {
        let verb = if call.by_user { "You ran" } else { "Ran" };
        return command_title(call, verb, failed);
    }
    View::title(called_title(call, failed), failed)
}

fn command_title(call: &ToolCall, verb: &str, failed: bool) -> View {
    let script = call.subject_or("");
    let lines = highlight_code_to_lines(&script, "bash");
    View::title(super::command::header(verb, &lines).0, failed)
}

pub(super) fn view(call: &ToolCall, failed: bool, lines: Vec<String>, extent: Extent) -> View {
    let summary = matches!(
        call.name.as_str(),
        "request_user_input"
            | "request_user_input_async"
            | "create_goal"
            | "get_goal"
            | "update_goal"
    );
    let shown = if call.name == COMPRESS {
        1
    } else if summary {
        5
    } else {
        PREVIEW_LINES
    };
    let mut preview = Vec::new();
    let mut cut = false;
    for line in lines.iter().take(shown) {
        if summary {
            preview.extend(wrapped(Line::from(line.clone()).dim(), extent.columns, ""));
            continue;
        }
        let (row, was_cut) = clipped(Line::from(line.clone()).dim(), extent.columns);
        cut |= was_cut;
        preview.push(row);
    }
    let mut details = Vec::new();
    if extent.expanded {
        for line in lines.iter().cloned() {
            details.extend(wrapped(Line::from(line).dim(), extent.columns, ""));
        }
    }
    let hidden = match lines.len().checked_sub(shown) {
        Some(more) if more > 0 && call.name == COMPRESS => Some(ActivityDisclosure::Generic),
        Some(more) if more > 0 => Some(ActivityDisclosure::OutputLines(more)),
        _ => cut.then_some(ActivityDisclosure::Generic),
    };
    View {
        failed,
        title: called_title(call, failed),
        preview,
        details,
        hidden,
    }
}

fn called_title(call: &ToolCall, failed: bool) -> Vec<Span<'static>> {
    if failed {
        let title = match call.name.as_str() {
            "ask_advisor" => Some("Advisor consultation failed"),
            "record_advisor_outcome" => Some("Advisor outcome recording failed"),
            _ => None,
        };
        if let Some(title) = title {
            return vec![title.bold()];
        }
    } else if let Some(title) = operation_title(call, false) {
        return title;
    }
    let mut title = vec![if failed { "Failed" } else { "Called" }.bold(), " ".into()];
    title.extend(named(call));
    title
}

/// `name: subject`.
fn named(call: &ToolCall) -> Vec<Span<'static>> {
    let mut spans = vec![call.name.clone().cyan()];
    if call.subject.is_some() {
        spans.push(": ".dim());
        spans.push(call.subject_or("").into());
    }
    spans
}

fn operation_title(call: &ToolCall, running: bool) -> Option<Vec<Span<'static>>> {
    let verb = match (call.name.as_str(), running) {
        ("request_user_input" | "request_user_input_async", true) => "Asking user",
        ("request_user_input" | "request_user_input_async", false) => "Asked user",
        ("create_goal", true) => "Creating goal",
        ("create_goal", false) => "Created goal",
        ("get_goal", true) => "Reading goal",
        ("get_goal", false) => "Read goal",
        ("update_goal", true) => "Updating goal",
        ("update_goal", false) => "Updated goal",
        ("ask_advisor", true) => "Consulting",
        ("ask_advisor", false) => "Consulted",
        ("record_advisor_outcome", true) => "Recording Advisor outcome",
        ("record_advisor_outcome", false) => "Recorded Advisor outcome",
        ("spawn_agent", true) => "Starting agent",
        ("spawn_agent", false) => "Started agent",
        ("send_message", true) => "Sending message to",
        ("send_message", false) => "Sent message to",
        ("followup_task", true) => "Assigning task to",
        ("followup_task", false) => "Assigned task to",
        ("interrupt_agent", true) => "Interrupting agent",
        ("interrupt_agent", false) => "Interrupt requested for",
        ("wait_agent", true) => "Waiting for agents",
        ("wait_agent", false) => "Finished waiting",
        ("list_agents", true) => "Listing agents",
        ("list_agents", false) => "Agents",
        ("write_stdin", true) => "Interacting with terminal",
        ("write_stdin", false) => "Interacted with terminal",
        ("resize_execution", true) => "Resizing terminal",
        ("resize_execution", false) => "Resized terminal",
        ("list_agent_memories", true) => "Listing memories",
        ("list_agent_memories", false) => "Listed memories",
        ("search_agent_memories", true) => "Searching memories",
        ("search_agent_memories", false) => "Searched memories",
        ("read_agent_memory", true) => "Reading memory",
        ("read_agent_memory", false) => "Read memory",
        ("append_agent_memory_note", true) => "Queuing memory note",
        ("append_agent_memory_note", false) => "Queued memory note",
        ("read_thread", true) => "Reading conversation",
        ("read_thread", false) => "Read conversation",
        ("compress", true) => "Compressing context",
        ("compress", false) => "Compressed context",
        ("decompress", true) => "Restoring context",
        ("decompress", false) => "Restored context",
        ("search_context", true) => "Searching context",
        ("search_context", false) => "Searched context",
        ("new_context", true) => "Starting new context",
        ("new_context", false) => "Started new context",
        ("acp_status", true) => "Reading context status",
        ("acp_status", false) => "Read context status",
        ("acp_cache", true) => "Managing context cache",
        ("acp_cache", false) => "Managed context cache",
        ("absorb", true) => "Absorbing content",
        ("absorb", false) => "Absorbed content",
        ("acp_retrieve", true) => "Retrieving stored content",
        ("acp_retrieve", false) => "Retrieved stored content",
        ("acp_rule", true) => "Managing context rules",
        ("acp_rule", false) => "Managed context rules",
        ("update_plan", true) => "Updating plan",
        ("update_plan", false) => "Updated plan",
        ("apply_patch", true) => "Editing files",
        ("move_path", true) => "Moving path",
        ("delete_path", true) => "Deleting path",
        _ => return None,
    };
    let mut title = vec![verb.bold()];
    if call.subject.is_some() {
        title.extend([" ".into(), call.subject_or("").cyan()]);
    }
    Some(title)
}
