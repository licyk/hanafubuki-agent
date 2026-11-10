//! `exec` end to end: the built binary runs one turn against the scripted
//! model server, in a temporary home and working directory.

mod support;

use std::io::Write;
use std::process::Stdio;
use std::time::Duration;

use hanafubuki_agent::test_support::{
    assistant_events, reasoning_answer_events, tool_events, FakeServer,
};
use serde_json::Value;
use support::Home;

/// The scripted model, answering each request with the next response.
struct Model {
    _runtime: tokio::runtime::Runtime,
    server: FakeServer,
}

impl Model {
    fn start(responses: Vec<Vec<u8>>) -> Self {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let server = runtime.block_on(FakeServer::start(
            responses
                .into_iter()
                .map(|body| (body, Duration::ZERO))
                .collect(),
        ));
        Self {
            _runtime: runtime,
            server,
        }
    }

    fn request_count(&self) -> usize {
        self.server.requests().len()
    }
}

fn command_call(command: &str) -> Vec<u8> {
    let arguments = serde_json::json!({
        "cmd": command,
        "shell": null,
        "login": null,
        "tty": null,
        "yield_time_ms": 30_000,
        "max_output_bytes": 32_768,
        "workdir": null
    });
    tool_events("exec_command", &arguments.to_string())
}

#[test]
fn a_turn_prints_its_answer_and_writes_the_last_message() {
    let model = Model::start(vec![assistant_events("The answer is 42.")]);
    let home = Home::new().with_provider(&model.server.base_url);
    let last = home.home.join("last.txt");
    let run = home
        .run(&["exec", "What is the answer?", "-o", last.to_str().unwrap()])
        .expect(0);
    assert!(
        run.stdout().contains("The answer is 42."),
        "{}",
        run.stdout()
    );
    assert!(run.stderr().contains("session "), "{}", run.stderr());
    assert_eq!(std::fs::read_to_string(last).unwrap(), "The answer is 42.");
    assert_eq!(model.request_count(), 1);
}

#[test]
fn the_thinking_time_goes_to_stderr_without_its_text() {
    let model = Model::start(vec![reasoning_answer_events(
        "Weighing it.",
        "The answer is 42.",
    )]);
    let home = Home::new().with_provider(&model.server.base_url);
    let run = home.run(&["exec", "What is the answer?"]).expect(0);
    assert!(
        run.stderr().contains("· thought for 0s"),
        "{}",
        run.stderr()
    );
    assert!(!run.stderr().contains("Weighing it."), "{}", run.stderr());
    assert!(!run.stdout().contains("Weighing it."), "{}", run.stdout());
    assert!(
        run.stdout().contains("The answer is 42."),
        "{}",
        run.stdout()
    );

    let model = Model::start(vec![reasoning_answer_events(
        "Weighing it.",
        "The answer is 42.",
    )]);
    let home = Home::new().with_provider(&model.server.base_url);
    let run = home
        .run(&["exec", "--json", "What is the answer?"])
        .expect(0);
    let kinds = run
        .stdout()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .map(|event| event["kind"].clone())
        .collect::<Vec<_>>();
    let position = |kind: &str| {
        kinds
            .iter()
            .position(|event| event["type"] == kind)
            .unwrap_or_else(|| panic!("no {kind}: {kinds:?}"))
    };
    assert!(position("reasoning_started") < position("reasoning_delta"));
    assert!(position("reasoning_delta") < position("reasoning_completed"));
    assert_eq!(kinds[position("reasoning_delta")]["delta"], "Weighing it.");
}

