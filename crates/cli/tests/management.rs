//! The management commands, run as the built binary in a temporary home:
//! the configuration and its layers, Providers, MCP servers, sessions and
//! `doctor`.

mod support;

use std::time::Duration;

use hanafubuki_agent::test_support::{assistant_events, FakeServer};
use serde_json::Value;
use support::Home;

#[test]
fn config_get_reads_every_layer_and_set_writes_the_users_file() {
    let home = Home::new();
    std::fs::write(
        home.home.join("config.toml"),
        "# kept\ndisabledPlugins = [\"titles\"]\n\n[profiles.ci]\nlogLevel = \"debug\"\n",
    )
    .unwrap();
    let project_file = home.project.join(".hanafubuki-agent/config.toml");
    std::fs::create_dir_all(project_file.parent().unwrap()).unwrap();
    std::fs::write(&project_file, "webFetchUseJinaReader = true\n").unwrap();

    let get = |arguments: &[&str]| home.run(arguments).expect(0).stdout().trim().to_string();
    assert_eq!(get(&["config", "get", "logLevel"]), "info");
    assert_eq!(get(&["-p", "ci", "config", "get", "logLevel"]), "debug");
    assert_eq!(
        get(&[
            "-p",
            "ci",
            "--log-level",
            "warn",
            "config",
            "get",
            "logLevel"
        ]),
        "warn",
        "a flag over the profile"
    );
    assert_eq!(
        get(&[
            "-c",
            "memory.enabled=true",
            "config",
            "get",
            "memory.enabled"
        ]),
        "true"
    );
    assert_eq!(get(&["config", "get", "disabledPlugins"]), "[\"titles\"]");
    // The project is not trusted yet.
    assert_eq!(get(&["config", "get", "webFetchUseJinaReader"]), "false");

    home.run(&["config", "set", "network.proxy", "http://127.0.0.1:7890"])
        .expect(0);
    let trust = format!("projects.'{}'.trustLevel", home.project.display());
    home.run(&["config", "set", &trust, "trusted"]).expect(0);
    assert_eq!(get(&["config", "get", &trust]), "trusted");
    assert_eq!(get(&["config", "get", "webFetchUseJinaReader"]), "true");
    assert_eq!(
        get(&["config", "get", "network.proxy"]),
        "http://127.0.0.1:7890"
    );
    // A proxy's credentials are written, never printed.
    home.run(&[
        "config",
        "set",
        "network.proxy",
        "http://me:hunter2@127.0.0.1:7890",
    ])
    .expect(0);
    assert_eq!(
        get(&["config", "get", "network.proxy"]),
        "http://***@127.0.0.1:7890"
    );
    assert!(!get(&["config", "get"]).contains("hunter2"));
    let file = home.file("config.toml");
    assert!(file.contains("hunter2"), "{file}");
    assert!(file.starts_with("# kept\n"), "{file}");
    assert!(file.contains("[profiles.ci]"), "{file}");

    let run = home.run(&["config", "set", "logLevel", "loud"]).expect(1);
    assert!(
        run.stderr().contains("cannot take this value"),
        "{}",
        run.stderr()
    );
    home.run(&["config", "set", "noSuchKey", "1"]).expect(1);
    let run = home.run(&["-p", "missing", "config", "get"]).expect(1);
    assert!(
        run.stderr().contains("no profile `missing`"),
        "{}",
        run.stderr()
    );

    let paths = get(&["config", "path"]);
    assert!(paths.contains(&home.home.join("providers.toml").display().to_string()));
}

#[test]
fn providers_are_added_made_default_listed_without_tokens_and_removed() {
    let home = Home::new();
    let run = home
        .command()
        .args([
            "providers",
            "add",
            "Local",
            "--base-url",
            "http://127.0.0.1:9/v1",
            "--api-key-env",
            "TEST_PROVIDER_TOKEN",
        ])
        .env("TEST_PROVIDER_TOKEN", "sk-secret-value")
        .output()
        .unwrap();
    let run = support::Run(run).expect(0);
    assert_eq!(run.stdout().trim(), "added provider-1");
    home.run(&[
        "providers",
        "add",
        "Other",
        "--base-url",
        "http://127.0.0.1:9/v2",
        "--protocol",
        "chat-completions",
    ])
    .expect(0);
    home.run(&[
        "providers",
        "default",
        "provider-2",
        "--model",
        "fixture-model",
    ])
    .expect(0);

    let list = home.run(&["providers", "list"]).expect(0).stdout();
    assert!(list.contains("model: fixture-model"), "{list}");
    assert!(list.contains("provider-1 Local  Responses"), "{list}");
    assert!(list.contains("token: set"), "{list}");
    assert!(
        list.contains("provider-2 Other (default)  ChatCompletions"),
        "{list}"
    );
    assert!(!list.contains("sk-secret-value"), "{list}");
    assert!(home.file("providers.toml").contains("sk-secret-value"));

    home.run(&["providers", "remove", "provider-2"]).expect(0);
    let list = home.run(&["providers", "list"]).expect(0).stdout();
    assert!(!list.contains("provider-2"), "{list}");
    assert!(
        list.contains("provider-1 Local (default)"),
        "the default moves: {list}"
    );
    home.run(&["providers", "remove", "provider-9"]).expect(1);
}

