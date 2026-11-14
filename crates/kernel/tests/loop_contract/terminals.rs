//! A model's background terminals wake their session: the first terminal
//! status a tool's sink reports for an execution a model's call started
//! queues its end, with the call's command and the last output as plain
//! text; the disposal a call reports (its result told the model) withdraws
//! it, before any step reads it. A command that ends within its own call,
//! a `Cancelled` status, the Agent's own reports, a `!command`, a session
//! not woken or archived: nothing. The tools here stand in for the
//! execution plugin: `fake_exec` starts an execution and reports as the
//! plugin does (`end` says when it ends), keeping the sink a monitor would
//! report through; `fake_poll` is a `write_stdin` that sees the end.

use std::collections::HashMap;

use hanafubuki_agent_approval::PermissionRequirement;
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryContextEffect, AgentNotification, AgentObserver,
    AgentToolHandler, AgentToolProgressSink, ArgumentSubjects, ObserverRegistry, ToolDescriptor,
    ToolEffect, ToolFuture, ToolInvocation, ToolPresenterRegistry, ToolRegistry, ToolResult,
};
use hanafubuki_agent_kernel::{
    AgentExecutionProgressStatus, AgentTerminalWake, AgentToolProgressUpdate, AgentWakeEvent,
    AgentWakePolicy, AgentWakeSource, MAXIMUM_WAKE_OUTPUT_BYTES,
};
use hanafubuki_agent_tool_contracts::{ToolExecutionClass, ToolExposure, ToolName};

use super::*;

/// The sinks of the calls that started the executions still running, by
/// execution: what their monitors report through.
type Monitors = Arc<Mutex<HashMap<u64, Arc<dyn AgentToolProgressSink>>>>;

struct FakeExecution {
    descriptor: ToolDescriptor,
    monitors: Monitors,
    /// `fake_poll`'s: the notifications of queued events it waits on.
    poll: Option<Arc<QueuedNotifications>>,
}

impl FakeExecution {
    fn handler(
        name: &str,
        monitors: &Monitors,
        poll: Option<Arc<QueuedNotifications>>,
    ) -> Arc<dyn AgentToolHandler> {
        Arc::new(Self {
            descriptor: ToolDescriptor {
                name: ToolName::parse(name).unwrap(),
                description: format!("{name} fixture"),
                input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": true }),
                exposure: ToolExposure::Direct,
                execution_class: ToolExecutionClass::ConcurrentRead,
                effect: ToolEffect::ReadOnly,
                action: AgentActivityAction::fixed("exec_command"),
                memory_effect: AgentMemoryContextEffect::None,
                timeout: None,
            },
            monitors: monitors.clone(),
            poll,
        })
    }
}

impl AgentToolHandler for FakeExecution {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn permission<'a>(
        &'a self,
        _invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        Box::pin(async { Ok(PermissionRequirement::NotRequired) })
    }

    fn invoke(
        &self,
        invocation: ToolInvocation,
        _approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async move {
            let id = invocation.arguments["id"].as_u64().unwrap();
            let progress = invocation.progress.clone();
            if let Some(notified) = &self.poll {
                // The monitor sees the end, which is queued; only then does
                // the poll's result report it and dispose of the execution:
                // the step that follows must not read that end.
                let monitor = self.monitors.lock().unwrap().remove(&id).unwrap();
                monitor.report(output(id, "the rest\n"));
                monitor.report(status(id, AgentExecutionProgressStatus::Exited, Some(0)));
                tokio::time::timeout(Duration::from_secs(20), async {
                    while *notified.0.lock().unwrap() == 0 {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                })
                .await
                .expect("the monitor's end is queued");
                progress.report(disposed(id));
                return Ok(ToolResult::success(json!({ "exit_code": 0 })));
            }
            progress.report(AgentToolProgressUpdate::ExecutionStarted {
                execution_id: id,
                generation: 1,
                action: AgentActivityAction::fixed("exec_command"),
                status: AgentExecutionProgressStatus::Running,
            });
            match invocation.arguments["end"].as_str() {
                Some("within") => {
                    progress.report(output(id, "done\n"));
                    progress.report(status(id, AgentExecutionProgressStatus::Exited, Some(0)));
                    progress.report(disposed(id));
                    Ok(ToolResult::success(json!({ "exit_code": 0 })))
                }
                // It ended after the call's wait returned its session,
                // before the call's result was committed.
                Some("after_yield") => {
                    progress.report(status(id, AgentExecutionProgressStatus::Exited, Some(0)));
                    Ok(ToolResult::success(json!({ "session_id": id })))
                }
                _ => {
                    self.monitors.lock().unwrap().insert(id, progress);
                    Ok(ToolResult::success(json!({ "session_id": id })))
                }
            }
        })
    }
}