#[test]
fn json_prints_one_event_per_line_ending_with_the_completion() {
    let model = Model::start(vec![assistant_events("Lines.")]);
    let home = Home::new().with_provider(&model.server.base_url);
    let mut child = home
        .command()
        .args(["exec", "--json", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"From standard input")
        .unwrap();
    let run = support::Run(child.wait_with_output().unwrap()).expect(0);
    let events = run
        .stdout()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let kinds = events
        .iter()
        .map(|event| event["kind"]["type"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(kinds.first().map(String::as_str), Some("turn_started"));
    assert_eq!(kinds.last().map(String::as_str), Some("turn_completed"));
    assert!(kinds.iter().any(|kind| kind == "assistant_text_delta"));
    for event in &events {
        assert!(event["sessionId"].is_u64(), "{event}");
        assert!(event["sequence"].is_u64(), "{event}");
    }
    let request = String::from_utf8_lossy(&model.server.requests()[0].body).into_owned();
    assert!(request.contains("From standard input"));
}

#[test]
fn an_approval_is_denied_under_approve_deny_and_the_turn_goes_on() {
    let model = Model::start(vec![
        command_call("echo should-not-run"),
        assistant_events("It was denied."),
    ]);
    let home = Home::new().with_provider(&model.server.base_url);
    let run = home
        .run(&["exec", "--approve", "deny", "Run the command."])
        .expect(0);
    let stdout = run.stdout();
    assert!(stdout.contains("→ exec_command"), "{stdout}");
    assert!(
        stdout.contains("approval denied: exec_command: "),
        "{stdout}"
    );
    assert!(stdout.contains("It was denied."), "{stdout}");
    assert_eq!(model.request_count(), 2);

    // In JSON, the kernel's resolution carries the denial.
    let model = Model::start(vec![
        command_call("echo should-not-run"),
        assistant_events("Denied again."),
    ]);
    let home = Home::new().with_provider(&model.server.base_url);
    let run = home.run(&["exec", "--json", "Run the command."]).expect(0);
    let resolution = run
        .stdout()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|event| event["kind"]["type"] == "approval_resolved")
        .expect("an approval_resolved event");
    assert_eq!(resolution["kind"]["resolution"]["decision"], "deny");
    assert!(run.stderr().contains("approval denied: exec_command"));
}

#[test]
fn an_allowed_tool_runs() {
    #[cfg(not(windows))]
    let command = "printf allowed-command";
    #[cfg(windows)]
    let command = "Write-Output allowed-command";
    let model = Model::start(vec![command_call(command), assistant_events("It ran.")]);
    let home = Home::new().with_provider(&model.server.base_url);
    let run = home
        .run(&["exec", "--approve", "allow:exec_command", "Run it."])
        .expect(0);
    let stdout = run.stdout();
    assert!(
        stdout.contains("approval allowed: exec_command"),
        "{stdout}"
    );
    assert!(stdout.contains("exec_command done"), "{stdout}");
    let output = String::from_utf8_lossy(&model.server.requests()[1].body).into_owned();
    assert!(output.contains("allowed-command"), "{output}");
}

#[test]
fn the_exit_code_says_how_the_command_ended() {
    let home = Home::new();
    // The terminal interface needs a terminal, and so does `resume`; a
    // session id that is not one, or one beside --last, is a usage error.
    let run = home.run(&[]).expect(2);
    assert!(run.stderr().contains("hanafubuki-agent exec PROMPT"));
    home.run(&["resume", "--last"]).expect(2);
    let run = home.run(&["resume", "not-an-id"]).expect(2);
    assert!(
        run.stderr().contains("is not a session id"),
        "{}",
        run.stderr()
    );
    let run = home.run(&["resume", "--last", "7", "hi"]).expect(2);
    assert!(
        run.stderr().contains("takes no session id"),
        "{}",
        run.stderr()
    );
    // An empty prompt, an unknown policy.
    home.run(&["exec", "-"]).expect(2);
    home.run(&["exec", "--approve", "sometimes", "hi"])
        .expect(2);
    // No Provider: the session cannot be made.
    let run = home.run(&["exec", "hi"]).expect(1);
    assert!(
        run.stderr()
            .contains("No default Agent Provider is configured"),
        "{}",
        run.stderr()
    );
    // A model that fails the turn.
    let model = Model::start(vec![
        b"data: {\"type\":\"response.failed\",\"response\":{\"id\":\"failed\",\"error\":{\"code\":\"invalid_request\",\"message\":\"no\"}}}\n\n".to_vec(),
    ]);
    let home = Home::new().with_provider(&model.server.base_url);
    let run = home.run(&["exec", "hi"]).expect(1);
    assert!(run.stderr().contains("the turn failed"), "{}", run.stderr());
}

/// Ctrl-C cancels the turn under way, and `exec` ends once it did.
#[cfg(unix)]
#[test]
fn an_interrupt_cancels_the_turn_and_exits_with_130() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(FakeServer::start(vec![(
        assistant_events("Too late."),
        Duration::from_secs(60),
    )]));
    let home = Home::new().with_provider(&server.base_url);
    let child = home
        .command()
        .args(["exec", "Take your time."])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while server.requests().is_empty() {
        assert!(std::time::Instant::now() < deadline, "no model request");
        std::thread::sleep(Duration::from_millis(20));
    }
    // The handler is installed once the turn was accepted, before its
    // request; give it a moment to be in place.
    std::thread::sleep(Duration::from_millis(500));
    let interrupted = std::process::Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(interrupted.success());
    let run = support::Run(child.wait_with_output().unwrap()).expect(130);
    assert!(
        run.stderr()
            .contains("the turn was cancelled (user_interrupted)"),
        "{}",
        run.stderr()
    );
    assert!(run.stdout().contains("interrupted; cancelling the turn"));
}

/// The binary is its own sandbox helper: by default on Linux a command
/// writes in the workspace (and `/tmp`), nowhere else. Skipped where the
/// kernel has no Landlock, which `doctor` reports.
#[cfg(target_os = "linux")]
#[test]
fn a_command_runs_in_the_workspace_write_sandbox_by_default() {
    let home = Home::new();
    if !home
        .run(&["doctor"])
        .stdout()
        .contains("ok    sandbox   workspace-write")
    {
        eprintln!("skipped: the Linux sandbox is unavailable here");
        return;
    }
    let outside = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let escaped = outside.path().join("escaped.txt");
    let command = format!(
        "echo in > inside.txt; echo out > '{}' || echo refused-outside",
        escaped.display()
    );
    let model = Model::start(vec![command_call(&command), assistant_events("It ran.")]);
    // The fixture Provider's configuration is full access; unset the mode
    // again, back to the default.
    let home = home.with_provider(&model.server.base_url);
    let config = home
        .file("config.toml")
        .replace("sandboxMode = \"full-access\"\n", "");
    std::fs::write(home.home.join("config.toml"), config).unwrap();
    home.run(&["exec", "--approve", "allow:exec_command", "Run it."])
        .expect(0);
    assert_eq!(
        std::fs::read_to_string(home.cwd.join("inside.txt")).unwrap(),
        "in\n"
    );
    assert!(!escaped.exists());
    let output = String::from_utf8_lossy(&model.server.requests()[1].body).into_owned();
    assert!(output.contains("refused-outside"), "{output}");
}

/// The sandbox keeps a command off the network by default; the model asks
/// to run it outside the sandbox (`require_escalated`), and once approved
/// it connects. The connection is to a listener of this test's, never the
/// scripted model.
#[cfg(target_os = "linux")]
#[test]
fn an_escalated_command_reaches_the_network_the_sandbox_refused() {
    let home = Home::new();
    if !home
        .run(&["doctor"])
        .stdout()
        .contains("ok    sandbox   workspace-write")
    {
        eprintln!("skipped: the Linux sandbox is unavailable here");
        return;
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let command = format!(
        "bash -c 'exec 3<>/dev/tcp/127.0.0.1/{}' 2>/dev/null && echo connected || echo refused-network",
        listener.local_addr().unwrap().port()
    );
    let call = |escalated: bool, call_id: &str| {
        let mut arguments = serde_json::json!({
            "cmd": command,
            "shell": null,
            "login": null,
            "tty": null,
            "yield_time_ms": 30_000,
            "max_output_bytes": 32_768,
            "workdir": null,
            "sandbox_permissions": null,
            "justification": null
        });
        if escalated {
            arguments["sandbox_permissions"] = "require_escalated".into();
            arguments["justification"] = "Do you want to let the command use the network?".into();
        }
        String::from_utf8(tool_events("exec_command", &arguments.to_string()))
            .unwrap()
            .replace("call-tool", call_id)
            .into_bytes()
    };
    let model = Model::start(vec![
        call(false, "call-sandboxed"),
        call(true, "call-escalated"),
        assistant_events("It connected."),
    ]);
    let home = home.with_provider(&model.server.base_url);
    let config = home
        .file("config.toml")
        .replace("sandboxMode = \"full-access\"\n", "");
    std::fs::write(home.home.join("config.toml"), config).unwrap();
    let run = home
        .run(&["exec", "--approve", "allow:exec_command", "Connect."])
        .expect(0);
    assert!(
        run.stdout().contains("command outside the sandbox in "),
        "{}",
        run.stdout()
    );
    let output = |request: usize| {
        let body: Value = serde_json::from_slice(&model.server.requests()[request].body).unwrap();
        body["input"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|item| item["type"] == "function_call_output")
            .unwrap()["output"]
            .to_string()
    };
    assert!(output(1).contains("refused-network"), "{}", output(1));
    assert!(output(2).contains("connected"), "{}", output(2));
    assert!(!output(2).contains("refused-network"), "{}", output(2));
    // The model was told the sandbox keeps the network off.
    let first = String::from_utf8_lossy(&model.server.requests()[0].body).into_owned();
    assert!(
        first.contains(r#"kind=\"sandbox\""#) && first.contains(r#"\"networkAccess\":false"#),
        "{first}"
    );
}

/// On Windows the sandbox is opt-in: with `[windows] sandbox =
/// "unelevated"` a workspace-write command, run by PowerShell through the
/// binary as its helper, writes in the workspace, nowhere else.
#[cfg(windows)]
#[test]
fn a_command_runs_in_the_unelevated_windows_sandbox_once_chosen() {
    let outside = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let escaped = outside.path().join("escaped.txt");
    let command = format!(
        "Set-Content -Path inside.txt -Value in; \
         try {{ Set-Content -Path '{}' -Value out -ErrorAction Stop }} catch {{ 'refused-outside' }}",
        escaped.display()
    );
    let model = Model::start(vec![command_call(&command), assistant_events("It ran.")]);
    let home = Home::new().with_provider(&model.server.base_url);
    let config = home.file("config.toml").replace(
        "sandboxMode = \"full-access\"\n",
        "sandboxMode = \"workspace-write\"\n",
    ) + "\n[windows]\nsandbox = \"unelevated\"\n";
    std::fs::write(home.home.join("config.toml"), config).unwrap();
    assert!(home
        .run(&["doctor"])
        .stdout()
        .contains("ok    sandbox   workspace-write: a restricted token"));
    home.run(&["exec", "--approve", "allow:exec_command", "Run it."])
        .expect(0);
    assert_eq!(
        std::fs::read_to_string(home.cwd.join("inside.txt"))
            .unwrap()
            .trim(),
        "in"
    );
    assert!(!escaped.exists());
    let output = String::from_utf8_lossy(&model.server.requests()[1].body).into_owned();
    assert!(output.contains("refused-outside"), "{output}");
}
