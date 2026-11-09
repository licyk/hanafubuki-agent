//! `/permissions`, `/approve`, `/cd` and `/worktree` end to end over the
//! standalone Agent: what each changed, read back through the facade.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use hanafubuki_agent::sandbox::SandboxStatus;
use hanafubuki_agent::test_support::assistant_events;
use hanafubuki_agent::{AgentPermissionMode, AgentSessionListRequest, AgentWorktree};
use ratatui::crossterm::event::KeyCode;

use super::{command_call, Harness};
use crate::transcript_mode::TranscriptMode;

/// What a scripted automatic review answers to deny an action.
const DENIAL: &str = r#"{"riskLevel":"high","userAuthorization":"absent","outcome":"deny","rationale":"The user did not ask for this command."}"#;

impl Harness {
    /// The Agent's worktrees of the project's repository.
    fn worktrees(&self) -> Vec<AgentWorktree> {
        let agent = self.runtime.slot().agent().unwrap();
        let call = agent.clone();
        agent
            .runtime()
            .block_on(async move { call.worktree_list().await })
            .unwrap()
            .unwrap()
    }

    fn working_directory(&self) -> PathBuf {
        let agent = self.runtime.slot().agent().unwrap();
        agent.workspace_roots().unwrap().into_vec().remove(0)
    }

    /// The project as a repository with one commit, in place of the empty
    /// `.git` the harness makes.
    fn repository(&self) -> PathBuf {
        let project = self.home.path().join("project");
        std::fs::remove_dir_all(project.join(".git")).unwrap();
        std::fs::write(project.join("file.txt"), "committed\n").unwrap();
        git(&project, &["init", "-q"]);
        // Windows' Git converts line endings at checkout by default.
        git(&project, &["config", "core.autocrlf", "false"]);
        git(&project, &["add", "."]);
        git(
            &project,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-q",
                "-m",
                "initial",
            ],
        );
        project
    }

    /// `/worktree`, Browse, the one worktree listed, Remove, and yes.
    fn remove_the_worktree(&mut self) {
        self.type_and_submit("/worktree");
        self.pump_until_screen_shows("Browse worktrees");
        self.key(KeyCode::Down);
        self.key(KeyCode::Down);
        self.key(KeyCode::Enter);
        self.pump_until_screen_shows("· detached · session");
        self.key(KeyCode::Enter);
        self.pump_until_screen_shows("Remove worktree");
        self.key(KeyCode::Down);
        self.key(KeyCode::Down);
        self.key(KeyCode::Enter);
        self.confirm("Remove this worktree?");
    }
}

