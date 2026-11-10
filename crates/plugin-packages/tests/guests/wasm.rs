//! WASM guests: modules written here in the text format, assembled into
//! the package. One runs without a process, a sandbox or a grant and
//! contributes what its manifest declares; one whose call runs forever is
//! stopped by its time or its fuel, one that outgrows its memory or traps
//! fails, and each restarts alone, its contributions retiring meanwhile;
//! an error the guest reports itself is only the call's.

use std::time::{Duration, Instant};

use hanafubuki_agent_contracts::{
    AgentToolError, CancellationToken, ContextRegistry, ToolSourceRegistry,
};
use hanafubuki_agent_host::RestartPolicy;
use hanafubuki_agent_packages::{
    ContributedSection, ContributedTool, PackageContributes, WasmRuntimeLimits, WorkspaceAccess,
};
use hanafubuki_agent_plugin_packages::{GuestPackage, GuestState, GuestStatus};
use hanafubuki_agent_sandbox::Sandbox;
use serde_json::{json, Map, Value};

use crate::contribute::{context_request, entries, owners, planned, run, tool};
use crate::fixture::{eventually, Fixture, Options};

/// Answers with the request it was given (`{ key, method, args }`).
const ECHO: &str = "(call $read (i32.const 4096))
    (call $write (i32.const 4096) (local.get $len))
    (i32.const 0)";
/// Reports an error of its own.
const REFUSE: &str = "(call $write (i32.const REFUSAL) (i32.const REFUSAL_LEN)) (i32.const 1)";
const REFUSAL: &str = "the guest says no";
/// Answers with its `workspace` view.
const VIEW: &str = "(local $length i32)
    (local.set $length (call $view (i32.const WORKSPACE) (i32.const WORKSPACE_LEN)))
    (call $read (i32.const 4096))
    (call $write (i32.const 4096) (local.get $length))
    (i32.const 0)";
/// Never returns.
const SPIN: &str = "(loop $forever (br $forever)) (i32.const 0)";
/// Traps, as a Rust guest that panics does.
const CRASH: &str = "unreachable";
/// Grows its memory by 2 MiB, then echoes.
const GROW: &str = "(drop (memory.grow (i32.const 32)))
    (call $read (i32.const 4096))
    (call $write (i32.const 4096) (local.get $len))
    (i32.const 0)";