#[derive(Default)]
struct QueuedNotifications(Mutex<usize>);

impl AgentObserver for QueuedNotifications {
    fn id(&self) -> &str {
        "queued-notifications"
    }

    fn observe(&self, notification: &AgentNotification) {
        if matches!(notification, AgentNotification::WakeEventQueued { .. }) {
            *self.0.lock().unwrap() += 1;
        }
    }
}

struct Fixture {
    _tools: FakeTools,
    service: AgentService,
    model: ScriptedEventModel,
    session_id: AgentSessionId,
    monitors: Monitors,
    notified: Arc<QueuedNotifications>,
}

/// A session woken for its background work over `responses`, with the two
/// tools, `fake_exec`'s presenter (its `cmd`) and an observer counting the
/// notifications of queued events.
async fn fixture(responses: Vec<Vec<ModelResult<ModelEvent>>>) -> Fixture {
    let monitors = Monitors::default();
    let notified = Arc::new(QueuedNotifications::default());
    let tools = FakeTools::new([]).await;
    tools
        .mount(&registering::<ToolRegistry>(
            "fake-executions",
            vec![
                FakeExecution::handler("fake_exec", &monitors, None),
                FakeExecution::handler("fake_poll", &monitors, Some(notified.clone())),
            ],
        ))
        .await;
    tools
        .mount(&registering::<ToolPresenterRegistry>(
            "fake-execution-presenter",
            vec![Arc::new(ArgumentSubjects::new(&[("fake_exec", "cmd")]))
                as Arc<dyn hanafubuki_agent_contracts::AgentToolPresenter>],
        ))
        .await;
    tools
        .mount(&registering::<ObserverRegistry>(
            "queued-notifications",
            vec![notified.clone() as Arc<dyn AgentObserver>],
        ))
        .await;
    let model = ScriptedEventModel::new(responses);
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session_id = service.create_session(config()).unwrap().id;
    service
        .set_session_wake_policy(session_id, AgentWakePolicy::On)
        .await
        .unwrap();
    Fixture {
        _tools: tools,
        service,
        model,
        session_id,
        monitors,
        notified,
    }
}

impl Fixture {
    async fn turn(&self, text: &str) -> String {
        self.service
            .start_turn(self.session_id, text.to_string())
            .await
            .unwrap()
            .final_answer
    }

    /// Reports `updates` through the sink of the call that started
    /// execution `id`, as its monitor does, and waits until they are
    /// applied.
    async fn monitor(&self, id: u64, updates: &[AgentToolProgressUpdate]) {
        let sink = self.monitors.lock().unwrap().get(&id).cloned().unwrap();
        for update in updates {
            sink.report(update.clone());
        }
        self.flush().await;
    }

    async fn flush(&self) {
        self.service
            .flush_session_execution_progress(self.session_id)
            .await
            .unwrap();
    }

    fn queued(&self) -> Vec<AgentWakeSource> {
        self.service
            .test_wake_queue(self.session_id)
            .unwrap()
            .into_iter()
            .map(|event: AgentWakeEvent| event.source)
            .collect()
    }

    fn notified(&self) -> usize {
        *self.notified.0.lock().unwrap()
    }

