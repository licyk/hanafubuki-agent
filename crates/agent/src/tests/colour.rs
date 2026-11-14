//! Colour in command output, end to end on the fake model server: a
//! command the model ran and one the person typed print a coloured word.
//! What the model's next request carries, the history's tool output and
//! the export are plain; the display (live, and read back after a restart)
//! has the styles. The environment commands get is unchanged
//! (`NO_COLOR=1`, `TERM=dumb`), so the command forces its colour itself,
//! as `ls --color=always` does.

use super::*;
use crate::test_support::TestPaths;
use hanafubuki_agent_process::executor::ExecutorManager;
use serde_json::Value;

#[cfg(unix)]
const COLOURED: &str = r"printf '\033[31mred\033[0m plain\n'";
#[cfg(windows)]
const COLOURED: &str = r#"[Console]::Out.Write("$([char]27)[31mred$([char]27)[0m plain`n")"#;

/// The text of each run of a command display's styles, with its colour.
fn styled_runs(display: &AgentToolDisplay) -> Vec<(String, Option<AgentTextColor>)> {
    let AgentToolDisplay::Command { output, styled, .. } = display else {
        panic!("a command: {display:?}");
    };
    let Some(styled) = styled else {
        return Vec::new();
    };
    styled
        .spans
        .iter()
        .map(|span| (output[span.start..span.end].to_string(), span.style.fg))
        .collect()
}

/// Whether the turn it hears has ended.
#[derive(Default)]
struct Ended(std::sync::atomic::AtomicBool);

impl AgentEventSink for Ended {
    fn emit(&self, event: AgentEvent) {
        if event.kind.is_terminal() {
            self.0.store(true, Ordering::SeqCst);
        }
    }
}

fn assert_plain(text: &str) {
    assert!(!text.contains('\u{1b}'), "{text:?}");
    assert!(!text.contains("\\u001b"), "{text:?}");
    assert!(!text.contains("styled"), "{text:?}");
    assert!(!text.contains("displayMetadata"), "{text:?}");
}

fn transcript(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
) -> Vec<ConversationItem> {
    service
        .read_transcript_page(AgentTranscriptPageRequest {
            session_id,
            page_size: 100,
            cursor: None,
            direction: AgentHistorySortDirection::Ascending,
            through_revision: None,
        })
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.content)
        .collect()
}

/// A new Agent over `paths`' storage, as after a restart, its plugins (the
/// presenters among them) started.
async fn reopen(config: AgentConfig, paths: &TestPaths) -> AgentApplicationService {
    let service = AgentApplicationService::new_with_host(
        config,
        test_host(paths),
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        ExecutorManager::default(),
    )
    .unwrap();
    service.start_plugins().await;
    service
}

/// Runs the model's coloured command (approved by hand) and its answer.
async fn model_runs_the_coloured_command(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
) {
    let executing = service.clone();
    let turn = tokio::spawn(async move {
        executing
            .start_turn(session_id, "Run it.".to_string().into())
            .await
    });
    let (turn_identity, request) = tokio::time::timeout(TEST_WAIT, async {
        loop {
            if let Some(turn_identity) = service.snapshot(session_id).unwrap().turn {
                if let Some(request) = service
                    .events_since(session_id, None)
                    .unwrap()
                    .events
                    .into_iter()
                    .find_map(|event| match event.kind {
                        AgentEventKind::ApprovalRequested { request } => Some(request),
                        _ => None,
                    })
                {
                    break (turn_identity, request);
                }
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("the command's approval must be asked");
    service
        .resolve_approval(
            session_id,
            turn_identity.id,
            turn_identity.generation,
            request.request_id,
            ApprovalUserDecision::AllowOnce,
        )
        .unwrap();
    tokio::time::timeout(TEST_WAIT, turn)
        .await
        .expect("the turn must end")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn a_models_coloured_command_is_plain_for_the_model_and_styled_for_the_display() {
    let arguments = serde_json::to_string(&json!({
        "cmd": COLOURED,
        "shell": null,
        "login": null,
        "tty": null,
        "yield_time_ms": 30_000,
        "max_output_bytes": 32_768,
        "workdir": null
    }))
    .unwrap();
    let server = FakeServer::start(vec![
        (tool_events("exec_command", &arguments), Duration::ZERO),
        (assistant_events("It printed red."), Duration::ZERO),
    ])
    .await;
    let (service, paths, config) = service_with_config("colour-model-command", |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    model_runs_the_coloured_command(&service, session.id).await;

    // The request after the call carries the output as the model reads it.
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    let second: Value = serde_json::from_slice(&requests[1].body).unwrap();
    let output = second["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .expect("the command's output")
        .to_string();
    assert!(output.contains("red plain"), "{output}");
    assert_plain(&output);

    let items = transcript(&service, session.id);
    let Some(ConversationItem::ToolOutput {
        call_id,
        output,
        display_metadata,
        ..
    }) = items
        .iter()
        .find(|item| matches!(item, ConversationItem::ToolOutput { .. }))
    else {
        panic!("{items:#?}");
    };
    assert_plain(&serde_json::to_string(output).unwrap());
    assert!(display_metadata.styled_output.is_some());
    let red = Some(AgentTextColor::Indexed(1));
    let shown = service.tool_display(session.id, call_id).unwrap().unwrap();
    assert_eq!(
        styled_runs(shown.display.as_ref().unwrap()),
        [("red".to_string(), red)]
    );
    let call_id = call_id.clone();
    assert_plain(&service.export_session_markdown(session.id).unwrap());
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());

    // Read back from the history after a restart, as `/resume` does.
    let reopened = reopen(config, &paths).await;
    let shown = reopened
        .tool_display(session.id, &call_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        styled_runs(shown.display.as_ref().unwrap()),
        [("red".to_string(), red)]
    );
    assert!(reopened
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn a_typed_coloured_command_is_plain_for_the_model_and_styled_for_the_display() {
    let server = FakeServer::start(vec![(assistant_events("Seen."), Duration::ZERO)]).await;
    let (service, paths, config) = service_with_config("colour-user-shell", |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let ended = Arc::new(Ended::default());
    let turn = service
        .submit_user_shell_command_with_sink(session.id, COLOURED.to_string(), ended.clone())
        .await
        .unwrap();
    let call_id = format!("user-shell-{}", turn.turn_id);
    tokio::time::timeout(TEST_WAIT, async {
        while !ended.0.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the command's turn must end");

    let red = Some(AgentTextColor::Indexed(1));
    let shown = service.tool_display(session.id, &call_id).unwrap().unwrap();
    assert_eq!(
        styled_runs(shown.display.as_ref().unwrap()),
        [("red".to_string(), red)]
    );
    service
        .start_turn(session.id, "What did it print?".to_string().into())
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    let input = body["input"].to_string();
    assert!(input.contains("<user_shell_command>"), "{input}");
    assert!(input.contains("red plain"), "{input}");
    assert_plain(&input);
    assert_plain(&service.export_session_markdown(session.id).unwrap());
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());

    let reopened = reopen(config, &paths).await;
    let items = transcript(&reopened, session.id);
    assert!(
        items.iter().any(|item| matches!(
            item,
            ConversationItem::UserShellCommand { styled: Some(_), output, .. }
                if !output.contains('\u{1b}')
        )),
        "{items:#?}"
    );
    let shown = reopened
        .tool_display(session.id, &call_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        styled_runs(shown.display.as_ref().unwrap()),
        [("red".to_string(), red)]
    );
    assert!(reopened
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
