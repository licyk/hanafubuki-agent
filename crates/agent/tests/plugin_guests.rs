//! A code plugin end to end: installed from a directory, granted, given
//! settings, its guest started by the `plugins` row under the platform's
//! sandbox; then a turn against the scripted model, whose request carries
//! the guest's context section and prompt fragment, which calls the guest's
//! tool, and whose next request carries the tool's output and the guest's
//! hook context; the guest observes the turn's end. The tool's call is
//! approved like an MCP tool's, by the turn's permission mode: the user in
//! `Manual`, the reviewer model in `AutoReview`, no one in `FullAccess`.
//! A guest that crashes in a tool call is that call's error: the turn ends
//! with its answer, and the guest comes back. The changed package installed
//! again from its directory (what `plugins dev` does on a change) starts
//! anew with the change. The same plugin linked instead runs from its
//! directory, and starts anew on its own once a file there changes.
//!
//! This binary is also the guest (kardia-guest's `serve`, with
//! [`GUEST_ARG`]) and the sandbox helper, so the guest it installs, and
//! confines, re-executes it. It runs where the sandbox's plugin profile was
//! checked (Linux), and skips elsewhere.

use std::time::{Duration, Instant};

use hanafubuki_agent::test_support::{
    assistant_events, configure_test_provider, namespace_tool_events, service_with_host, test_host,
    FakeServer, TEST_SHUTDOWN_DEADLINE, TEST_WAIT,
};
use hanafubuki_agent::{
    AgentApplicationService, AgentEventKind, AgentPackageSource, AgentPackageStatus,
    AgentPermissionMode, AgentSessionId, AgentSessionMode, AgentSettingsPatch, AgentTurnInput,
    AgentTurnOutcome, ApprovalActionKind, ApprovalUserDecision, GuestState, PackagePermissions,
    WorkspaceAccess,
};
use kardia_guest::{json, Guest, Host, Map, Meta, Request, Result, Value};

const GUEST_ARG: &str = "--hanafubuki-facade-guest";
const ID: &str = "demo@local";

