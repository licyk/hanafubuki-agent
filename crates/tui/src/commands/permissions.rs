// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/permissions [preset|mode]`: the session's permissions, from Codex's
//! presets, each a permission mode with a sandbox mode (the one in effect
//! marked), Full access after a confirmation; or, at once, the preset
//! `read-only` or `default`, or the permission mode `manual`, `auto-review`
//! or `full-access` alone, which keeps the sandbox. It lasts for the
//! session only (Codex's permissions popup).
//!
//! `/approve`: the session's latest actions automatic review denied (the
//! bridge keeps ten), searchable; the one chosen is approved for one retry,
//! which is still reviewed (Codex's auto-review denials popup).

use hanafubuki_agent::{AgentPermissionMode, AgentSandboxMode};

use super::confirm::{confirm, Confirmation};
use super::{send_session_call, send_step, CommandContext, CommandSpec};
use crate::agent_bridge::{DeniedAction, SessionCall, SessionReply};
use crate::bottom_pane::{SelectionItem, SelectionViewParams};
use crate::history_cell;

pub(super) static PERMISSIONS: CommandSpec = CommandSpec {
    name: "permissions",
    aliases: &[],
    description: "choose what the Agent may do without asking, and its sandbox, for this session",
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

/// A permission mode with a sandbox mode (Codex's approval presets, with
/// its Auto-review).
struct Preset {
    /// The argument that applies it; `None` where the name is the
    /// permission mode's, which applies the mode alone.
    argument: Option<&'static str>,
    label: &'static str,
    description: &'static str,
    mode: AgentPermissionMode,
    sandbox: AgentSandboxMode,
}

const PRESETS: [Preset; 4] = [
    Preset {
        argument: Some("read-only"),
        label: "Read only",
        description: "Commands can read files but not write them or use the network; you \
                      approve every operation",
        mode: AgentPermissionMode::Manual,
        sandbox: AgentSandboxMode::ReadOnly,
    },
    Preset {
        argument: Some("default"),
        label: "Default",
        description: "Commands can read files and edit the workspace, without the network; you \
                      approve every operation",
        mode: AgentPermissionMode::Manual,
        sandbox: AgentSandboxMode::WorkspaceWrite,
    },
    Preset {
        argument: None,
        label: "Auto-review",
        description: "As Default, but the session's model reviews each operation and decides",
        mode: AgentPermissionMode::AutoReview,
        sandbox: AgentSandboxMode::WorkspaceWrite,
    },
    Preset {
        argument: None,
        label: "Full access",
        description: "No sandbox, and every operation runs without asking",
        mode: AgentPermissionMode::FullAccess,
        sandbox: AgentSandboxMode::FullAccess,
    },
];

fn dispatch(context: &mut CommandContext<'_>, args: &str) {
    let args = args.trim();
    if args.is_empty() {
        return context.session_call(SessionCall::PermissionModes);
    }
    let name = args.to_lowercase().replace('_', "-");
    if let Some(preset) = PRESETS
        .iter()
        .find(|preset| preset.argument == Some(name.as_str()))
    {
        return context.session_call(SessionCall::SetPreset(preset.mode, preset.sandbox));
    }
    match parse_mode(&name) {
        Some(mode) => context.session_call(SessionCall::SetPermissionMode(mode)),
        None => context.add_to_history(history_cell::new_error_event(format!(
            "/permissions: no preset or mode `{args}`; use read-only, default, manual, \
             auto-review or full-access"
        ))),
    }
}

/// A mode as `--permission-mode` or `config.toml` spells it (lower case,
/// `-` for `_`).
fn parse_mode(text: &str) -> Option<AgentPermissionMode> {
    match text {
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
            sandbox,
            sandbox_available,
            sandbox_reason,
        } => show_presets(
            context,
            session.unwrap_or(configured),
            sandbox,
            sandbox_available,
            sandbox_reason,
        ),
        SessionReply::PermissionModeSet(mode) => {
            context.add_to_history(history_cell::new_info_event(
                format!("Permissions set to {} for this session", label(mode)),
                Some(
                    "The configured permissionMode stays as it is; a running turn keeps its mode."
                        .to_string(),
                ),
            ))
        }
        SessionReply::PresetSet(mode, sandbox) => {
            let preset = PRESETS
                .iter()
                .find(|preset| (preset.mode, preset.sandbox) == (mode, sandbox))
                .map_or("the chosen preset", |preset| preset.label);
            context.add_to_history(history_cell::new_info_event(
                format!("Permissions set to {preset} for this session"),
                Some(format!(
                    "Permission mode {}, sandbox {}; the configuration stays as it is, and a \
                     running turn keeps its own.",
                    label(mode),
                    sandbox.as_str()
                )),
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

fn show_presets(
    context: &mut CommandContext<'_>,
    mode: AgentPermissionMode,
    sandbox: AgentSandboxMode,
    sandbox_available: bool,
    sandbox_reason: Option<String>,
) {
    let mut items: Vec<_> = PRESETS
        .iter()
        .map(|preset| {
            let (preset_mode, preset_sandbox) = (preset.mode, preset.sandbox);
            // A sandbox this machine cannot give would refuse every command.
            let unavailable = !sandbox_available && preset_sandbox != AgentSandboxMode::FullAccess;
            SelectionItem {
                name: preset.label.to_string(),
                description: Some(preset.description.to_string()),
                is_current: (preset_mode, preset_sandbox) == (mode, sandbox),
                is_disabled: unavailable,
                disabled_reason: unavailable.then(|| {
                    sandbox_reason
                        .clone()
                        .unwrap_or_else(|| "The sandbox is unavailable on this machine".to_string())
                }),
                actions: vec![Box::new(move |tx| match preset_mode {
                    AgentPermissionMode::FullAccess => {
                        send_step(tx, &PERMISSIONS, confirm_full_access)
                    }
                    mode => send_session_call(tx, SessionCall::SetPreset(mode, preset_sandbox)),
                })],
                dismiss_on_select: true,
                ..Default::default()
            }
        })
        .collect();
    if cfg!(windows) && !sandbox_available {
        items.push(SelectionItem {
            name: "Set up Windows sandbox".into(),
            description: Some(
                "Windows asks for administrator consent once; enables sandboxed presets".into(),
            ),
            actions: vec![Box::new(|tx| {
                send_session_call(tx, SessionCall::SetupWindowsSandbox)
            })],
            dismiss_on_select: true,
            ..Default::default()
        });
    }
    context.show_selection_view(SelectionViewParams {
        title: Some("Select Permissions".to_string()),
        subtitle: Some(
            "For this session only; the configured permissionMode and sandboxMode stay as they \
             are."
                .to_string(),
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
            call: SessionCall::SetPreset(
                AgentPermissionMode::FullAccess,
                AgentSandboxMode::FullAccess,
            ),
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
