//! A `!command` (`submit_user_shell_command_with_sink`): a turn that asks no
//! model, runs the command through `exec_command` in the session's sandbox
//! as the person's own call, commits a `UserShellCommand` item, and leaves
//! it for the next request. Plain commands that every platform's default
//! shell runs (`echo`, `exit 3`, `sleep 30`: PowerShell on Windows).

use hanafubuki_agent_approval::{ApprovalAction, ApprovalActionKind};
use hanafubuki_agent_contracts::{
    AgentSessionOrigin, AgentToolDisplay, ApprovalRule, ApprovalRuleRegistry, ApprovalRuleVerdict,
    Register,
};
use hanafubuki_agent_kernel::{
    AgentEvent, AgentEventSink, AgentHistorySortDirection, AgentTranscriptPageRequest,
    AgentTurnAccepted, ConversationItem,
};
use kardia_core::{Context, Inject, PluginDef, Service};
use serde_json::Value;

use super::host::{AgentPluginDescriptor, AgentPluginGroup, AgentPluginRow};
use super::*;
use crate::test_support::TestPaths;
use hanafubuki_agent_process::executor::ExecutorManager;

/// The turn's events, as an interface's sink gets them.
#[derive(Default)]
struct Recorded(Mutex<Vec<AgentEvent>>);

impl AgentEventSink for Recorded {
    fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Recorded {
    fn kinds(&self) -> Vec<AgentEventKind> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|event| event.kind.clone())
            .collect()
    }
}

/// Runs `!command` in `session_id` and waits for its turn's end.
async fn run(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
    command: &str,
) -> (AgentTurnAccepted, Arc<Recorded>, AgentEventKind) {
    let sink = Arc::new(Recorded::default());
    let turn = service
        .submit_user_shell_command_with_sink(session_id, command.to_string(), sink.clone())
        .await
        .unwrap();
    let end = turn_end(&sink).await;
    (turn, sink, end)
}