fn main() {
    hanafubuki_agent::sandbox::helper_dispatch();
    if std::env::args().nth(1).as_deref() == Some(GUEST_ARG) {
        kardia_guest::serve::<DemoGuest>();
        return;
    }
    if !cfg!(target_os = "linux") {
        eprintln!("skipped: a sandboxed guest is checked on Linux only");
        return;
    }
    if let hanafubuki_agent::sandbox::SandboxStatus::Unavailable { reason } =
        hanafubuki_agent::sandbox::Sandbox::detect().status()
    {
        eprintln!("skipped: the sandbox is unavailable here ({reason})");
        return;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(a_code_plugin_contributes_to_a_turn_and_its_crash_stays_its_own());
    eprintln!("ok: a_code_plugin_contributes_to_a_turn_and_its_crash_stays_its_own");
    runtime.block_on(a_linked_code_plugin_starts_anew_once_its_directory_changes());
    eprintln!("ok: a_linked_code_plugin_starts_anew_once_its_directory_changes");
}

/// The plugin's guest: its tools echo or crash, its section and prompt
/// name its settings, its hook adds context after a call, and it writes
/// what it observes to its data directory, its working directory.
struct DemoGuest {
    board: String,
}

impl Guest for DemoGuest {
    type Config = Value;

    fn meta() -> Meta {
        Meta::plugin("demo").provide("plugin")
    }

    fn apply(host: &Host, config: Value, _imports: Map<String, Value>) -> Result<Self> {
        host.provide("plugin", &json!({}))?;
        let board = config["board"].as_str().unwrap_or("none").to_string();
        Ok(Self { board })
    }

    fn call(&mut self, _host: &Host, request: Request) -> Result<Value> {
        let args = &request.args;
        match request.method.as_str() {
            "tool" if args["name"] == "crash" => std::process::exit(9),
            "tool" => Ok(json!({ "echoed": args["arguments"] })),
            "context" => Ok(json!(format!(
                "Board {} is open ({}).",
                self.board,
                args["kind"].as_str().unwrap_or_default()
            ))),
            "prompt" => Ok(json!(format!("Use board {} for plugin work.", self.board))),
            "beforeToolUse" => Ok(Value::Null),
            "afterToolUse" => Ok(json!({
                "context": [format!("The demo plugin saw {}.", args["call"]["toolName"].as_str().unwrap_or_default())]
            })),
            "observe" => {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("observed.jsonl")?;
                writeln!(file, "{args}")?;
                Ok(Value::Null)
            }
            method => Err(format!("no method {method}").into()),
        }
    }
}

fn write(path: &std::path::Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// The demo plugin's manifest at `source`: this binary as its guest, its
/// `version` and its context sections of `kinds`.
fn write_manifest(source: &std::path::Path, version: &str, kinds: &[&str]) {
    let program = std::env::current_exe().unwrap();
    let sections: Vec<Value> = kinds.iter().map(|kind| json!({ "kind": kind })).collect();
    write(
        &source.join(".hanafubuki-plugin/plugin.json"),
        &json!({
            "name": "demo",
            "version": version,
            "runtime": {
                "type": "process",
                "command": [program.to_str().unwrap(), GUEST_ARG],
                "interpreter": true,
            },
            "contributes": {
                "tools": [
                    { "name": "echo", "description": "Echoes.", "inputSchema": { "type": "object" } },
                    { "name": "crash", "description": "Crashes.", "inputSchema": { "type": "object" } },
                ],
                "contextSections": sections,
                "prompt": { "slot": "capabilities" },
                "toolLifecycleHooks": true,
                "observers": ["turn_finished"],
            },
            "permissions": { "workspace": "none" },
            "configSchema": {
                "type": "object",
                "properties": { "board": { "type": "string" } },
            },
        })
        .to_string(),
    );
}

async fn package(service: &AgentApplicationService) -> AgentPackageStatus {
    service
        .packages()
        .await
        .unwrap()
        .into_iter()
        .find(|package| package.id == ID)
        .unwrap()
}

/// The package once its guest runs a process other than `before`'s.
async fn running(service: &AgentApplicationService, before: Option<u32>) -> AgentPackageStatus {
    let deadline = Instant::now() + TEST_WAIT;
    loop {
        let package = package(service).await;
        let status = &package.guest.as_ref().unwrap().status;
        if status.state == GuestState::Running
            && status.process_id.is_some()
            && status.process_id != before
        {
            return package;
        }
        assert!(Instant::now() < deadline, "{package:#?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Runs a turn on `text` in `Manual`, whose one tool call the user allows
/// once.
async fn turn_approved(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
    text: &str,
) -> AgentTurnOutcome {
    let asked = approvals(service, session_id).len();
    let running = service.clone();
    let input = AgentTurnInput {
        permission_mode: Some(AgentPermissionMode::Manual),
        ..text.into()
    };
    let turn = tokio::spawn(async move { running.start_turn(session_id, input).await });
    let deadline = Instant::now() + TEST_WAIT;
    let request = loop {
        if let Some(request) = approvals(service, session_id).into_iter().nth(asked) {
            break request;
        }
        assert!(Instant::now() < deadline, "no approval asked for");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(request.action.kind, ApprovalActionKind::PluginToolCall);
    let identity = service.snapshot(session_id).unwrap().turn.unwrap();
    service
        .resolve_approval(
            session_id,
            identity.id,
            identity.generation,
            request.request_id,
            ApprovalUserDecision::AllowOnce,
        )
        .unwrap();
    tokio::time::timeout(TEST_WAIT, turn)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

fn approvals(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
) -> Vec<hanafubuki_agent::ApprovalRequest> {
    service
        .events_since(session_id, None)
        .unwrap()
        .events
        .into_iter()
        .filter_map(|event| match event.kind {
            AgentEventKind::ApprovalRequested { request } => Some(request),
            _ => None,
        })
        .collect()
}

async fn a_code_plugin_contributes_to_a_turn_and_its_crash_stays_its_own() {
    let server = FakeServer::start(
        [
            namespace_tool_events("demo", "echo", r#"{"q":"x"}"#),
            assistant_events("The plugin answered."),
            namespace_tool_events("demo", "echo", r#"{"q":"y"}"#),
            assistant_events(&review_allows()),
            assistant_events("The reviewer allowed it."),
            namespace_tool_events("demo", "crash", "{}"),
            assistant_events("The plugin crashed; going on."),
            assistant_events("The plugin was updated."),
        ]
        .into_iter()
        .map(|body| (body, Duration::ZERO))
        .collect(),
    )
    .await;
    let (service, paths, _) = service_with_host("plugin-guests", test_host, |_, config| {
        configure_test_provider(config, &server.base_url, None);
        // Its title requests would take the scripted answers.
        config.set_plugin_disabled("titles", true).unwrap();
        // A session given no mode of its own runs in this one.
        config
            .write_settings(AgentSettingsPatch {
                permission_mode: Some(AgentPermissionMode::FullAccess),
                ..AgentSettingsPatch::default()
            })
            .unwrap();
    });
    let source = paths.app_data_dir.join("sources/demo");
    write_manifest(&source, "1.0.0", &["demo_board"]);
    service.start_plugins().await;
    service
        .install_package(AgentPackageSource::Directory {
            path: source.clone(),
            marketplace: None,
        })
        .await
        .unwrap();
    let granted = PackagePermissions {
        network: false,
        workspace: WorkspaceAccess::None,
        env: Vec::new(),
    };
    service
        .grant_package_permissions(ID, granted)
        .await
        .unwrap();
    let refused = service
        .set_package_settings(ID, json!({ "board": 7 }))
        .await
        .unwrap_err();
    assert!(refused.contains("configSchema"), "{refused}");
    service
        .set_package_settings(ID, json!({ "board": "B-7" }))
        .await
        .unwrap();
    let first = running(&service, None).await;
    let guest = first.guest.as_ref().unwrap();
    assert_eq!(
        guest.settings,
        Map::from_iter([("board".into(), json!("B-7"))])
    );
    assert!(guest.config_schema.is_some());
    assert!(guest.status.issues.is_empty(), "{:?}", guest.status.issues);

    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let outcome = turn_approved(&service, session.id, "Use the demo plugin.").await;
    assert_eq!(outcome.final_answer, "The plugin answered.");
    let requests = server.requests();
    let first_request = String::from_utf8(requests[0].body.clone()).unwrap();
    assert!(
        first_request.contains("Board B-7 is open (demo_board)."),
        "{first_request}"
    );
    assert!(first_request.contains("Use board B-7 for plugin work."));
    assert!(first_request.contains("\"demo\""), "{first_request}");
    let second_request = String::from_utf8(requests[1].body.clone()).unwrap();
    assert!(second_request.contains("echoed"), "{second_request}");
    assert!(second_request.contains("The demo plugin saw demo__echo."));
    let observed = paths
        .data_dir
        .join("agent-test/plugins/data/demo-local/observed.jsonl");
    let deadline = Instant::now() + TEST_WAIT;
    while !std::fs::read_to_string(&observed)
        .unwrap_or_default()
        .contains("turn_finished")
    {
        assert!(Instant::now() < deadline, "no turn_finished observed");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // In `AutoReview` the reviewer model allows the call; no one is asked.
    service
        .set_session_permission_mode(session.id, AgentPermissionMode::AutoReview)
        .unwrap();
    let outcome = tokio::time::timeout(
        TEST_WAIT,
        service.start_turn(session.id, "Use the demo plugin again.".into()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(outcome.final_answer, "The reviewer allowed it.");
    assert_eq!(approvals(&service, session.id).len(), 1);
    let review = String::from_utf8(server.requests()[3].body.clone()).unwrap();
    assert!(
        review.contains("Call the tool echo of the plugin demo@local"),
        "{review}"
    );
    let reviewed = String::from_utf8(server.requests()[4].body.clone()).unwrap();
    assert!(reviewed.contains("echoed"), "{reviewed}");

    // In `FullAccess`, here the configured mode of a session given none,
    // the call runs without asking. A crash in it is that call's error; the
    // turn goes on.
    let pid = first.guest.as_ref().unwrap().status.process_id;
    let unset = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let outcome = tokio::time::timeout(
        TEST_WAIT,
        service.start_turn(unset.id, "Crash the demo plugin.".into()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(outcome.final_answer, "The plugin crashed; going on.");
    let turn = service.snapshot(unset.id).unwrap().turn.unwrap();
    assert_eq!(turn.permission_mode, AgentPermissionMode::FullAccess);
    assert!(approvals(&service, unset.id).is_empty());
    let after_crash = String::from_utf8(server.requests()[6].body.clone()).unwrap();
    assert!(
        after_crash.contains("the plugin demo@local failed"),
        "{after_crash}"
    );
    let back = running(&service, pid).await;
    assert_eq!(back.guest.as_ref().unwrap().status.restarts, 1);

    // `plugins dev`'s reload: the changed directory installed again. The
    // guest starts anew with the change, its grant and settings kept.
    write_manifest(&source, "1.0.1", &["demo_board", "demo_notes"]);
    let updated = service.update_package(ID).await.unwrap();
    assert_eq!(updated.package.version, "1.0.1");
    assert_eq!(updated.permissions, None);
    let pid = back.guest.unwrap().status.process_id;
    let reloaded = running(&service, pid).await;
    let guest = reloaded.guest.as_ref().unwrap();
    assert_eq!(guest.settings["board"], "B-7");
    assert!(guest.status.issues.is_empty(), "{:?}", guest.status.issues);
    let outcome = service
        .start_turn(session.id, "Is the plugin updated?".into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "The plugin was updated.");
    let after_update = String::from_utf8(server.requests().last().unwrap().body.clone()).unwrap();
    assert!(
        after_update.contains("Board B-7 is open (demo_notes)."),
        "{after_update}"
    );

    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

async fn a_linked_code_plugin_starts_anew_once_its_directory_changes() {
    let (service, paths, _) = service_with_host("plugin-guests-linked", test_host, |_, _| {});
    let source = paths.app_data_dir.join("sources/demo");
    write_manifest(&source, "1.0.0", &["demo_board"]);
    service.start_plugins().await;
    let linked = service
        .install_package(AgentPackageSource::Linked {
            path: source.clone(),
            marketplace: None,
        })
        .await
        .unwrap();
    let permissions = linked.permissions.unwrap();
    service
        .grant_package_permissions(ID, permissions)
        .await
        .unwrap();
    let first = running(&service, None).await;
    let pid = first.guest.unwrap().status.process_id;

    // The OS watch is armed on its own thread: a change made before it is
    // lost, so it is made again until the guest starts anew.
    let deadline = Instant::now() + TEST_WAIT;
    let mut attempt = 0;
    let restarted = 'changes: loop {
        attempt += 1;
        write(&source.join("notes.txt"), &format!("change {attempt}"));
        let retry = Instant::now() + Duration::from_secs(3);
        while Instant::now() < retry {
            let package = package(&service).await;
            let status = &package.guest.as_ref().unwrap().status;
            if status.state == GuestState::Running
                && status.process_id.is_some()
                && status.process_id != pid
            {
                break 'changes package;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(Instant::now() < deadline, "{:#?}", package(&service).await);
    };
    let guest = restarted.guest.as_ref().unwrap();
    assert_eq!(guest.status.restarts, 0);
    assert!(guest.status.issues.is_empty(), "{:?}", guest.status.issues);

    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The reviewer model's answer allowing the action.
fn review_allows() -> String {
    json!({
        "riskLevel": "low",
        "userAuthorization": "explicit",
        "outcome": "allow",
        "rationale": "The user asked for the plugin's tool."
    })
    .to_string()
}