/// A guest module declaring the plugin `name` (providing `plugin`), whose
/// `apply` provides it and whose every `call` runs `call`.
fn module(name: &str, call: &str) -> String {
    let meta = json!({ "plugin": { "name": name, "provide": ["plugin"] } }).to_string();
    let strings = [meta.as_str(), "plugin", "{}", REFUSAL, "workspace"];
    let mut offsets = Vec::new();
    let mut data = String::new();
    let mut offset = 0;
    for text in strings {
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
        data += &format!("(data (i32.const {offset}) \"{escaped}\")\n");
        offsets.push((offset, text.len()));
        offset += text.len();
    }
    let place = |index: usize| {
        format!(
            "(i32.const {}) (i32.const {})",
            offsets[index].0, offsets[index].1
        )
    };
    let call = call
        .replace("(i32.const REFUSAL) (i32.const REFUSAL_LEN)", &place(3))
        .replace("(i32.const WORKSPACE) (i32.const WORKSPACE_LEN)", &place(4));
    format!(
        r#"(module
  (import "cordis" "read" (func $read (param i32)))
  (import "cordis" "write" (func $write (param i32 i32)))
  (import "cordis" "provide" (func $provide (param i32 i32 i32 i32) (result i32)))
  (import "cordis" "view" (func $view (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  {data}
  (func (export "cordis_meta") (result i32) (call $write {meta}) (i32.const 0))
  (func (export "cordis_apply") (param $len i32) (result i32)
    (drop (call $provide {key} {value}))
    (i32.const 0))
  (func (export "cordis_call") (param $len i32) (result i32)
    {call}))"#,
        meta = place(0),
        key = place(1),
        value = place(2),
    )
}

fn running(status: &GuestStatus) -> bool {
    status.state == GuestState::Running
}

/// `package` declaring the tool `name` and the context section `kind`.
fn contributing(mut package: GuestPackage, name: &str, kind: &str) -> GuestPackage {
    package.contributes = PackageContributes {
        tools: vec![ContributedTool {
            name: name.to_string(),
            description: format!("The {name} tool."),
            input_schema: Map::from_iter([("type".to_string(), json!("object"))]),
            timeout: None,
        }],
        context_sections: vec![ContributedSection {
            kind: kind.to_string(),
            renderer: None,
        }],
        ..PackageContributes::default()
    };
    package
}

pub async fn a_wasm_guest_runs_without_a_process_and_contributes_a_tool() {
    // No sandbox here: a process guest would not start, a WASM guest needs
    // none.
    let fixture = Fixture::start(Options {
        sandbox: Some(Sandbox::unavailable("no sandbox on this machine")),
        ..Options::default()
    })
    .await;
    let echo = contributing(
        fixture.wasm_package("echo", &module("echo", ECHO), WasmRuntimeLimits::default()),
        "echo",
        "echo_board",
    );
    let refusing =
        fixture.wasm_package("refusing", &module("refusing", REFUSE), Default::default());
    let liar = fixture.wasm_package("liar", &module("someone-else", ECHO), Default::default());
    // Even asking for the workspace, a module is told of none.
    let mut viewer = fixture.wasm_package("viewer", &module("viewer", VIEW), Default::default());
    viewer.permissions.workspace = WorkspaceAccess::Write;
    fixture
        .host
        .apply(vec![
            echo.clone(),
            refusing.clone(),
            liar.clone(),
            viewer.clone(),
        ])
        .await;

    let status = fixture.wait(&echo.id.key(), "start", running).await;
    assert_eq!(status.process_id, None);
    assert_eq!(fixture.processes(), 0);
    // Its metadata must be its manifest's, or it does not start.
    let failed = fixture
        .wait(&liar.id.key(), "failed start", |status| {
            status.state == GuestState::Failed
        })
        .await;
    assert!(
        failed
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("someone-else"),
        "{failed:?}"
    );
    assert_eq!(failed.restarts, 0);

    // Its tool runs as its `call`.
    eventually("its tool source", || {
        owners::<ToolSourceRegistry>(&fixture) == ["guest:echo@local"]
    })
    .await;
    let tools = planned(&fixture).await.unwrap();
    let output = run(tool(&tools, "echo__echo"), json!({ "q": "x" }))
        .await
        .unwrap();
    let answer = output.structured_content.unwrap();
    assert_eq!(answer["method"], "tool");
    assert_eq!(answer["args"]["name"], "echo");
    assert_eq!(answer["args"]["arguments"], json!({ "q": "x" }));

    fixture.wait(&viewer.id.key(), "start", running).await;
    assert_eq!(
        fixture.call(&viewer, "view", Value::Null).await.unwrap(),
        json!({ "access": "none", "roots": [] })
    );

    // An error the guest reports itself is the call's alone.
    fixture.wait(&refusing.id.key(), "start", running).await;
    for _ in 0..2 {
        let error = fixture.call(&refusing, "anything", Value::Null).await;
        assert_eq!(error, Err(REFUSAL.to_string()));
    }
    let status = fixture.host.status(&refusing.id.key()).unwrap();
    assert!(running(&status) && status.restarts == 0, "{status:?}");
}

pub async fn an_endless_wasm_call_is_stopped_and_the_guest_restarts_alone() {
    let fixture = Fixture::start(Options::default()).await;
    // Bounded by time: by default, as long as its longest call may take.
    let spinning = fixture.wasm_package("spinning", &module("spinning", SPIN), Default::default());
    // Bounded by fuel too.
    let burning = fixture.wasm_package(
        "burning",
        &module("burning", SPIN),
        WasmRuntimeLimits {
            fuel: Some(1_000_000),
            ..WasmRuntimeLimits::default()
        },
    );
    let steady = fixture.wasm_package("steady", &module("steady", ECHO), Default::default());
    fixture
        .host
        .apply(vec![spinning.clone(), burning.clone(), steady.clone()])
        .await;
    for package in [&spinning, &burning, &steady] {
        fixture.wait(&package.id.key(), "start", running).await;
    }

    let started = Instant::now();
    let error = fixture
        .host
        .call(
            &spinning.id,
            "spin",
            Value::Null,
            Duration::from_millis(300),
        )
        .await
        .unwrap_err();
    assert!(error.contains("interrupt"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(10));
    fixture
        .wait(&spinning.id.key(), "restart", |status| {
            running(status) && status.restarts == 1
        })
        .await;

    let started = Instant::now();
    let error = fixture
        .call(&burning, "burn", Value::Null)
        .await
        .unwrap_err();
    assert!(error.contains("fuel"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(10));
    fixture
        .wait(&burning.id.key(), "restart", |status| {
            running(status) && status.restarts == 1
        })
        .await;

    // The others' failures were theirs alone.
    let status = fixture.host.status(&steady.id.key()).unwrap();
    assert!(running(&status) && status.restarts == 0, "{status:?}");
    assert_eq!(
        fixture.call(&steady, "hello", json!([1])).await.unwrap()["method"],
        "hello"
    );
}

pub async fn a_wasm_guest_that_outgrows_its_memory_traps_and_restarts() {
    let fixture = Fixture::start(Options::default()).await;
    let capped = fixture.wasm_package(
        "capped",
        &module("capped", GROW),
        WasmRuntimeLimits {
            memory_bytes: 1024 * 1024,
            ..WasmRuntimeLimits::default()
        },
    );
    // The default 256 MiB lets it grow.
    let roomy = fixture.wasm_package("roomy", &module("roomy", GROW), Default::default());
    fixture
        .host
        .apply(vec![capped.clone(), roomy.clone()])
        .await;
    fixture.wait(&capped.id.key(), "start", running).await;
    fixture.wait(&roomy.id.key(), "start", running).await;

    let error = fixture
        .call(&capped, "grow", Value::Null)
        .await
        .unwrap_err();
    assert!(error.contains("growing memory"), "{error}");
    fixture
        .wait(&capped.id.key(), "restart", |status| {
            running(status) && status.restarts == 1
        })
        .await;
    assert_eq!(
        fixture.call(&roomy, "grow", Value::Null).await.unwrap()["method"],
        "grow"
    );
    let status = fixture.host.status(&roomy.id.key()).unwrap();
    assert!(running(&status) && status.restarts == 0, "{status:?}");
}

pub async fn a_trapping_wasm_guests_contributions_retire_and_return() {
    let fixture = Fixture::start(Options {
        policy: RestartPolicy {
            delays: vec![Duration::from_secs(1); 3],
            window: Duration::from_secs(600),
        },
        ..Options::default()
    })
    .await;
    let package = contributing(
        fixture.wasm_package("crashing", &module("crashing", CRASH), Default::default()),
        "crash",
        "crash_board",
    );
    let id = package.id.key();
    fixture.host.apply(vec![package.clone()]).await;
    fixture.wait(&id, "start", running).await;
    eventually("its contributions", || {
        owners::<ToolSourceRegistry>(&fixture).len() == 1
            && owners::<ContextRegistry>(&fixture).len() == 1
    })
    .await;

    // Its tool traps: the call's error, and the guest fails.
    let tools = planned(&fixture).await.unwrap();
    let crashed = run(tool(&tools, "crashing__crash"), json!({})).await;
    assert!(
        matches!(crashed, Err(AgentToolError::Failed(_))),
        "{crashed:?}"
    );
    let restarting = fixture
        .wait(&id, "the restart", |status| {
            status.state == GuestState::Restarting
        })
        .await;
    assert!(
        restarting
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("unreachable"),
        "{restarting:?}"
    );
    eventually("its contributions retired", || {
        owners::<ToolSourceRegistry>(&fixture).is_empty()
            && owners::<ContextRegistry>(&fixture).is_empty()
    })
    .await;

    // They come back with its restart; its context section traps too, and
    // is left out.
    fixture
        .wait(&id, "its restart", |status| {
            running(status) && status.restarts == 1
        })
        .await;
    eventually("its contributions back", || {
        owners::<ToolSourceRegistry>(&fixture).len() == 1
            && owners::<ContextRegistry>(&fixture).len() == 1
    })
    .await;
    let contributor = entries::<ContextRegistry>(&fixture).remove(0);
    let left_out = contributor
        .contribute(&context_request(), CancellationToken::default())
        .await;
    assert!(left_out.is_err());
    fixture
        .wait(&id, "the second restart", |status| {
            running(status) && status.restarts == 2
        })
        .await;
}