fn git(directory: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn a_denied_action_is_approved_between_turns_and_refused_during_one() {
    // A review is named after its call: the second call needs an id of its own.
    let second = String::from_utf8(command_call("echo second-denied"))
        .unwrap()
        .replace("call-tool", "call-tool-2")
        .into_bytes();
    let responses = vec![
        (command_call("echo first-denied"), Duration::ZERO),
        (assistant_events(DENIAL), Duration::ZERO),
        (second, Duration::ZERO),
        (assistant_events(DENIAL), Duration::ZERO),
        (assistant_events("Not run."), Duration::ZERO),
        // Long enough for `/approve` to be tried while it runs.
        (assistant_events("Going on."), Duration::from_secs(10)),
    ];
    let mut harness = Harness::start_timed(responses, true, TranscriptMode::Terminal);
    harness.type_and_submit("/permissions auto-review");
    harness.pump_until_screen_shows("Permissions set to Auto-review for this session");
    let session = harness.app.bridge.session_id().unwrap();
    let agent = harness.runtime.slot().agent().unwrap();
    assert_eq!(
        agent.session_permission_mode(session),
        Ok(Some(AgentPermissionMode::AutoReview))
    );
    // The list is read through the bridge. The configuration's sandbox is
    // full access, so no preset is the one in effect; the presets that need
    // a sandbox cannot be chosen where there is none: a test binary is no
    // sandbox helper, but macOS's Seatbelt is the system's.
    let sandbox = agent.sandbox_status().unwrap();
    harness.type_and_submit("/permissions");
    let list = harness.pump_until_screen_shows("Select Permissions");
    assert!(!list.contains("(current)"), "{list}");
    assert_eq!(
        list.contains("The sandbox is unavailable on this machine"),
        !matches!(sandbox, SandboxStatus::Available { .. }),
        "{sandbox:?}: {list}"
    );
    harness.key(KeyCode::Esc);
    harness.type_and_submit("/status");
    harness.pump_until_screen_shows("Permissions:");
    assert!(harness.transcript().contains("auto-review"));

    harness.exchange("Run the commands.", "Not run.");
    harness.type_and_submit("/approve");
    // The latest first.
    let picker = harness.pump_until_screen_shows("Auto-review Denials");
    // A long summary wraps its rationale beside it, or has it underneath.
    let unwrapped: String = picker.split_whitespace().collect();
    for text in [
        "echo second-denied",
        "The user did not ask for this command.",
    ] {
        let text: String = text.split_whitespace().collect();
        assert!(unwrapped.contains(&text), "{text}: {picker}");
    }
    for character in "first".chars() {
        harness.key(KeyCode::Char(character));
    }
    // The summaries name the directory, so where they wrap varies.
    harness.pump_until_screen_shows("first-denied");
    let requests = harness.server.requests().len();
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Approved one retry of:");
    assert!(harness.transcript().contains("first-denied"));
    assert_eq!(
        harness.server.requests().len(),
        requests,
        "the approval starts no turn"
    );

    harness.type_and_submit("Go on.");
    harness.pump_until("the turn to start", |harness| {
        harness.app.chat_widget.is_task_running()
    });
    harness.type_and_submit("/approve");
    harness.pump_until_screen_shows("Type to search denials");
    // The approved one is gone from the list.
    for character in "first".chars() {
        harness.key(KeyCode::Char(character));
    }
    harness.pump_until_screen_shows("no matches");
    for _ in 0.."first".len() {
        harness.key(KeyCode::Backspace);
    }
    harness.pump_until_screen_shows("second-denied");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("/approve: the action was not approved");
    harness.pump_until_screen_shows("Going on.");
}

#[test]
fn the_status_line_shows_the_mode_permissions_set() {
    let mut harness = Harness::owned(Vec::new());
    harness.pump_until_screen_shows("fixture-model");
    harness.type_and_submit("/statusline permission-mode --no-colors");
    harness.pump_until_screen_shows("Updated for this session; it is not saved.");
    // The session is created and its status read first.
    harness.type_and_submit("/permissions manual");
    harness.pump_until("the status line to show manual", |harness| {
        harness.bottom_rows(1).trim() == "manual"
    });
    harness.type_and_submit("/permissions full-access");
    harness.pump_until_screen_shows("Permissions set to Full access for this session");
    harness.pump_until("the status line to show the session's mode", |harness| {
        harness.bottom_rows(1).trim() == "full-access"
    });

    // A preset sets the session's sandbox mode with its permission mode.
    harness.type_and_submit("/permissions read-only");
    harness.pump_until_screen_shows("Permissions set to Read only for this session");
    let session = harness.app.bridge.session_id().unwrap();
    let agent = harness.runtime.slot().agent().unwrap();
    assert_eq!(
        agent.session_permission_mode(session),
        Ok(Some(AgentPermissionMode::Manual))
    );
    assert_eq!(
        agent.session_sandbox_mode(session),
        Ok(Some(hanafubuki_agent::AgentSandboxMode::ReadOnly))
    );
    harness.pump_until("the status line to show the preset's mode", |harness| {
        harness.bottom_rows(1).trim() == "manual"
    });
}

#[test]
fn approve_without_denials_says_so() {
    let mut harness = Harness::start(Vec::new());
    harness.type_and_submit("/approve");
    harness.pump_until_screen_shows("No denied actions to approve.");
}

#[test]
fn cd_moves_the_working_directory_and_refuses_a_missing_one() {
    let mut harness = Harness::start(Vec::new());
    std::fs::create_dir_all(harness.home.path().join("other")).unwrap();
    harness.type_and_submit("/cd ../other");
    harness.pump_until_screen_shows("Working directory changed to");
    let moved = harness.working_directory();
    assert!(moved.ends_with("other"), "{}", moved.display());
    assert!(harness
        .transcript()
        .contains("stays the one read at start until you restart"));

    harness.type_and_submit("/cd missing");
    harness.pump_until_screen_shows("/cd: the working directory was not changed");
    assert_eq!(harness.working_directory(), moved);
}

#[test]
fn worktree_continues_the_conversation_there_and_browse_removes_it() {
    let mut harness = Harness::start(vec![assistant_events("Shared answer.")]);
    let project = harness.repository();
    let original = harness.exchange("Shared question", "Shared answer.");

    harness.type_and_submit("/worktree");
    harness.pump_until_screen_shows("Continue this conversation");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Continuing in a fork of this conversation");
    harness.pump_until("the fork", |harness| {
        harness
            .app
            .bridge
            .session_id()
            .is_some_and(|session| session != original)
    });
    let fork = harness.app.bridge.session_id().unwrap();
    let worktrees = harness.worktrees();
    assert_eq!(worktrees.len(), 1, "{worktrees:?}");
    let worktree = &worktrees[0].path;
    assert_eq!(worktrees[0].owner, Some(fork));
    assert_eq!(harness.working_directory(), *worktree);
    assert_eq!(
        std::fs::read_to_string(worktree.join("file.txt")).unwrap(),
        "committed\n"
    );
    harness.pump_until("the fork's transcript", |harness| {
        harness.transcript().contains("Shared answer.")
    });

    // The working directory is in it: it stays.
    harness.remove_the_worktree();
    harness.pump_until_screen_shows("/worktree: the worktree was not removed");
    assert!(worktree.exists());

    harness.type_and_submit(&format!("/cd {}", project.display()));
    harness.pump_until_screen_shows("Working directory changed to");
    harness.remove_the_worktree();
    harness.pump_until_screen_shows("Removed the worktree");
    assert!(!worktree.exists());
    assert!(harness.worktrees().is_empty());
}

#[test]
fn worktree_starts_the_next_conversation_there_bound_to_it() {
    let mut harness = Harness::start(vec![assistant_events("Fresh answer.")]);
    harness.repository();
    harness.type_and_submit("/worktree");
    harness.pump_until_screen_shows("Start a new conversation");
    // With no conversation yet, this row is the one selected.
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Moved into the new worktree");
    assert_eq!(harness.app.bridge.session_id(), None);

    let session = harness.exchange("Fresh question", "Fresh answer.");
    let worktrees = harness.worktrees();
    assert_eq!(worktrees.len(), 1, "{worktrees:?}");
    assert_eq!(worktrees[0].owner, Some(session));
    assert_eq!(harness.working_directory(), worktrees[0].path);
    let agent = harness.runtime.slot().agent().unwrap();
    let summary = agent
        .list_sessions_page(AgentSessionListRequest::default())
        .unwrap()
        .sessions
        .into_iter()
        .find(|summary| summary.session_id == session)
        .unwrap();
    assert_eq!(
        summary.cwd.as_deref(),
        worktrees[0].path.to_str(),
        "the session records the worktree it was created in"
    );
}

#[test]
fn worktree_outside_a_git_repository_says_why_there_is_none() {
    let mut harness = Harness::start(Vec::new());
    let project = harness.home.path().join("project");
    std::fs::remove_dir_all(project.join(".git")).unwrap();
    if project
        .ancestors()
        .any(|directory| directory.join(".git").exists())
    {
        eprintln!("skipped: the temporary directory is inside a Git repository here");
        return;
    }
    harness.type_and_submit("/worktree");
    // The message names the directory, so where the screen wraps it varies.
    harness.pump_until("the reason there is no worktree", |harness| {
        harness.transcript().contains("is not in a Git repository")
    });
}
