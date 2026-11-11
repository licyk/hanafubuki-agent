//! The `review` plugin: each target's request, the reviewer's answer read
//! tolerantly, and on a running Agent over a scripted model a review whose
//! findings reach the parent, whose mutating command is denied while a
//! read-only `git` runs, one interrupted, the targets of a repository, and
//! the plugin turned off.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use hanafubuki_agent_kernel::{
    AgentError, AgentSessionId, AgentSessionListRequest, AgentSessionMode, AgentSessionOrigin,
    AgentSessionParentFilter, ConversationItem,
};

use super::output::parse_review_answer;
use super::read_only::READ_ONLY_REASON;
use super::request::{review_prompt, user_facing_hint};
use super::{
    AgentReviewCodeLocation, AgentReviewCommit, AgentReviewFinding, AgentReviewLineRange,
    AgentReviewOutput, AgentReviewTarget, REVIEW_PLUGIN,
};
use crate::test_support::{
    assistant_events, configure_test_provider, service_with_host, test_host, tool_calls_events,
    FakeServer, TestPaths, TEST_SHUTDOWN_DEADLINE,
};
use crate::{AgentApplicationService, AgentPluginState};

const FINDINGS: &str = r#"{
  "findings": [
    {
      "title": "[P1] Check the length before indexing",
      "body": "`items[0]` panics on an empty list.",
      "confidence_score": 0.8,
      "priority": 1,
      "code_location": {
        "absolute_file_path": "/work/src/lib.rs",
        "line_range": {"start": 10, "end": 12}
      }
    }
  ],
  "overall_correctness": "patch is incorrect",
  "overall_explanation": "One panic on empty input.",
  "overall_confidence_score": 0.7
}"#;

fn finding() -> AgentReviewFinding {
    AgentReviewFinding {
        title: "[P1] Check the length before indexing".to_string(),
        body: "`items[0]` panics on an empty list.".to_string(),
        confidence_score: 0.8,
        priority: Some(1),
        code_location: Some(AgentReviewCodeLocation {
            absolute_file_path: PathBuf::from("/work/src/lib.rs"),
            line_range: AgentReviewLineRange { start: 10, end: 12 },
        }),
    }
}

#[test]
fn each_target_asks_for_its_review() {
    assert_eq!(
        review_prompt(&AgentReviewTarget::UncommittedChanges, None),
        "Review the current code changes (staged, unstaged, and untracked files) and provide prioritized findings."
    );
    let branch = AgentReviewTarget::BaseBranch {
        branch: "main".to_string(),
    };
    assert_eq!(
        review_prompt(&branch, Some("abc123")),
        "Review the code changes against the base branch 'main'. The merge base commit for this comparison is abc123. Run `git diff abc123` to inspect the changes relative to main. Provide prioritized, actionable findings."
    );
    assert_eq!(
        review_prompt(&branch, None),
        "Review the code changes against the base branch 'main'. Start by finding the merge diff between the current branch and main's upstream e.g. (`git merge-base HEAD \"$(git rev-parse --abbrev-ref \"main@{upstream}\")\"`), then run `git diff` against that SHA to see what changes we would merge into the main branch. Provide prioritized, actionable findings."
    );
    let commit = AgentReviewTarget::Commit {
        sha: "0123456789abcdef".to_string(),
        title: Some("Fix the parser".to_string()),
    };
    assert_eq!(
        review_prompt(&commit, None),
        "Review the code changes introduced by commit 0123456789abcdef (\"Fix the parser\"). Provide prioritized, actionable findings."
    );
    let untitled = AgentReviewTarget::Commit {
        sha: "0123456789abcdef".to_string(),
        title: None,
    };
    assert_eq!(
        review_prompt(&untitled, None),
        "Review the code changes introduced by commit 0123456789abcdef. Provide prioritized, actionable findings."
    );
    let custom = AgentReviewTarget::Custom {
        instructions: "  Look at the error handling.\n".to_string(),
    };
    assert_eq!(review_prompt(&custom, None), "Look at the error handling.");

    assert_eq!(
        [
            AgentReviewTarget::UncommittedChanges,
            branch,
            commit,
            untitled,
            custom
        ]
        .iter()
        .map(user_facing_hint)
        .collect::<Vec<_>>(),
        [
            "current changes",
            "changes against 'main'",
            "commit 0123456: Fix the parser",
            "commit 0123456",
            "Look at the error handling."
        ]
    );
}

