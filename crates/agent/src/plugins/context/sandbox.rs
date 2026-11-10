// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/prompts/templates/permissions/` (the sandbox mode, its network
// access and writable roots, and the escalation guidance of each approval
// policy) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: a world-state section rather than a
// developer message; the permission mode stands for the approval policy.

use hanafubuki_agent_contracts::{ContextSection, ContextSectionKind};
use hanafubuki_agent_foundation::paths::native_form;
use hanafubuki_agent_kernel::AgentPermissionMode;
use hanafubuki_agent_sandbox::SandboxPolicy;

/// The `sandbox` section: what the turn's commands may reach, and how one
/// may run outside the sandbox in `permission_mode`. A sandbox that does not
/// block the network (`blocks_network`, Windows' unelevated one) adds
/// `"networkEnforced": false` to a policy without network access.
pub(super) fn section(
    policy: &SandboxPolicy,
    blocks_network: bool,
    permission_mode: AgentPermissionMode,
) -> ContextSection {
    let mut content = serde_json::json!({
        "mode": policy.mode().as_str(),
        "escalation": escalation(policy, permission_mode),
    });
    match policy {
        SandboxPolicy::ReadOnly => content["networkAccess"] = false.into(),
        // A turn's policy is never a plugin's; it would read as its writes.
        SandboxPolicy::WorkspaceWrite {
            writable_roots,
            network_access,
        }
        | SandboxPolicy::Plugin {
            writable_roots,
            network_access,
            ..
        } => {
            content["networkAccess"] = (*network_access).into();
            content["writableRoots"] = writable_roots
                .iter()
                .map(|root| {
                    native_form(root)
                        .unwrap_or_else(|_| root.clone())
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
        }
        SandboxPolicy::FullAccess => content["networkAccess"] = true.into(),
    }
    if content["networkAccess"] == false && !blocks_network {
        content["networkEnforced"] = false.into();
    }
    ContextSection::from_json(ContextSectionKind::SANDBOX, &content)
}

fn escalation(policy: &SandboxPolicy, permission_mode: AgentPermissionMode) -> &'static str {
    if *policy == SandboxPolicy::FullAccess {
        return "There is no sandbox: commands run with the user's rights. Do not set sandbox_permissions.";
    }
    match permission_mode {
        AgentPermissionMode::Manual => {
            "A command that needs more than the sandbox allows runs outside it with sandbox_permissions \"require_escalated\" and a justification question; the user approves or denies it."
        }
        AgentPermissionMode::AutoReview => {
            "A command that needs more than the sandbox allows runs outside it with sandbox_permissions \"require_escalated\" and a justification question; automatic review decides it for compliance with the user's request. If it is denied, proceed only with a materially safer alternative, or tell the user the risk and ask for approval in your final message."
        }
        AgentPermissionMode::FullAccess => {
            "The permission mode is full access, which asks no one, so no command can run outside the sandbox: do not set sandbox_permissions, a command that does is refused."
        }
    }
}