#[test]
fn mcp_servers_are_added_listed_without_secrets_and_removed() {
    let home = Home::new();
    home.run(&[
        "mcp",
        "add",
        "local",
        "--disabled",
        "--env",
        "API_TOKEN=very-secret",
        "--",
        "fake-mcp",
        "--stdio",
    ])
    .expect(0);
    home.run(&[
        "mcp",
        "add",
        "remote",
        "--disabled",
        "--url",
        "http://127.0.0.1:9/mcp",
    ])
    .expect(0);
    let list = home.run(&["mcp", "list"]).expect(0).stdout();
    assert!(
        list.contains("local (disabled)  stdio fake-mcp --stdio  env: API_TOKEN"),
        "{list}"
    );
    assert!(
        list.contains("remote (disabled)  http http://127.0.0.1:9/mcp"),
        "{list}"
    );
    assert!(!list.contains("very-secret"), "{list}");

    home.run(&["mcp", "remove", "local"]).expect(0);
    let list = home.run(&["mcp", "list"]).expect(0).stdout();
    assert!(!list.contains("local"), "{list}");
    home.run(&["mcp", "add", "bad id", "--url", "http://127.0.0.1:9/mcp"])
        .expect(2);
    home.run(&["mcp", "add", "nothing"]).expect(2);
}

#[test]
fn sessions_are_listed_shown_archived_and_deleted() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(FakeServer::start(vec![(
        assistant_events("Recorded answer."),
        Duration::ZERO,
    )]));
    let home = Home::new().with_provider(&server.base_url);
    home.run(&["exec", "Remember this question"]).expect(0);

    let list = home.run(&["sessions", "list"]).expect(0).stdout();
    let row = list.lines().next().unwrap_or_default().to_string();
    let id = row.split_whitespace().next().unwrap().to_string();
    assert!(row.contains("fixture-model"), "{list}");
    let json = home.run(&["sessions", "list", "--json"]).expect(0).stdout();
    let summary: Value = serde_json::from_str(json.lines().next().unwrap()).unwrap();
    assert_eq!(summary["sessionId"].to_string(), id);

    let show = home.run(&["sessions", "show", &id]).expect(0).stdout();
    assert!(show.contains("user: Remember this question"), "{show}");
    assert!(show.contains("assistant: Recorded answer."), "{show}");

    home.run(&["sessions", "archive", &id]).expect(0);
    assert!(home
        .run(&["sessions", "list"])
        .expect(0)
        .stdout()
        .is_empty());
    let archived = home
        .run(&["sessions", "list", "--archived"])
        .expect(0)
        .stdout();
    assert!(archived.starts_with(&id), "{archived}");
    home.run(&["sessions", "delete", &id]).expect(0);
    assert!(home
        .run(&["sessions", "list", "--archived"])
        .expect(0)
        .stdout()
        .is_empty());
    home.run(&["sessions", "show", "nope"]).expect(2);
}

#[test]
fn doctor_offline_checks_each_part_and_fails_on_any_failure() {
    let home = Home::new();
    // Without a Provider the check fails.
    let run = home.run(&["doctor"]).expect(1);
    let report = run.stdout();
    assert!(report.contains("FAIL  providers 0 configured"), "{report}");
    assert!(report.contains("ok    config    "), "{report}");
    assert!(
        report.contains("ok    mcp       0 configured, 0 enabled"),
        "{report}"
    );
    assert!(report.contains("ok    data      "), "{report}");
    assert!(report.contains("skip  online    "), "{report}");

    let home = Home::new().with_provider("http://127.0.0.1:9/v1");
    let run = home.run(&["doctor"]);
    let report = run.stdout();
    assert!(
        report.contains("ok    providers 1 configured; the default provider-1"),
        "{report}"
    );
    // The keyring is reachable or not depending on the machine (a headless
    // Linux has no Secret Service); the exit code follows the report.
    let failed = report.contains("FAIL");
    assert_eq!(run.code(), i32::from(failed), "{report}");
    assert!(report.contains("keyring"), "{report}");
}