#[test]
fn the_answer_is_read_as_json_else_as_its_text() {
    let child = AgentSessionId::new(7);
    let expected = AgentReviewOutput {
        findings: vec![finding()],
        overall_correctness: "patch is incorrect".to_string(),
        overall_explanation: "One panic on empty input.".to_string(),
        overall_confidence_score: 0.7,
        child_session: child,
    };
    assert_eq!(parse_review_answer(FINDINGS, child), expected);
    assert_eq!(
        parse_review_answer(
            &format!("Here is the review:\n```json\n{FINDINGS}\n```"),
            child
        ),
        expected,
        "the first JSON object in prose"
    );

    let prose = "The change looks right to me.";
    assert_eq!(
        parse_review_answer(prose, child),
        AgentReviewOutput {
            findings: Vec::new(),
            overall_correctness: String::new(),
            overall_explanation: prose.to_string(),
            overall_confidence_score: 0.0,
            child_session: child,
        }
    );
    let unrelated = r#"{"answer": 42}"#;
    assert_eq!(
        parse_review_answer(unrelated, child).overall_explanation,
        unrelated,
        "an object with none of a review's fields is not one"
    );
}

#[test]
fn a_finding_that_does_not_read_is_left_out_and_missing_fields_default() {
    let answer = r#"{
      "findings": [
        "not a finding",
        {"title": "Unlocated", "priority": null, "confidence_score": "high"},
        {"title": "Located", "body": "Line one.\nLine two.", "code_location": {"absolute_file_path": "/a.rs", "line_range": {"start": 3, "end": 3}}}
      ],
      "overall_correctness": "patch is correct"
    }"#;
    let output = parse_review_answer(answer, AgentSessionId::new(1));

    assert_eq!(
        output.findings,
        vec![
            AgentReviewFinding {
                title: "Unlocated".to_string(),
                body: String::new(),
                confidence_score: 0.0,
                priority: None,
                code_location: None,
            },
            AgentReviewFinding {
                title: "Located".to_string(),
                body: "Line one.\nLine two.".to_string(),
                confidence_score: 0.0,
                priority: None,
                code_location: Some(AgentReviewCodeLocation {
                    absolute_file_path: PathBuf::from("/a.rs"),
                    line_range: AgentReviewLineRange { start: 3, end: 3 },
                }),
            },
        ]
    );
    assert_eq!(output.overall_explanation, "");
    assert_eq!(
        output.render_text(),
        "Full review comments:\n\n- Unlocated\n\n- Located — /a.rs:3-3\n  Line one.\n  Line two."
    );
    assert_eq!(
        AgentReviewOutput {
            findings: Vec::new(),
            ..output
        }
        .render_text(),
        "Reviewer failed to output a response."
    );
}

