// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/permissions [manual|auto-review|full-access]`: the session's
//! permission mode, from a list of the three (the one in effect marked:
//! the session's, else the configured `permissionMode`), Full access after
//! a confirmation; or the mode named, at once. It lasts for the session
//! only (Codex's permissions popup).
//!
//! `/approve`: the session's latest actions automatic review denied (the
//! bridge keeps ten), searchable; the one chosen is approved for one retry,
//! which is still reviewed (Codex's auto-review denials popup).

use hanafubuki_agent::AgentPermissionMode;

use super::confirm::{confirm, Confirmation};
use super::{send_session_call, send_step, CommandContext, CommandSpec};
use crate::agent_bridge::{DeniedAction, SessionCall, SessionReply};
use crate::bottom_pane::{SelectionItem, SelectionViewParams};
use crate::history_cell;

pub(super) static PERMISSIONS: CommandSpec = CommandSpec {
    name: "permissions",
    aliases: &[],
    description: "choose what the Agent may do without asking, for this session",
    inline_args: true,
    during_task: true,
    needs_session: true,
    dispatch,
};

pub(super) static APPROVE: CommandSpec = CommandSpec {
    name: "approve",
    aliases: &[],
    description: "approve one retry of an action automatic review denied",
    inline_args: false,
    during_task: true,
    needs_session: false,
    dispatch: |context, _args| context.session_call(SessionCall::Denials),
};

const MODES: [AgentPermissionMode; 3] = [
    AgentPermissionMode::Manual,
    AgentPermissionMode::AutoReview,
    AgentPermissionMode::FullAccess,
];

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    let args = args.trim();
    if args.is_empty() {
        return context.session_call(SessionCall::PermissionModes);
    }
    match parse_mode(args) {
        Some(mode) => context.session_call(SessionCall::SetPermissionMode(mode)),
        None => context.add_to_history(history_cell::new_error_event(format!(
            "/permissions: no mode `{args}`; use manual, auto-review or full-access"
        ))),
    }
}

/// A mode as `--permission-mode` or `config.toml` spells it.
fn parse_mode(text: &str) -> Option<AgentPermissionMode> {
    match text.to_lowercase().replace('_', "-").as_str() {
        "manual" => Some(AgentPermissionMode::Manual),
        "auto-review" => Some(AgentPermissionMode::AutoReview),
        "full-access" => Some(AgentPermissionMode::FullAccess),
        _ => None,
    }
}

pub(super) fn on_reply(context: &mut CommandContext<'_>, reply: SessionReply) {
    match reply {
        SessionReply::PermissionModes {
            session,
            configured,
        } => show_modes(context, session.unwrap_or(configured)),
        SessionReply::PermissionModeSet(mode) => {
            context.add_to_history(history_cell::new_info_event(
                format!("Permissions set to {} for this session", label(mode)),
                Some(
                    "The configured permissionMode stays as it is; a running turn keeps its mode."
                        .to_string(),
                ),
            ))
        }
        SessionReply::Denials(denials) => show_denials(context, denials),
        SessionReply::Approved(summary) => context.add_to_history(history_cell::new_info_event(
            format!("Approved one retry of: {summary}"),
            Some(
                "The model sees the approval from its next request; the retry is still reviewed."
                    .to_string(),
            ),
        )),
        _ => {}
    }
}

fn show_modes(context: &mut CommandContext<'_>, current: AgentPermissionMode) {
    let items = MODES
        .into_iter()
        .map(|mode| SelectionItem {
            name: label(mode).to_string(),
            description: Some(description(mode).to_string()),
            is_current: mode == current,
            actions: vec![Box::new(move |tx| match mode {
                AgentPermissionMode::FullAccess => send_step(tx, &PERMISSIONS, confirm_full_access),
                mode => send_session_call(tx, SessionCall::SetPermissionMode(mode)),
            })],
            dismiss_on_select: true,
            ..Default::default()
        })
        .collect();
    context.show_selection_view(SelectionViewParams {
        title: Some("Select Permissions".to_string()),
        subtitle: Some(
            "For this session only; the configured permissionMode stays as it is.".to_string(),
        ),
        items,
        ..Default::default()
    });
}

fn confirm_full_access(context: &mut CommandContext<'_>) {
    confirm(
        context,
        Confirmation {
            title: "Enable full access?",
            subtitle: "Every operation that needs an approval then runs without asking, with \
                       your user's rights: there is no sandbox.",
            no: "Cancel",
            yes: "Yes, continue anyway",
            yes_description: "Apply full access for this session",
            call: SessionCall::SetPermissionMode(AgentPermissionMode::FullAccess),
        },
    )
}

fn show_denials(context: &mut CommandContext<'_>, denials: Vec<DeniedAction>) {
    if denials.is_empty() {
        return context.add_to_history(history_cell::new_info_event(
            "No denied actions to approve.".to_string(),
            Some(
                "They are kept when automatic review denies an action in this session.".to_string(),
            ),
        ));
    }
    let items = denials
        .into_iter()
        .map(|denial| {
            let rationale = denial
                .rationale
                .unwrap_or_else(|| "The review gave no rationale".to_string());
            let review_id = denial.review_id;
            SelectionItem {
                search_value: Some(format!("{} {rationale}", denial.summary)),
                name: denial.summary,
                description: Some(rationale),
                actions: vec![Box::new(move |tx| {
                    send_session_call(tx, SessionCall::ApproveDenial(review_id.clone()))
                })],
                dismiss_on_select: true,
                ..Default::default()
            }
        })
        .collect();
    context.show_selection_view(SelectionViewParams {
        title: Some("Auto-review Denials".to_string()),
        subtitle: Some(
            "Select a denied action to approve; it applies to the next turn, and the retry is \
             still reviewed."
                .to_string(),
        ),
        items,
        is_searchable: true,
        search_placeholder: Some("Type to search denials".to_string()),
        ..Default::default()
    });
}

fn label(mode: AgentPermissionMode) -> &'static str {
    match mode {
        AgentPermissionMode::Manual => "Manual",
        AgentPermissionMode::AutoReview => "Auto-review",
        AgentPermissionMode::FullAccess => "Full access",
    }
}

fn description(mode: AgentPermissionMode) -> &'static str {
    match mode {
        AgentPermissionMode::Manual => "Every operation that needs an approval asks you",
        AgentPermissionMode::AutoReview => {
            "The session's model reviews each such operation and decides"
        }
        AgentPermissionMode::FullAccess => "Every such operation runs without asking",
    }
}
