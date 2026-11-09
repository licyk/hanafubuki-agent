// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/worktree`: a menu (Codex's managed worktree picker) to continue this
//! conversation in a new worktree (a fork of it goes on there), to start a
//! new one there (the next message creates it, bound to the worktree), or
//! to browse the repository's worktrees: resume a worktree's session, copy
//! its path, or remove it after a confirmation (Codex's worktree browser).
//! A worktree is a checkout of the repository the working directory is in,
//! under the data directory's `worktrees/`; outside a repository, or
//! without the `worktree` plugin, the command says why there is none.

use hanafubuki_agent::AgentWorktree;

use super::cd::PROJECT_NOTE;
use super::confirm::{confirm, Confirmation};
use super::{send_session_call, send_step, CommandContext, CommandSpec};
use crate::agent_bridge::{SessionCall, SessionReply};
use crate::app_event::{AgentCommand, AppCommand, AppEvent, Op};
use crate::bottom_pane::{SelectionItem, SelectionViewParams};
use crate::history_cell;

/// How many characters of a commit a listed worktree shows.
const SHORT_HEAD: usize = 8;

pub(super) static WORKTREE: CommandSpec = CommandSpec {
    name: "worktree",
    aliases: &[],
    description: "continue or start a conversation in a new worktree",
    inline_args: false,
    during_task: false,
    needs_session: false,
    dispatch: |context, _args| context.session_call(SessionCall::WorktreeMenu),
};

pub(super) fn on_reply(context: &mut CommandContext<'_>, reply: SessionReply) {
    match reply {
        SessionReply::WorktreeMenu { conversation } => show_menu(context, conversation),
        SessionReply::WorktreeEntered {
            path, continued, ..
        } => {
            let message = if continued {
                format!(
                    "Continuing in a fork of this conversation, in the new worktree {}",
                    path.display()
                )
            } else {
                format!(
                    "Moved into the new worktree {}; your next message starts a conversation there",
                    path.display()
                )
            };
            context.add_to_history(history_cell::new_info_event(
                message,
                Some(PROJECT_NOTE.to_string()),
            ));
        }
        SessionReply::Worktrees(worktrees) => show_worktrees(context, worktrees),
        SessionReply::WorktreeRemoved(path) => context.add_to_history(
            history_cell::new_info_event(format!("Removed the worktree {}", path.display()), None),
        ),
        _ => {}
    }
}

fn show_menu(context: &mut CommandContext<'_>, conversation: bool) {
    let items = vec![
        SelectionItem {
            name: "Continue this conversation".to_string(),
            description: Some("Fork it into a new worktree and go on there".to_string()),
            is_disabled: !conversation,
            disabled_reason: (!conversation).then(|| "There is no conversation yet".to_string()),
            actions: vec![Box::new(|tx| {
                send_session_call(tx, enter(true, None));
            })],
            dismiss_on_select: true,
            ..Default::default()
        },
        SelectionItem {
            name: "Start a new conversation".to_string(),
            description: Some(
                "Move into a new worktree; your next message starts a conversation there"
                    .to_string(),
            ),
            actions: vec![Box::new(|tx| send_session_call(tx, enter(false, None)))],
            dismiss_on_select: true,
            ..Default::default()
        },
        SelectionItem {
            name: "Browse worktrees".to_string(),
            description: Some(
                "Resume a worktree's session, copy its path, or remove it".to_string(),
            ),
            actions: vec![Box::new(|tx| {
                send_session_call(tx, SessionCall::ListWorktrees)
            })],
            dismiss_on_select: true,
            ..Default::default()
        },
    ];
    context.show_selection_view(SelectionViewParams {
        title: Some("Worktrees".to_string()),
        subtitle: Some(
            "A worktree is a separate checkout of this repository, in the Agent's data directory."
                .to_string(),
        ),
        items,
        // Without a conversation, the first row cannot be chosen.
        initial_selected_idx: (!conversation).then_some(1),
        ..Default::default()
    });
}

/// A new worktree to move into: a fork of the session goes on there with
/// `continued`, else the next session starts there.
pub(super) fn enter(continued: bool, name: Option<String>) -> SessionCall {
    SessionCall::EnterWorktree { continued, name }
}

fn show_worktrees(context: &mut CommandContext<'_>, worktrees: Vec<AgentWorktree>) {
    if worktrees.is_empty() {
        return context.add_to_history(history_cell::new_info_event(
            "This repository has no worktrees of the Agent's yet".to_string(),
            None,
        ));
    }
    let items = worktrees
        .into_iter()
        .map(|worktree| {
            // The directory's name; its whole path is the next view's.
            let name = worktree.path.file_name().map_or_else(
                || worktree.path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            SelectionItem {
                search_value: Some(worktree.path.display().to_string()),
                name,
                description: Some(summary(&worktree)),
                actions: vec![Box::new(move |tx| {
                    let worktree = worktree.clone();
                    send_step(tx, &WORKTREE, move |context| {
                        show_actions(context, worktree)
                    })
                })],
                dismiss_on_select: true,
                ..Default::default()
            }
        })
        .collect();
    context.show_selection_view(SelectionViewParams {
        title: Some("Worktrees".to_string()),
        subtitle: Some(
            "Select a worktree to resume its session, copy its path, or remove it.".to_string(),
        ),
        items,
        is_searchable: true,
        search_placeholder: Some("Type to search worktrees".to_string()),
        ..Default::default()
    });
}

/// Its commit, its branch, and its session.
fn summary(worktree: &AgentWorktree) -> String {
    let head: String = worktree.head.chars().take(SHORT_HEAD).collect();
    let branch = worktree.branch.as_deref().unwrap_or("detached");
    match worktree.owner {
        Some(owner) => format!("{head} · {branch} · session {owner}"),
        None => format!("{head} · {branch} · no session"),
    }
}

fn show_actions(context: &mut CommandContext<'_>, worktree: AgentWorktree) {
    let mut items = Vec::new();
    if let Some(owner) = worktree.owner {
        items.push(SelectionItem {
            name: format!("Resume session {owner}"),
            actions: vec![Box::new(move |tx| {
                tx.send(AppEvent::Op(Op::Command(AgentCommand::Resume(owner))))
            })],
            dismiss_on_select: true,
            ..Default::default()
        });
    }
    let path = worktree.path.display().to_string();
    let copied = path.clone();
    items.push(SelectionItem {
        name: "Copy path".to_string(),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::Command(AppCommand::Copy(copied.clone())))
        })],
        dismiss_on_select: true,
        ..Default::default()
    });
    let removed = worktree.path.clone();
    items.push(SelectionItem {
        name: "Remove worktree".to_string(),
        description: Some("Delete its checkout, local changes and all".to_string()),
        actions: vec![Box::new(move |tx| {
            let removed = removed.clone();
            send_step(tx, &WORKTREE, move |context| {
                confirm_removal(context, removed)
            })
        })],
        dismiss_on_select: true,
        ..Default::default()
    });
    context.show_selection_view(SelectionViewParams {
        title: Some("Worktree".to_string()),
        subtitle: Some(path),
        items,
        ..Default::default()
    });
}

fn confirm_removal(context: &mut CommandContext<'_>, path: std::path::PathBuf) {
    confirm(
        context,
        Confirmation {
            title: "Remove this worktree?",
            subtitle: "Its checkout is deleted, local changes and all; its session stays.",
            no: "Cancel",
            yes: "Yes, remove",
            yes_description: "Remove the worktree now",
            call: SessionCall::RemoveWorktree(path),
        },
    )
}