/// A started Agent on `server`.
async fn started(name: &str, server: &FakeServer) -> (AgentApplicationService, TestPaths) {
    let (service, paths, _) = service_with_host(name, test_host, |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    service.start_plugins().await;
    (service, paths)
}

fn finish(service: AgentApplicationService, paths: TestPaths) {
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A session in `mode` that has had one turn, answered by the server's
/// next response.
async fn parent(service: &AgentApplicationService, mode: AgentSessionMode) -> AgentSessionId {
    let parent = service.create_session(mode, None).await.unwrap().id;
    service
        .start_turn(parent, "Hello.".to_string().into())
        .await
        .unwrap();
    parent
}

fn body(server: &FakeServer, index: usize) -> String {
    String::from_utf8(server.requests()[index].body.clone()).unwrap()
}

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

/// The test workspace as a repository with `keep.txt` committed.
fn seed_repository(paths: &TestPaths) -> PathBuf {
    let workspace = paths.managed_instances_dir.clone();
    std::fs::write(workspace.join("keep.txt"), "kept\n").unwrap();
    git(&workspace, &["init", "-q", "-b", "main"]);
    git(&workspace, &["add", "keep.txt"]);
    git(&workspace, &["commit", "-q", "-m", "Seed the repository"]);
    workspace
}

/// Waits for the command up to the 30 s maximum: on a loaded runner a
/// `git` can outlast the default 10 s, and the result then holds a live
/// session with no output yet.
fn exec_arguments(cmd: &str) -> String {
    serde_json::json!({
        "cmd": cmd, "shell": null, "login": null, "tty": null,
        "yield_time_ms": 30_000, "max_output_bytes": null, "workdir": null
    })
    .to_string()
}

fn children(service: &AgentApplicationService, parent: AgentSessionId) -> Vec<AgentSessionId> {
    service
        .list_sessions_page(AgentSessionListRequest {
            parent: AgentSessionParentFilter::ChildrenOf(parent),
            ..AgentSessionListRequest::default()
        })
        .unwrap()
        .sessions
        .into_iter()
        .map(|summary| summary.session_id)
        .collect()
}

/// The review's continuation context in `session`'s history.
fn exit_notes(service: &AgentApplicationService, session: AgentSessionId) -> Vec<String> {
    service
        .open_session(session, 50)
        .unwrap()
        .transcript
        .items
        .into_iter()
        .filter_map(|item| match item.content {
            ConversationItem::ContinuationContext {
                hook_id, content, ..
            } if hook_id.as_str() == "review-exit" => Some(content),
            _ => None,
        })
        .collect()
}

/// The review runs in a child session with the rubric: its mutating
/// command is denied while a read-only `git` runs, its answer is the
/// output, and the parent's model reads the findings at its next turn.
// The turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn a_review_reads_only_and_its_findings_reach_the_parent() {
    let server = FakeServer::start(vec![
        (assistant_events("Hi."), Duration::ZERO),
        (
            tool_calls_events(&[
                ("call-rm", "exec_command", &exec_arguments("rm keep.txt")),
                (
                    "call-log",
                    "exec_command",
                    &exec_arguments("git --no-pager log --oneline"),
                ),
            ]),
            Duration::ZERO,
        ),
        (assistant_events(FINDINGS), Duration::ZERO),
        (assistant_events("I will fix it."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = started("review-findings", &server).await;
    let workspace = seed_repository(&paths);
    let parent = parent(&service, AgentSessionMode::Plan).await;

    let output = service
        .start_review(parent, AgentReviewTarget::UncommittedChanges)
        .await
        .unwrap();

    let child = output.child_session;
    assert_eq!(
        output,
        AgentReviewOutput {
            findings: vec![finding()],
            overall_correctness: "patch is incorrect".to_string(),
            overall_explanation: "One panic on empty input.".to_string(),
            overall_confidence_score: 0.7,
            child_session: child,
        }
    );
    let snapshot = service.snapshot(child).unwrap();
    assert_eq!(snapshot.parent_session_id, Some(parent));
    assert_eq!(snapshot.origin, Some(AgentSessionOrigin::Review));
    assert_eq!(
        snapshot.mode,
        AgentSessionMode::Default,
        "a review runs commands even for a parent in Plan mode"
    );
    assert_eq!(
        snapshot.title.unwrap().value,
        "Review: current changes".to_string()
    );
    assert_eq!(children(&service, parent), vec![child]);
    let first = body(&server, 1);
    assert!(first.contains("# Review guidelines:"), "{first}");
    assert!(first.contains("Review the current code changes"));
    assert!(first.contains(r#""name":"exec_command""#));
    assert!(
        !first.contains("Hello."),
        "a review starts from nothing of its parent's"
    );

    let after_tools = body(&server, 2);
    assert_eq!(
        after_tools.matches(&READ_ONLY_REASON[..40]).count(),
        1,
        "only the removal is denied: {after_tools}"
    );
    assert!(
        after_tools.contains("Seed the repository"),
        "the read-only git ran: {after_tools}"
    );
    assert!(workspace.join("keep.txt").exists());

    let notes = exit_notes(&service, parent);
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("User initiated a review task."));
    assert!(notes[0].contains(
        "One panic on empty input.\n\nReview comment:\n\n- [P1] Check the length before indexing — /work/src/lib.rs:10-12\n  `items[0]` panics on an empty list."
    ));
    service
        .start_turn(parent, "Fix what the review found.".to_string().into())
        .await
        .unwrap();
    let parent_request = body(&server, 3);
    assert!(parent_request.contains("User initiated a review task."));
    assert!(parent_request.contains("Check the length before indexing"));
    assert!(
        !body(&server, 0).contains("# Review guidelines:")
            && !parent_request.contains("# Review guidelines:"),
        "the rubric is the review session's only"
    );
    finish(service, paths);
}

/// An answer that is not the rubric's JSON is the explanation, and the
/// parent reads it as the review's results.
#[tokio::test(flavor = "multi_thread")]
async fn a_review_answer_that_is_not_json_is_its_explanation() {
    let server = FakeServer::start(vec![
        (assistant_events("Hi."), Duration::ZERO),
        (
            assistant_events("Nothing to flag; the change is small."),
            Duration::ZERO,
        ),
    ])
    .await;
    let (service, paths) = started("review-prose", &server).await;
    let parent = parent(&service, AgentSessionMode::Default).await;

    let output = service
        .start_review(
            parent,
            AgentReviewTarget::Custom {
                instructions: "Check the README.".to_string(),
            },
        )
        .await
        .unwrap();

    assert!(output.findings.is_empty());
    assert_eq!(
        output.overall_explanation,
        "Nothing to flag; the change is small."
    );
    assert!(body(&server, 1).contains("Check the README."));
    assert!(exit_notes(&service, parent)[0]
        .contains("<results>\n  Nothing to flag; the change is small.\n  </results>"));
    assert_eq!(
        service
            .start_review(
                parent,
                AgentReviewTarget::Custom {
                    instructions: " \n".to_string(),
                },
            )
            .await,
        Err(AgentError::InvalidInput(
            "Review prompt cannot be empty".to_string()
        ))
    );
    finish(service, paths);
}

/// An interrupted review is the call's error, and the parent is told it
/// was interrupted.
#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_review_tells_the_parent_so() {
    let server = FakeServer::start(vec![
        (assistant_events("Hi."), Duration::ZERO),
        (assistant_events(FINDINGS), Duration::from_secs(30)),
    ])
    .await;
    let (service, paths) = started("review-cancel", &server).await;
    let parent = parent(&service, AgentSessionMode::Default).await;

    let review = tokio::spawn({
        let service = service.clone();
        async move {
            service
                .start_review(parent, AgentReviewTarget::UncommittedChanges)
                .await
        }
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while server.requests().len() < 2 {
        assert!(Instant::now() < deadline, "the review samples");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let child = children(&service, parent)[0];
    assert!(matches!(
        service.cancel_review(parent).await,
        Err(AgentError::InvalidInput(_))
    ));
    service.cancel_review(child).await.unwrap();

    assert_eq!(review.await.unwrap(), Err(AgentError::Cancelled));
    let notes = exit_notes(&service, parent);
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("User initiated a review task, but was interrupted."));
    finish(service, paths);
}

/// A history starts with a turn, so a conversation that has had none
/// cannot be told how a review ended: no review starts for it.
#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_without_a_turn_is_not_reviewed() {
    let server = FakeServer::start(Vec::new()).await;
    let (service, paths) = started("review-no-turn", &server).await;
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;

    assert!(matches!(
        service
            .start_review(parent, AgentReviewTarget::UncommittedChanges)
            .await,
        Err(AgentError::InvalidInput(message)) if message.contains("has had no turn yet")
    ));
    assert!(children(&service, parent).is_empty());
    assert!(server.requests().is_empty());
    finish(service, paths);
}

/// The targets are the working directory's branches, current branch and
/// latest commits.
#[tokio::test(flavor = "multi_thread")]
async fn the_targets_are_the_repositorys_branches_and_commits() {
    let server = FakeServer::start(Vec::new()).await;
    let (service, paths) = started("review-targets", &server).await;
    let workspace = seed_repository(&paths);
    git(&workspace, &["branch", "feature"]);

    let targets = service.review_targets().await.unwrap();

    assert_eq!(targets.branches, ["main", "feature"]);
    assert_eq!(targets.current_branch.as_deref(), Some("main"));
    assert!(matches!(
        targets.commits.as_slice(),
        [AgentReviewCommit { subject, .. }] if subject == "Seed the repository"
    ));
    finish(service, paths);
}

/// With the plugin off its calls are unavailable; the rest of the Agent
/// works on.
#[tokio::test(flavor = "multi_thread")]
async fn with_the_review_plugin_off_the_calls_are_unavailable() {
    let server = FakeServer::start(Vec::new()).await;
    let (service, paths) = started("review-off", &server).await;
    service
        .set_plugin_enabled(REVIEW_PLUGIN.into(), false)
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !service
        .plugin_status()
        .iter()
        .any(|status| status.name == REVIEW_PLUGIN && status.state == AgentPluginState::Disabled)
    {
        assert!(Instant::now() < deadline, "the review plugin is off");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let parent = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;

    let unavailable = AgentError::FeatureUnavailable {
        feature: "review".to_string(),
        reason: "the review Agent plugin is unavailable: it is turned off".to_string(),
    };
    assert_eq!(
        service
            .start_review(parent, AgentReviewTarget::UncommittedChanges)
            .await,
        Err(unavailable.clone())
    );
    assert_eq!(
        service.cancel_review(parent).await,
        Err(unavailable.clone())
    );
    assert_eq!(service.review_targets().await, Err(unavailable));
    assert!(children(&service, parent).is_empty());
    finish(service, paths);
}
