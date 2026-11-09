//! The read-only rule: only a review session is restricted, and in it only
//! a plain `git --no-pager` read through a POSIX shell or PowerShell runs.

use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalOrigin, ApprovalRiskClass, PermissionActionDetails,
};
use hanafubuki_agent_contracts::{AgentSessionOrigin, ApprovalRule, ApprovalRuleVerdict};
use hanafubuki_agent_execution::POWERSHELL_UTF8_OUTPUT_PREFIX;

use super::{ReviewIsReadOnly, READ_ONLY_REASON};

fn action(kind: ApprovalActionKind, argv: &[&str]) -> ApprovalAction {
    ApprovalAction::from_canonical_bytes(
        kind,
        ApprovalRiskClass::Critical,
        ApprovalOrigin::Model,
        "Run a command".to_string(),
        argv.join(" ").as_bytes(),
    )
    .unwrap()
    .with_details(PermissionActionDetails::Command {
        program: argv[0].to_string(),
        argv: argv.iter().map(|part| part.to_string()).collect(),
        cwd: "/work".to_string(),
    })
    .unwrap()
}

fn posix(script: &str) -> ApprovalAction {
    action(
        ApprovalActionKind::ShellExecution,
        &["/bin/bash", "-c", script],
    )
}

fn review(action: &ApprovalAction) -> Option<ApprovalRuleVerdict> {
    ReviewIsReadOnly.evaluate(Some(AgentSessionOrigin::Review), action)
}

fn denied() -> Option<ApprovalRuleVerdict> {
    Some(ApprovalRuleVerdict::Deny {
        reason: READ_ONLY_REASON.to_string(),
    })
}

#[test]
fn only_a_review_session_is_restricted() {
    let removal = posix("rm -rf src");
    for origin in [
        None,
        Some(AgentSessionOrigin::Side),
        Some(AgentSessionOrigin::Subagent),
    ] {
        assert_eq!(ReviewIsReadOnly.evaluate(origin, &removal), None);
    }
    assert_eq!(review(&removal), denied());
}

#[test]
fn a_plain_read_only_git_command_runs_in_a_review() {
    for script in [
        "git --no-pager diff",
        "git --no-pager diff 1f2e3d4c --stat",
        "git --no-pager log -n 20 --format=%h%x09%s",
        "git --no-pager show HEAD~1:src/lib.rs",
        "  git\t--no-pager status --short  ",
        "git --no-pager merge-base HEAD main",
    ] {
        assert_eq!(review(&posix(script)), None, "{script}");
    }
    assert_eq!(
        review(&action(
            ApprovalActionKind::ShellExecution,
            &["/bin/zsh", "-lc", "git --no-pager blame src/lib.rs"]
        )),
        None
    );
    let powershell = format!("{POWERSHELL_UTF8_OUTPUT_PREFIX}git --no-pager diff");
    assert_eq!(
        review(&action(
            ApprovalActionKind::ShellExecution,
            &[
                "pwsh.exe",
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &powershell
            ]
        )),
        None
    );
}

#[test]
fn anything_else_is_denied_in_a_review() {
    for script in [
        // A pager could wait on a terminal for input.
        "git diff",
        "git --no-pager push",
        "git --no-pager -c core.pager=sh diff",
        "git --no-pager diff --output=patch.diff",
        "git --no-pager diff --ext-diff",
        "git --no-pager diff; rm -rf src",
        "git --no-pager diff && touch x",
        "git --no-pager diff | tee x",
        "git --no-pager diff > x",
        "git --no-pager diff $(touch x)",
        "git --no-pager log '--format=%h'",
        "git --no-pager diff\nrm -rf src",
        "cat src/lib.rs",
        "GIT_EXTERNAL_DIFF=sh git --no-pager diff",
    ] {
        assert_eq!(review(&posix(script)), denied(), "{script}");
    }
    // `cmd` runs a `git.cmd` from the working directory first.
    assert_eq!(
        review(&action(
            ApprovalActionKind::ShellExecution,
            &["cmd.exe", "/D", "/S", "/C", "git --no-pager diff"]
        )),
        denied()
    );
    // A PowerShell script without the execution crate's prefix is not one
    // it built.
    assert_eq!(
        review(&action(
            ApprovalActionKind::ShellExecution,
            &[
                "pwsh.exe",
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "git --no-pager diff"
            ]
        )),
        denied()
    );
    assert_eq!(
        review(&action(
            ApprovalActionKind::FilePatch,
            &["git", "--no-pager", "diff"]
        )),
        denied()
    );
    let without_details = ApprovalAction::from_canonical_bytes(
        ApprovalActionKind::McpToolCall,
        ApprovalRiskClass::High,
        ApprovalOrigin::Model,
        "Call an MCP tool".to_string(),
        b"mcp",
    )
    .unwrap();
    assert_eq!(review(&without_details), denied());
}
