// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/copy`: a picker of what to copy from the session's last answer (read
//! from its history): the whole answer as Markdown, each code block, each
//! quote (Codex's `chatwidget/copy_picker.rs`). In fullscreen it also
//! offers select mode in the transcript, to copy any part of it.

mod targets;

use super::{CommandContext, CommandSpec};
use crate::agent_bridge::SessionCall;
use crate::app_event::{AppCommand, AppEvent};
use crate::bottom_pane::{SelectionItem, SelectionViewParams};
use crate::history_cell;
use crate::text_formatting::truncate_text;
use targets::{extract_copy_targets, CopyTarget};

/// The longest preview of a choice's first line.
const PREVIEW_GRAPHEMES: usize = 72;

pub(super) static COPY: CommandSpec = CommandSpec {
    name: "copy",
    aliases: &[],
    description: "copy the last answer or part of it",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _args| context.session_call(SessionCall::LastAnswer),
};

pub(super) fn on_reply(context: &mut CommandContext<'_>, answer: Option<String>) {
    let mut items: Vec<SelectionItem> = answer
        .as_deref()
        .map(copy_choices)
        .unwrap_or_default()
        .into_iter()
        .map(|(label, text)| {
            let description = text
                .lines()
                .find(|line| !line.trim().is_empty())
                .map(|line| truncate_text(line.trim(), PREVIEW_GRAPHEMES));
            SelectionItem {
                name: label,
                description,
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::Command(AppCommand::Copy(text.clone())))
                })],
                dismiss_on_select: true,
                ..Default::default()
            }
        })
        .collect();
    if context.owned_transcript() {
        items.push(SelectionItem {
            name: "Select in the transcript".to_string(),
            description: Some("Choose any part of the conversation with the keys".to_string()),
            actions: vec![Box::new(|tx| {
                tx.send(AppEvent::Command(AppCommand::SelectTranscript))
            })],
            dismiss_on_select: true,
            ..Default::default()
        });
    }
    if items.is_empty() {
        let message = "There is no answer to copy yet".to_string();
        return context.add_to_history(history_cell::new_info_event(message, None));
    }
    context.show_selection_view(SelectionViewParams {
        title: Some("Copy to clipboard".to_string()),
        items,
        ..Default::default()
    });
}

/// What the picker offers from `answer`, each with its label: the whole
/// answer first, then its code blocks and quotes in order.
fn copy_choices(answer: &str) -> Vec<(String, String)> {
    if answer.trim().is_empty() {
        return Vec::new();
    }
    let mut choices = vec![("Whole answer".to_string(), answer.to_string())];
    choices.extend(
        extract_copy_targets(answer)
            .into_iter()
            .map(|target| match target {
                CopyTarget::Code { language, content } => {
                    let label = language.map_or_else(
                        || "Code block".to_string(),
                        |language| format!("{language} code"),
                    );
                    (label, content)
                }
                CopyTarget::Quote(content) => ("Quote".to_string(), content),
            }),
    );
    choices
}

#[cfg(test)]
mod tests;