    /// The `<background_events>` fragments of every request so far.
    fn fragments(&self) -> Vec<String> {
        self.model
            .requests()
            .iter()
            .flat_map(|request| request.input.clone())
            .filter_map(|item| match item {
                ModelInputItem::ContextFragment { kind, content, .. }
                    if kind == "background_events" =>
                {
                    Some(content)
                }
                _ => None,
            })
            .collect()
    }

    /// Admits the turn that delivers the queue and waits for its end.
    async fn woken(&self) {
        let accepted = self
            .service
            .submit_wake_turn_with_sink(
                self.session_id,
                AgentTurnInput::from(String::new()),
                Arc::new(NoopEventSink),
            )
            .unwrap()
            .expect("events are queued");
        tokio::time::timeout(Duration::from_secs(20), async {
            while !self
                .service
                .snapshot(self.session_id)
                .unwrap()
                .turn
                .is_some_and(|turn| turn.id == accepted.turn.turn_id && turn.phase.is_terminal())
            {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("the woken turn ends");
    }
}

fn exec(call_id: &str, id: u64, cmd: &str, end: &str) -> (String, String, serde_json::Value) {
    (
        call_id.to_string(),
        "fake_exec".to_string(),
        json!({ "cmd": cmd, "id": id, "end": end }),
    )
}

fn calls(calls: &[(String, String, serde_json::Value)]) -> Vec<ModelResult<ModelEvent>> {
    let calls = calls
        .iter()
        .map(|(call_id, name, arguments)| (call_id.as_str(), name.as_str(), arguments.clone()))
        .collect::<Vec<_>>();
    calls_events(&calls)
}

fn output(id: u64, text: &str) -> AgentToolProgressUpdate {
    AgentToolProgressUpdate::ExecutionOutput {
        execution_id: id,
        generation: 1,
        text: text.to_string(),
        next_offset: text.len() as u64,
        truncated: false,
    }
}

fn status(
    id: u64,
    status: AgentExecutionProgressStatus,
    exit_code: Option<i32>,
) -> AgentToolProgressUpdate {
    AgentToolProgressUpdate::ExecutionStatus {
        execution_id: id,
        generation: 1,
        status,
        exit_code,
        cleanup_required: false,
    }
}

fn disposed(id: u64) -> AgentToolProgressUpdate {
    AgentToolProgressUpdate::ExecutionDisposed {
        execution_id: id,
        generation: 1,
    }
}

/// The round trip: a command still running when its call returned is a
/// background terminal the session waits on; its end (reported twice, as
/// the start's and the monitor's reports can) queues one event, told once,
/// with its command, how it ended and its last output as plain text; the
/// turn it opens reads it.
#[tokio::test]
async fn a_terminal_that_ends_after_its_turn_is_queued_once_with_its_end() {
    let fixture = fixture(vec![
        calls(&[exec("call-exec", 7, "cargo test --workspace", "yield")]),
        deterministic_assistant_events("started"),
        deterministic_assistant_events("the tests failed"),
    ])
    .await;
    assert_eq!(fixture.turn("run the tests").await, "started");
    let service = &fixture.service;
    assert_eq!(service.background_terminals(fixture.session_id), 1);
    assert_eq!(service.background_work(fixture.session_id).terminals, 1);
    assert!(fixture.queued().is_empty());

    let failed = status(7, AgentExecutionProgressStatus::Exited, Some(101));
    fixture
        .monitor(
            7,
            &[
                output(7, "running 3 tests\n\u{1b}[31mFAILED\u{1b}[0m it_works\n"),
                failed.clone(),
                failed,
            ],
        )
        .await;

    assert_eq!(service.background_terminals(fixture.session_id), 0);
    let queued = fixture.queued();
    let [AgentWakeSource::Terminal {
        execution_id: 7,
        command,
        event:
            AgentTerminalWake::Ended {
                status: AgentExecutionProgressStatus::Exited,
                exit_code: Some(101),
                output,
                truncated: false,
                ..
            },
    }] = &queued[..]
    else {
        panic!("one end is queued: {queued:?}");
    };
    assert_eq!(command, "cargo test --workspace");
    assert_eq!(output, "running 3 tests\nFAILED it_works");
    assert_eq!(fixture.notified(), 1);

    fixture.woken().await;
    let fragments = fixture.fragments();
    assert_eq!(fragments.len(), 1, "{fragments:?}");
    assert!(
        fragments[0].contains("Terminal session 7 exited with code 101 after ")
            && fragments[0].contains("<command>\ncargo test --workspace\n</command>")
            && fragments[0].contains(
                "Its last output (poll session 7 for the rest):\n<output>\nrunning 3 tests\nFAILED it_works\n</output>"
            ),
        "{}",
        fragments[0]
    );
    assert!(fixture.queued().is_empty());
}

/// A command that ends within its own call is that call's result: its end
/// is withdrawn by the call's disposal before any step reads it, and
/// nothing is left queued.
#[tokio::test]
async fn a_command_that_ends_within_its_call_queues_nothing() {
    let fixture = fixture(vec![
        calls(&[exec("call-exec", 3, "ls", "within")]),
        deterministic_assistant_events("listed"),
    ])
    .await;
    assert_eq!(fixture.turn("list").await, "listed");

    assert!(fixture.queued().is_empty());
    assert!(fixture.fragments().is_empty());
    assert_eq!(fixture.service.background_terminals(fixture.session_id), 0);
}

/// A command that ended after its call returned its session, but before
/// the call's result was committed, is a terminal the model was told runs:
/// its end reaches the turn's next step, after that result.
#[tokio::test]
async fn an_end_just_after_the_call_returned_its_session_reaches_the_next_step() {
    let fixture = fixture(vec![
        calls(&[exec("call-exec", 4, "make", "after_yield")]),
        deterministic_assistant_events("it already ended"),
    ])
    .await;
    assert_eq!(fixture.turn("build").await, "it already ended");

    let requests = fixture.model.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        matches!(
            &requests[1].input[..],
            [.., ModelInputItem::ToolOutput { .. }, ModelInputItem::ContextFragment { kind, content, .. }]
                if kind == "background_events" && content.contains("Terminal session 4 exited with code 0")
        ),
        "{:?}",
        requests[1].input
    );
    assert!(fixture.queued().is_empty());
}

/// A poll that sees a terminal end tells the model in its result: the end
/// its monitor queued (here before the poll's disposal is even reported)
/// is withdrawn before the next step, which reads no update, and nothing
/// is left to wake the session afterwards.
#[tokio::test]
async fn a_poll_that_saw_the_end_withdraws_it() {
    let fixture = fixture(vec![
        calls(&[exec("call-exec", 5, "npm run build", "yield")]),
        deterministic_assistant_events("started"),
        calls(&[(
            "call-poll".to_string(),
            "fake_poll".to_string(),
            json!({ "id": 5 }),
        )]),
        deterministic_assistant_events("built"),
    ])
    .await;
    assert_eq!(fixture.turn("build").await, "started");
    assert_eq!(fixture.turn("is it done?").await, "built");

    assert_eq!(fixture.notified(), 1, "the end was queued, then withdrawn");
    assert!(fixture.fragments().is_empty(), "{:?}", fixture.fragments());
    assert!(fixture.queued().is_empty());
    assert_eq!(fixture.service.background_terminals(fixture.session_id), 0);
}

/// Two terminals that end while the session waits are two events, read by
/// one turn as one update.
#[tokio::test]
async fn stale_disposal_and_duplicate_start_preserve_the_tracked_terminal() {
    let fixture = fixture(vec![
        calls(&[exec("call-exec", 7, "make", "yield")]),
        deterministic_assistant_events("started"),
    ])
    .await;
    fixture.turn("build").await;
    fixture
        .monitor(
            7,
            &[AgentToolProgressUpdate::ExecutionDisposed {
                execution_id: 7,
                generation: 2,
            }],
        )
        .await;
    assert_eq!(fixture.service.background_terminals(fixture.session_id), 1);
    fixture
        .monitor(
            7,
            &[status(7, AgentExecutionProgressStatus::Exited, Some(0))],
        )
        .await;
    assert_eq!(fixture.queued().len(), 1);
    fixture
        .monitor(
            7,
            &[
                AgentToolProgressUpdate::ExecutionStarted {
                    execution_id: 7,
                    generation: 1,
                    action: AgentActivityAction::fixed("exec_command"),
                    status: AgentExecutionProgressStatus::Running,
                },
                disposed(7),
            ],
        )
        .await;
    assert!(
        fixture.queued().is_empty(),
        "the duplicate start did not lose withdrawal state"
    );
    assert_eq!(fixture.service.background_terminals(fixture.session_id), 0);
}

#[tokio::test]
async fn two_ends_are_read_together() {
    let fixture = fixture(vec![
        calls(&[
            exec("call-a", 8, "cargo build", "yield"),
            exec("call-b", 9, "cargo doc", "yield"),
        ]),
        deterministic_assistant_events("started both"),
        deterministic_assistant_events("both ended"),
    ])
    .await;
    fixture.turn("build and document").await;
    assert_eq!(fixture.service.background_terminals(fixture.session_id), 2);
    fixture
        .monitor(
            8,
            &[status(8, AgentExecutionProgressStatus::Exited, Some(0))],
        )
        .await;
    fixture
        .monitor(
            9,
            &[status(9, AgentExecutionProgressStatus::TimedOut, None)],
        )
        .await;
    assert_eq!(fixture.queued().len(), 2);

    fixture.woken().await;
    let fragments = fixture.fragments();
    assert_eq!(fragments.len(), 1);
    assert!(
        fragments[0].contains("Terminal session 8 exited with code 0 after ")
            && fragments[0].contains("Terminal session 9 timed out after ")
            && fragments[0].contains("seconds and was stopped:\n<command>\ncargo doc\n</command>"),
        "{}",
        fragments[0]
    );
}

/// Ends that tell the model nothing new queue nothing: a `Cancelled`
/// status (the Agent's own stops), the Agent's own report of an end (it
/// names no call), a `!command`'s end, any end while the session is not
/// woken or archived. A failure says so.
#[tokio::test]
async fn ends_the_agent_caused_or_no_one_waits_for_queue_nothing() {
    let fixture = fixture(Vec::new()).await;
    let service = &fixture.service;
    let session_id = fixture.session_id;
    let started = |id| AgentToolProgressUpdate::ExecutionStarted {
        execution_id: id,
        generation: 1,
        action: AgentActivityAction::fixed("exec_command"),
        status: AgentExecutionProgressStatus::Running,
    };
    let report = |call: &str, updates: Vec<AgentToolProgressUpdate>| {
        let sink = service.execution_progress_sink(session_id, Some(call.to_string()));
        for update in updates {
            sink.report(update);
        }
    };

    report(
        "call-1",
        vec![
            started(1),
            status(1, AgentExecutionProgressStatus::Cancelled, None),
        ],
    );
    report(
        "user-shell-3",
        vec![
            started(2),
            status(2, AgentExecutionProgressStatus::Exited, Some(0)),
        ],
    );
    report("call-3", vec![started(3)]);
    fixture.flush().await;
    assert_eq!(
        service.background_terminals(session_id),
        1,
        "a `!command` is no terminal"
    );
    let agent = service.clone();
    tokio::task::spawn_blocking(move || {
        agent
            .report_session_execution_progress(
                session_id,
                status(3, AgentExecutionProgressStatus::Exited, Some(0)),
            )
            .unwrap()
    })
    .await
    .unwrap();
    // A later report of the same end from the monitor is no new end.
    report(
        "call-3",
        vec![status(3, AgentExecutionProgressStatus::Exited, Some(0))],
    );
    fixture.flush().await;
    assert!(fixture.queued().is_empty());
    assert_eq!(service.background_terminals(session_id), 0);

    service
        .set_session_wake_policy(session_id, AgentWakePolicy::Off)
        .await
        .unwrap();
    report(
        "call-4",
        vec![
            started(4),
            status(4, AgentExecutionProgressStatus::Exited, Some(1)),
        ],
    );
    fixture.flush().await;
    assert!(fixture.queued().is_empty());
    service
        .set_session_wake_policy(session_id, AgentWakePolicy::On)
        .await
        .unwrap();

    report(
        "call-5",
        vec![
            started(5),
            status(5, AgentExecutionProgressStatus::Failed, None),
        ],
    );
    fixture.flush().await;
    let queued = fixture.queued();
    assert!(
        matches!(
            &queued[..],
            [AgentWakeSource::Terminal {
                execution_id: 5,
                event: AgentTerminalWake::Ended {
                    status: AgentExecutionProgressStatus::Failed,
                    ..
                },
                ..
            }]
        ),
        "{queued:?}"
    );
    assert_eq!(fixture.notified(), 1);

    let snapshot = service.snapshot(session_id).unwrap();
    service
        .archive_session(session_id, snapshot.generation, snapshot.revision)
        .unwrap();
    report(
        "call-6",
        vec![
            started(6),
            status(6, AgentExecutionProgressStatus::Exited, Some(0)),
        ],
    );
    fixture.flush().await;
    assert_eq!(
        fixture.queued().len(),
        1,
        "an archived session queues nothing"
    );
}

/// The Agent's own disposal of a terminal whose end is queued (a stop, or
/// a start settling what the last run left) told the model nothing: the end
/// stays queued.
#[tokio::test]
async fn the_agents_own_disposal_keeps_a_queued_end() {
    let fixture = fixture(vec![
        calls(&[exec("call-exec", 6, "make bench", "yield")]),
        deterministic_assistant_events("started"),
    ])
    .await;
    fixture.turn("bench").await;
    fixture
        .monitor(
            6,
            &[status(6, AgentExecutionProgressStatus::Exited, Some(0))],
        )
        .await;
    let agent = fixture.service.clone();
    let session_id = fixture.session_id;
    tokio::task::spawn_blocking(move || {
        agent
            .report_session_execution_progress(session_id, disposed(6))
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(fixture.queued().len(), 1);
}

/// A terminal's output is read as the activity shows it, plain text, and
/// its last `MAXIMUM_WAKE_OUTPUT_BYTES` are kept, marked as cut; a long
/// command is its presentation's first line, bounded.
#[tokio::test]
async fn a_long_output_and_a_long_command_are_bounded() {
    let command = format!("echo {}\nexit 1", "x".repeat(5_000));
    let fixture = fixture(vec![
        calls(&[exec("call-exec", 7, &command, "yield")]),
        deterministic_assistant_events("started"),
    ])
    .await;
    fixture.turn("print").await;
    let lines = (0..40)
        .map(|line| format!("{line:02} {}\n", "y".repeat(600)))
        .collect::<String>();
    fixture
        .monitor(
            7,
            &[
                output(7, &lines),
                status(7, AgentExecutionProgressStatus::Exited, Some(1)),
            ],
        )
        .await;

    let queued = fixture.queued();
    let [AgentWakeSource::Terminal {
        command,
        event: AgentTerminalWake::Ended {
            output, truncated, ..
        },
        ..
    }] = &queued[..]
    else {
        panic!("one end is queued: {queued:?}");
    };
    assert!(command.starts_with("echo xxx") && command.ends_with('…'));
    assert!(command.chars().count() <= 201, "{}", command.len());
    assert!(*truncated);
    assert!(
        output.len() <= MAXIMUM_WAKE_OUTPUT_BYTES,
        "{}",
        output.len()
    );
    assert!(
        output.starts_with("3") && output.contains("39 yyy"),
        "{output}"
    );
    assert!(!output.contains("00 yyy"));
}
