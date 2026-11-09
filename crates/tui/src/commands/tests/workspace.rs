//! `/permissions`, `/approve`, `/cd`, `/worktree` and `/fork`'s worktree
//! choice: what each asks of the Agent for its arguments, and what their
//! views (and their later steps) do with the replies.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use hanafubuki_agent::{AgentPermissionMode, AgentSandboxMode, AgentSessionId, AgentWorktree};
use ratatui::crossterm::event::KeyCode;

use super::session::{call, chat, key, run, screen, sent, to_sent, Sent};
use super::*;
use crate::agent_bridge::{DeniedAction, TurnUpdate};
use crate::app_event::AppCommand;
use crate::chatwidget::ChatWidget;

/// What the chat sent, the commands' later steps run as they come.
fn sent_after_steps(chat: &mut ChatWidget, events: &Receiver<AppEvent>) -> Vec<Sent> {
    let mut sent = Vec::new();
    while let Ok(event) = events.try_recv() {
        match event {
            AppEvent::CommandStep(step) => chat.run_command_step(step),
            event => sent.extend(to_sent(event)),
        }
    }
    sent
}

fn reply(chat: &mut ChatWidget, reply: SessionReply) {
    chat.handle_turn_update(TurnUpdate::Session(reply));
}

fn cells(sent: Vec<Sent>) -> String {
    sent.into_iter()
        .filter_map(|sent| match sent {
            Sent::Cell(text) => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn permissions_with_a_mode_sets_it_and_without_one_lists_the_presets() {
    let (mut chat, events) = chat(false);
    for (line, mode) in [
        ("permissions manual", AgentPermissionMode::Manual),
        ("permissions auto-review", AgentPermissionMode::AutoReview),
        ("permissions auto_review", AgentPermissionMode::AutoReview),
        ("permissions Full-Access", AgentPermissionMode::FullAccess),
    ] {
        assert_eq!(
            run(&mut chat, &events, line),
            [
                Sent::Op(Op::EnsureSession),
                call(SessionCall::SetPermissionMode(mode))
            ],
            "/{line}"
        );
    }
    // The presets whose names are not a permission mode's set both modes.
    for (line, mode, sandbox) in [
        (
            "permissions read-only",
            AgentPermissionMode::Manual,
            AgentSandboxMode::ReadOnly,
        ),
        (
            "permissions Default",
            AgentPermissionMode::Manual,
            AgentSandboxMode::WorkspaceWrite,
        ),
    ] {
        assert_eq!(
            run(&mut chat, &events, line),
            [
                Sent::Op(Op::EnsureSession),
                call(SessionCall::SetPreset(mode, sandbox))
            ],
            "/{line}"
        );
    }
    let unknown = run(&mut chat, &events, "permissions sometimes");
    assert_eq!(unknown[0], Sent::Op(Op::EnsureSession));
    assert!(cells(unknown).contains(
        "no preset or mode `sometimes`; use read-only, default, manual, auto-review or full-access"
    ));

    assert_eq!(
        run(&mut chat, &events, "permissions"),
        [
            Sent::Op(Op::EnsureSession),
            call(SessionCall::PermissionModes)
        ]
    );
    reply(
        &mut chat,
        SessionReply::PermissionModes {
            session: None,
            configured: AgentPermissionMode::AutoReview,
            sandbox: AgentSandboxMode::WorkspaceWrite,
            sandbox_available: true,
        },
    );
    let shown = screen(&chat, 100);
    for preset in [
        "Read only",
        "Default",
        "Auto-review",
        "Full access",
        "For this session only",
    ] {
        assert!(shown.contains(preset), "{preset}: {shown}");
    }
    // The configured mode and the sandbox in effect make the preset that is
    // selected.
    key(&mut chat, KeyCode::Enter);
    assert_eq!(
        sent_after_steps(&mut chat, &events),
        [call(SessionCall::SetPreset(
            AgentPermissionMode::AutoReview,
            AgentSandboxMode::WorkspaceWrite
        ))]
    );

    reply(
        &mut chat,
        SessionReply::PresetSet(
            AgentPermissionMode::AutoReview,
            AgentSandboxMode::WorkspaceWrite,
        ),
    );
    let shown = cells(sent(&events));
    assert!(shown.contains("Permissions set to Auto-review for this session"));
    assert!(shown.contains("sandbox workspace-write"), "{shown}");
    reply(
        &mut chat,
        SessionReply::PermissionModeSet(AgentPermissionMode::Manual),
    );
    assert!(cells(sent(&events)).contains("Permissions set to Manual for this session"));
}

/// Where this machine has no sandbox, the presets that need one cannot be
/// chosen: their commands would all be refused.
#[test]
fn the_sandboxed_presets_are_disabled_without_a_sandbox() {
    let (mut chat, events) = chat(false);
    run(&mut chat, &events, "permissions");
    reply(
        &mut chat,
        SessionReply::PermissionModes {
            session: None,
            configured: AgentPermissionMode::Manual,
            sandbox: AgentSandboxMode::FullAccess,
            sandbox_available: false,
        },
    );
    assert!(screen(&chat, 100).contains("The sandbox is unavailable on this machine"));
    key(&mut chat, KeyCode::Enter);
    assert_eq!(sent_after_steps(&mut chat, &events), []);
}

#[test]
fn full_access_from_the_list_asks_first_and_cancel_is_selected() {
    let (mut chat, events) = chat(false);
    for confirmed in [false, true] {
        run(&mut chat, &events, "permissions");
        reply(
            &mut chat,
            SessionReply::PermissionModes {
                session: Some(AgentPermissionMode::Manual),
                configured: AgentPermissionMode::AutoReview,
                sandbox: AgentSandboxMode::WorkspaceWrite,
                sandbox_available: true,
            },
        );
        // The session's own mode and the sandbox make Default the preset in
        // effect: Auto-review, then Full access, come after it.
        key(&mut chat, KeyCode::Down);
        key(&mut chat, KeyCode::Down);
        key(&mut chat, KeyCode::Enter);
        assert_eq!(sent_after_steps(&mut chat, &events), []);
        assert!(screen(&chat, 100).contains("Enable full access?"));
        if confirmed {
            key(&mut chat, KeyCode::Down);
        }
        key(&mut chat, KeyCode::Enter);
        let expected = if confirmed {
            vec![call(SessionCall::SetPreset(
                AgentPermissionMode::FullAccess,
                AgentSandboxMode::FullAccess,
            ))]
        } else {
            Vec::new()
        };
        assert_eq!(sent_after_steps(&mut chat, &events), expected);
    }
}

#[test]
fn approve_lists_the_denials_and_approves_the_one_chosen() {
    let (mut chat, events) = chat(false);
    assert_eq!(
        run(&mut chat, &events, "approve"),
        [call(SessionCall::Denials)]
    );
    reply(&mut chat, SessionReply::Denials(Vec::new()));
    assert!(cells(sent(&events)).contains("No denied actions to approve."));

    run(&mut chat, &events, "approve");
    reply(
        &mut chat,
        SessionReply::Denials(vec![
            DeniedAction {
                review_id: "review-2".to_string(),
                summary: "rm -rf build".to_string(),
                rationale: Some("The user did not ask for this.".to_string()),
            },
            DeniedAction {
                review_id: "review-1".to_string(),
                summary: "curl example.com".to_string(),
                rationale: None,
            },
        ]),
    );
    let shown = screen(&chat, 120);
    for text in [
        "rm -rf build",
        "The user did not ask for this.",
        "curl example.com",
        "the retry is still reviewed",
    ] {
        assert!(shown.contains(text), "{text}: {shown}");
    }
    for character in "curl".chars() {
        key(&mut chat, KeyCode::Char(character));
    }
    key(&mut chat, KeyCode::Enter);
    assert_eq!(
        sent(&events),
        [call(SessionCall::ApproveDenial("review-1".to_string()))]
    );
    reply(
        &mut chat,
        SessionReply::Approved("curl example.com".to_string()),
    );
    assert!(cells(sent(&events)).contains("Approved one retry of: curl example.com"));
}

#[test]
fn cd_moves_to_the_path_typed_or_home_and_reports_the_new_roots() {
    let (mut chat, events) = chat(false);
    assert_eq!(
        run(&mut chat, &events, "cd ../other"),
        [call(SessionCall::ChangeDirectory(PathBuf::from(
            "../other"
        )))]
    );
    let home = std::env::home_dir().expect("the tests run with a home directory");
    assert_eq!(
        run(&mut chat, &events, "cd"),
        [call(SessionCall::ChangeDirectory(home.clone()))]
    );
    assert_eq!(
        run(&mut chat, &events, "cd ~/src"),
        [call(SessionCall::ChangeDirectory(home.join("src")))]
    );
    assert_eq!(cd::expand_home("~user"), Some(PathBuf::from("~user")));

    reply(
        &mut chat,
        SessionReply::DirectoryChanged(vec![PathBuf::from("/other"), PathBuf::from("/data")]),
    );
    let reported = cells(sent(&events));
    assert!(
        reported.contains("Working directory changed to /other"),
        "{reported}"
    );
    assert!(
        reported.contains("stays the one read at start"),
        "{reported}"
    );
    assert!(
        reported.contains("Also in the workspace: /data"),
        "{reported}"
    );
}

#[test]
fn worktree_menu_continues_starts_or_browses() {
    let (mut chat, events) = chat(false);
    assert_eq!(
        run(&mut chat, &events, "worktree"),
        [call(SessionCall::WorktreeMenu)]
    );
    for (downs, expected) in [
        (0, worktree::enter(true, None)),
        (1, worktree::enter(false, None)),
        (2, SessionCall::ListWorktrees),
    ] {
        reply(&mut chat, SessionReply::WorktreeMenu { conversation: true });
        for _ in 0..downs {
            key(&mut chat, KeyCode::Down);
        }
        key(&mut chat, KeyCode::Enter);
        assert_eq!(sent(&events), [call(expected)]);
    }

    // With no conversation yet, the first row cannot be chosen.
    reply(
        &mut chat,
        SessionReply::WorktreeMenu {
            conversation: false,
        },
    );
    let shown = screen(&chat, 200);
    assert!(
        shown.contains("(disabled: There is no conversation yet)"),
        "{shown}"
    );
    key(&mut chat, KeyCode::Enter);
    assert_eq!(sent(&events), [call(worktree::enter(false, None))]);

    reply(
        &mut chat,
        SessionReply::Failed {
            command: "worktree",
            error: "worktrees are unavailable: it is turned off".to_string(),
        },
    );
    assert_eq!(
        sent(&events),
        [Sent::Cell(
            "■ /worktree: worktrees are unavailable: it is turned off".to_string()
        )]
    );
}

#[test]
fn a_browsed_worktree_resumes_its_session_copies_its_path_or_is_removed() {
    let worktree = AgentWorktree {
        path: PathBuf::from("/data/worktrees/project-0123abcd"),
        head: "0123456789abcdef".to_string(),
        branch: None,
        owner: Some(AgentSessionId::new(7)),
    };
    let (mut chat, events) = chat(false);
    let browse = |chat: &mut ChatWidget, events: &Receiver<AppEvent>| {
        reply(chat, SessionReply::Worktrees(vec![worktree.clone()]));
        let shown = screen(chat, 120);
        assert!(shown.contains("01234567 · detached · session 7"), "{shown}");
        key(chat, KeyCode::Enter);
        assert_eq!(sent_after_steps(chat, events), []);
        assert!(screen(chat, 120).contains("Resume session 7"));
    };

    browse(&mut chat, &events);
    key(&mut chat, KeyCode::Enter);
    assert_eq!(
        sent(&events),
        [Sent::Op(Op::Command(AgentCommand::Resume(
            AgentSessionId::new(7)
        )))]
    );

    browse(&mut chat, &events);
    key(&mut chat, KeyCode::Down);
    key(&mut chat, KeyCode::Enter);
    assert_eq!(
        sent(&events),
        [Sent::App(AppCommand::Copy(
            "/data/worktrees/project-0123abcd".to_string()
        ))]
    );

    browse(&mut chat, &events);
    key(&mut chat, KeyCode::Down);
    key(&mut chat, KeyCode::Down);
    key(&mut chat, KeyCode::Enter);
    assert_eq!(sent_after_steps(&mut chat, &events), []);
    assert!(screen(&chat, 120).contains("Remove this worktree?"));
    key(&mut chat, KeyCode::Down);
    key(&mut chat, KeyCode::Enter);
    assert_eq!(
        sent(&events),
        [call(SessionCall::RemoveWorktree(worktree.path.clone()))]
    );

    reply(&mut chat, SessionReply::Worktrees(Vec::new()));
    assert!(cells(sent(&events)).contains("no worktrees"));
}

#[test]
fn fork_asks_where_it_runs_when_a_worktree_can_be_made() {
    let (mut chat, events) = chat(false);
    let name = Some("experiment".to_string());
    for (downs, expected) in [
        (0, SessionCall::ForkHere(name.clone())),
        (1, worktree::enter(true, name.clone())),
    ] {
        reply(&mut chat, SessionReply::ForkChoice(name.clone()));
        assert!(screen(&chat, 100).contains("Where should the fork run?"));
        for _ in 0..downs {
            key(&mut chat, KeyCode::Down);
        }
        key(&mut chat, KeyCode::Enter);
        assert_eq!(sent(&events), [call(expected)]);
    }
}