async fn turn_end(sink: &Recorded) -> AgentEventKind {
    // A cold PowerShell start on a loaded CI runner takes seconds.
    tokio::time::timeout(TEST_WAIT, async {
        loop {
            if let Some(end) = sink.kinds().into_iter().find(AgentEventKind::is_terminal) {
                return end;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the command's turn must end within the fixture deadline")
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

/// The text of the input items of a request body.
fn input_text(body: &Value) -> String {
    body["input"].to_string()
}

#[tokio::test]
async fn a_command_runs_without_the_model_and_the_next_request_carries_it() {
    let server = FakeServer::start(vec![(assistant_events("Noted."), Duration::ZERO)]).await;
    let (service, paths) = configured_service("user-shell-runs", &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();

    let (turn, sink, end) = run(&service, session.id, "echo user-shell-marker").await;

    assert_eq!(
        end,
        AgentEventKind::TurnCompleted {
            final_answer_message_id: None
        }
    );
    assert!(server.requests().is_empty(), "no model was asked");
    let call_id = format!("user-shell-{}", turn.turn_id);
    let kinds = sink.kinds();
    // The person's typing approved it: under `Manual` no one was asked.
    assert!(!kinds
        .iter()
        .any(|kind| matches!(kind, AgentEventKind::ApprovalRequested { .. })));
    assert!(kinds.iter().any(|kind| matches!(
        kind,
        AgentEventKind::ToolStarted { call_id: id, name, .. } if *id == call_id && name == "exec_command"
    )));
    assert!(kinds.iter().any(|kind| matches!(
        kind,
        AgentEventKind::ToolCompleted { call_id: id, is_error: false } if *id == call_id
    )));
    let items = transcript(&service, session.id);
    let [ConversationItem::TurnStart { turn_id }, ConversationItem::UserShellCommand {
        call_id: item_call,
        command,
        cwd,
        exit_code,
        output,
        ..
    }] = items.as_slice()
    else {
        panic!("{items:#?}");
    };
    assert_eq!(*turn_id, turn.turn_id);
    assert_eq!(*item_call, call_id);
    assert_eq!(command, "echo user-shell-marker");
    assert!(cwd.is_some(), "it ran in the working directory");
    assert_eq!(*exit_code, Some(0));
    assert!(output.contains("user-shell-marker"), "{output}");
    // The interface reads it back as a command.
    let shown = service.tool_display(session.id, &call_id).unwrap().unwrap();
    assert!(matches!(
        shown.display,
        Some(AgentToolDisplay::Command { exit_code: Some(0), ref output, .. })
            if output.contains("user-shell-marker")
    ));

    service
        .start_turn(session.id, "What did it print?".to_string().into())
        .await
        .unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    let input = input_text(&body);
    assert!(
        input.contains(
            "<user_shell_command>\\n<command>\\necho user-shell-marker\\n</command>\\n<result>\\nExit code: 0\\nDuration: "
        ),
        "{input}"
    );
    assert!(input.contains("user-shell-marker\\n"), "{input}");
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn a_failing_command_records_its_exit_code() {
    let (service, paths) = configured_service("user-shell-exit", "http://127.0.0.1:1/v1");
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();

    let (_, _, end) = run(&service, session.id, "exit 3").await;

    assert!(
        matches!(end, AgentEventKind::TurnCompleted { .. }),
        "{end:?}"
    );
    let items = transcript(&service, session.id);
    assert!(
        matches!(
            items.last(),
            Some(ConversationItem::UserShellCommand {
                exit_code: Some(3),
                ..
            })
        ),
        "{items:#?}"
    );
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Refused while a turn runs and in Plan mode; once running, it reads no
/// steering, and interrupting it terminates its process, which is gone
/// (not left as a background terminal) before the turn's end is told.
#[tokio::test]
async fn it_runs_alone_and_an_interrupt_stops_its_process() {
    let server = FakeServer::start(vec![(assistant_events("Late."), Duration::from_secs(3))]).await;
    let (service, paths) = configured_service("user-shell-alone", &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let model_turn = service
        .submit_turn_with_sink(
            session.id,
            AgentTurnRequest {
                text: "Take your time.".to_string(),
                images: Vec::new(),
                skills: Vec::new(),
                host: Default::default(),
            },
            Arc::new(Recorded::default()),
        )
        .await
        .unwrap();
    let refused = service
        .submit_user_shell_command_with_sink(
            session.id,
            "echo too-early".to_string(),
            Arc::new(Recorded::default()),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("already running"), "{refused}");
    service
        .cancel_turn(session.id, model_turn.turn_id, model_turn.generation)
        .await
        .unwrap();

    let sink = Arc::new(Recorded::default());
    let turn = service
        .submit_user_shell_command_with_sink(session.id, "sleep 30".to_string(), sink.clone())
        .await
        .unwrap();
    tokio::time::timeout(TEST_WAIT, async {
        while service
            .list_background_terminals(session.id)
            .unwrap()
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the command must start");
    let steered = service
        .steer_turn_with_sink(
            session.id,
            turn.turn_id,
            turn.generation,
            AgentSteeringRequest {
                text: "And then?".to_string(),
                images: Vec::new(),
                host: Default::default(),
            },
            Arc::new(Recorded::default()),
        )
        .await
        .unwrap_err();
    assert!(steered.contains("reads no messages"), "{steered}");
    service
        .cancel_turn(session.id, turn.turn_id, turn.generation)
        .await
        .unwrap();

    let end = turn_end(&sink).await;
    assert!(
        matches!(end, AgentEventKind::TurnCancelled { .. }),
        "{end:?}"
    );
    assert!(service
        .list_background_terminals(session.id)
        .unwrap()
        .is_empty());
    // What the model will read of it: it did not finish.
    let items = transcript(&service, session.id);
    assert!(
        matches!(
            items.last(),
            Some(ConversationItem::UserShellCommand { exit_code: None, output, .. })
                if output == "The command did not finish."
        ),
        "{items:#?}"
    );

    let plan = service
        .create_session(AgentSessionMode::Plan, None)
        .await
        .unwrap();
    let refused = service
        .submit_user_shell_command_with_sink(
            plan.id,
            "echo planning".to_string(),
            Arc::new(Recorded::default()),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("Plan mode"), "{refused}");
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Asks a person for every shell command that mentions `needs-a-person`.
struct PersonRule;

impl ApprovalRule for PersonRule {
    fn id(&self) -> &str {
        "needs-a-person"
    }

    fn evaluate(
        &self,
        _origin: Option<AgentSessionOrigin>,
        action: &ApprovalAction,
    ) -> Option<ApprovalRuleVerdict> {
        (action.kind == ApprovalActionKind::ShellExecution
            && action.summary.contains("needs-a-person"))
        .then_some(ApprovalRuleVerdict::RequireUser)
    }
}

fn person_rule_plugin() -> PluginDef {
    PluginDef::new(|ctx: Context, _| async move {
        ctx.get::<ApprovalRuleRegistry>()?
            .register(Arc::new(PersonRule) as Arc<dyn ApprovalRule>)?;
        Ok(())
    })
    .name("person-rule")
    .inject(Inject::new().with(ApprovalRuleRegistry::NAME))
}

/// The approval rules still judge a typed command: one that requires a
/// person asks, and a refusal is what the command's item records.
#[tokio::test]
async fn an_approval_rule_requiring_a_person_still_asks() {
    let paths = test_paths("user-shell-rule");
    let config = AgentConfig::new(&paths.config_dir);
    configure_test_provider(&config, "http://127.0.0.1:1/v1", None);
    let service = AgentApplicationService::new_with_test_plugins(
        &paths,
        config,
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        vec![(
            AgentPluginRow::unconfigured(AgentPluginGroup::Capability, person_rule_plugin()),
            AgentPluginDescriptor::new("person-rule", "a test plugin"),
        )],
    )
    .unwrap();
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let sink = Arc::new(Recorded::default());
    let turn = service
        .submit_user_shell_command_with_sink(
            session.id,
            "echo needs-a-person".to_string(),
            sink.clone(),
        )
        .await
        .unwrap();
    let request = tokio::time::timeout(TEST_WAIT, async {
        loop {
            if let Some(request) = sink.kinds().into_iter().find_map(|kind| match kind {
                AgentEventKind::ApprovalRequested { request } => Some(request),
                _ => None,
            }) {
                return request;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the rule must ask");
    service
        .resolve_approval(
            session.id,
            turn.turn_id,
            turn.generation,
            request.request_id,
            ApprovalUserDecision::Deny,
        )
        .unwrap();

    let end = turn_end(&sink).await;
    assert!(
        matches!(end, AgentEventKind::TurnCompleted { .. }),
        "{end:?}"
    );
    let items = transcript(&service, session.id);
    assert!(
        matches!(
            items.last(),
            Some(ConversationItem::UserShellCommand { exit_code: None, output, .. })
                if !output.is_empty()
        ),
        "{items:#?}"
    );
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A new Agent over `paths`' storage, as after a restart.
fn reopen(config: AgentConfig, paths: &TestPaths) -> AgentApplicationService {
    AgentApplicationService::new_with_host(
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
    .unwrap()
}

/// A session that began with a command, and whose second command was
/// stopped, opens after a restart with both items as committed, and its
/// next request carries both.
#[tokio::test]
async fn a_session_of_commands_reopens_and_continues() {
    let server = FakeServer::start(vec![(assistant_events("Both seen."), Duration::ZERO)]).await;
    let (service, paths, config) = service_with_config("user-shell-reopen", |_, config| {
        configure_test_provider(config, &server.base_url, None);
    });
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    run(&service, session.id, "echo user-shell-reopened").await;
    let sink = Arc::new(Recorded::default());
    let stopped = service
        .submit_user_shell_command_with_sink(session.id, "sleep 120".to_string(), sink.clone())
        .await
        .unwrap();
    tokio::time::timeout(TEST_WAIT, async {
        while service
            .list_background_terminals(session.id)
            .unwrap()
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the command must start");
    service
        .cancel_turn(session.id, stopped.turn_id, stopped.generation)
        .await
        .unwrap();
    turn_end(&sink).await;
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );

    let reopened = reopen(config, &paths);
    reopened
        .start_turn(session.id, "What ran?".to_string().into())
        .await
        .unwrap();
    let items = transcript(&reopened, session.id);
    let commands = items
        .iter()
        .filter_map(|item| match item {
            ConversationItem::UserShellCommand {
                command, exit_code, ..
            } => Some((command.as_str(), *exit_code)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        commands,
        [("echo user-shell-reopened", Some(0)), ("sleep 120", None)],
        "{items:#?}"
    );
    let requests = server.requests();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    let input = input_text(&body);
    assert!(input.contains("user-shell-reopened\\n"), "{input}");
    assert!(
        input.contains("sleep 120\\n</command>\\n<result>\\nExit code: none"),
        "{input}"
    );
    assert!(input.contains("The command did not finish."), "{input}");
    assert_eq!(
        reopened.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Editing a message after a command forks before that message: the fork
/// keeps the command, and the message goes back to be edited.
#[tokio::test]
async fn editing_a_message_after_a_command_keeps_the_command() {
    let server = FakeServer::start(vec![
        (assistant_events("First."), Duration::ZERO),
        (assistant_events("Second."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("user-shell-edit", &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "First message.".to_string().into())
        .await
        .unwrap();
    run(&service, session.id, "echo between-messages").await;
    let second = service
        .start_turn(session.id, "Second message.".to_string().into())
        .await
        .unwrap();

    let edited = service.edit_from_turn(session.id, second.turn_id).unwrap();

    assert_eq!(edited.restored_prompt, "Second message.");
    let forked = transcript(&service, edited.session.id);
    assert!(
        matches!(
            forked.last(),
            Some(ConversationItem::UserShellCommand { command, .. }) if command == "echo between-messages"
        ),
        "{forked:#?}"
    );
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// What the person sent, for an interface's history search: newest first,
/// each text once, the commands as typed, bounded.
#[tokio::test]
async fn the_persons_recent_messages_include_commands_newest_first() {
    let server = FakeServer::start(vec![
        (assistant_events("One."), Duration::ZERO),
        (assistant_events("Two."), Duration::ZERO),
        (assistant_events("Three."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("user-shell-recent", &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    for text in ["First message.", "Second message."] {
        service
            .start_turn(session.id, text.to_string().into())
            .await
            .unwrap();
    }
    run(&service, session.id, "echo recent-marker").await;
    service
        .start_turn(session.id, "First message.".to_string().into())
        .await
        .unwrap();

    assert_eq!(
        service.recent_user_messages(10, None).unwrap(),
        ["First message.", "!echo recent-marker", "Second message."]
    );
    assert_eq!(
        service.recent_user_messages(1, None).unwrap(),
        ["First message."]
    );
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
