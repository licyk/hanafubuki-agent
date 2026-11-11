//! The management commands, run as the built binary in a temporary home:
//! the configuration and its layers, Providers, MCP servers, sessions and
//! `doctor`.

mod support;

use std::time::Duration;

use hanafubuki_agent::test_support::{reasoning_answer_events, FakeServer};
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
fn config_get_and_set_check_a_plugin_option_against_its_plugin() {
    let home = Home::new();
    let get = |arguments: &[&str]| home.run(arguments).expect(0).stdout().trim().to_string();
    let key = "pluginOptions.subagents.maximumRunningAgents";
    assert_eq!(get(&["config", "get", key]), "3", "its default");
    home.run(&["config", "set", key, "6"]).expect(0);
    assert_eq!(get(&["config", "get", key]), "6");
    assert!(home
        .file("config.toml")
        .contains("maximumRunningAgents = 6"));
    assert_eq!(
        get(&["-c", &format!("{key}=40"), "config", "get", key]),
        "16"
    );

    let run = home.run(&["config", "set", key, "40"]).expect(1);
    assert!(
        run.stderr().contains("between 1 and 16"),
        "{}",
        run.stderr()
    );
    let run = home
        .run(&["config", "set", "pluginOptions.subagents.speed", "1"])
        .expect(1);
    assert!(
        run.stderr().contains("no option `speed`"),
        "{}",
        run.stderr()
    );
    let run = home
        .run(&["config", "set", "pluginOptions.web.proxy", "1"])
        .expect(1);
    assert!(run.stderr().contains("has no options"), "{}", run.stderr());
    assert!(home
        .file("config.toml")
        .contains("maximumRunningAgents = 6"));
    // The table, a plugin's and a profile's read as the file sets them.
    assert_eq!(
        get(&["config", "get", "pluginOptions.subagents"]),
        "maximumRunningAgents = 6"
    );
    assert!(get(&["config", "get", "pluginOptions"]).contains("maximumRunningAgents = 6"));
    std::fs::write(
        home.home.join("config.toml"),
        "[profiles.ci.pluginOptions.subagents]\nmaximumRunningAgents = 2\n",
    )
    .unwrap();
    assert_eq!(
        get(&[
            "config",
            "get",
            "profiles.ci.pluginOptions.subagents.maximumRunningAgents"
        ]),
        "2"
    );
    assert_eq!(get(&["-p", "ci", "config", "get", key]), "2");
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
            "--header",
            "X-Organization=acme-secret",
            "--header",
            "X-Trace=on",
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
    assert!(list.contains("headers: X-Organization, X-Trace"), "{list}");
    assert!(!list.contains("acme-secret"), "{list}");
    assert!(home.file("providers.toml").contains("acme-secret"));
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
    let forgot = home
        .run(&[
            "providers",
            "forget",
            "provider-1",
            "--model",
            "fixture-model",
        ])
        .expect(0)
        .stdout();
    assert_eq!(forgot.trim(), "forgot what was learned of 0 model(s)");
    home.run(&["providers", "forget", "provider-9"]).expect(1);
    home.run(&[
        "providers",
        "add",
        "Bad",
        "--base-url",
        "http://127.0.0.1:9/v1",
        "--header",
        "X-No-Value",
    ])
    .expect(2);
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
    home.run(&[
        "mcp",
        "add",
        "signed",
        "--disabled",
        "--oauth",
        "required",
        "--url",
        "http://127.0.0.1:9/signed",
    ])
    .expect(0);
    assert!(home.file("mcp.toml").contains("oauth = \"required\""));
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
    home.run(&[
        "mcp", "add", "local", "--oauth", "required", "--", "fake-mcp",
    ])
    .expect(2);

    // A login needs a server that takes one; the loopback itself is tested
    // with the facade.
    let run = home.run(&["mcp", "login", "remote"]).expect(1);
    assert!(
        run.stderr()
            .contains("OAuth is disabled for this MCP server"),
        "{}",
        run.stderr()
    );
    let run = home.run(&["mcp", "login", "missing"]).expect(1);
    assert!(
        run.stderr().contains("MCP server does not exist"),
        "{}",
        run.stderr()
    );
}

#[test]
fn sessions_are_listed_shown_archived_and_deleted() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(FakeServer::start(vec![(
        reasoning_answer_events("Recalling it.", "Recorded answer."),
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
    assert!(show.lines().any(|line| line == "thought for 0s"), "{show}");
    assert!(!show.contains("Recalling it."), "{show}");
    let shown = home
        .run(&["sessions", "show", &id, "--json"])
        .expect(0)
        .stdout();
    let page: Value = serde_json::from_str(&shown).unwrap();
    let reasoning = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["content"]["type"] == "reasoning")
        .expect(&shown);
    assert_eq!(reasoning["content"]["text"], "Recalling it.");

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

#[test]
fn roles_list_and_show_resolve_without_the_agent_and_doctor_reports_them() {
    let home = Home::new();
    std::fs::write(
        home.home.join("config.toml"),
        "[agents.reader]\ndescription = \"Reads.\"\nextends = \"explorer\"\n\
         tools.allow = [\"read_*\"]\npermissionModeCap = \"manual\"\n\n\
         [profiles.ci.agents.reader]\ndescription = \"The profile's.\"\n",
    )
    .unwrap();
    let agents = home.home.join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("orphan.toml"),
        "description = \"Lost.\"\nextends = \"nobody\"\ncolour = 1\n",
    )
    .unwrap();

    let run = home.run(&["roles", "list"]).expect(0);
    let list = run.stdout();
    let names: Vec<&str> = list
        .lines()
        .map(|line| line.split("  ").next().unwrap())
        .collect();
    assert_eq!(names, ["default", "explorer", "orphan", "reader"], "{list}");
    assert!(
        list.contains("explorer  built-in  Use `explorer`"),
        "{list}"
    );
    assert!(list.contains("orphan  user file "), "{list}");
    assert!(
        list.contains("unusable: `orphan` extends `nobody`"),
        "{list}"
    );
    assert!(
        run.stderr().contains("warning: role `orphan`: "),
        "{}",
        run.stderr()
    );
    assert!(run.stderr().contains("`colour`"), "{}", run.stderr());

    let show = home.run(&["roles", "show", "reader"]).expect(0).stdout();
    assert!(
        show.starts_with("reader  user [agents.reader] in "),
        "{show}"
    );
    assert!(
        show.contains("chain: reader (user [agents.reader]"),
        "{show}"
    );
    assert!(show.contains(" → explorer (built-in)"), "{show}");
    assert!(
        show.contains("description: Reads.  [from reader]"),
        "{show}"
    );
    assert!(
        show.contains("permissionModeCap: manual  [from reader]"),
        "{show}"
    );
    assert!(
        show.contains("tools.allow: every one of [") && show.contains("[from reader, explorer]"),
        "{show}"
    );
    assert!(show.contains("model: (not set)"), "{show}");
    let profile = home
        .run(&["-p", "ci", "roles", "show", "reader"])
        .expect(0)
        .stdout();
    assert!(
        profile.starts_with("reader  profile `ci` [profiles.ci.agents.reader]"),
        "{profile}"
    );
    assert!(profile.contains("tools.allow: every tool"), "{profile}");
    let unusable = home.run(&["roles", "show", "orphan"]).expect(0).stdout();
    assert!(
        unusable.contains("unusable: `orphan` extends `nobody`"),
        "{unusable}"
    );
    let missing = home.run(&["roles", "show", "nope"]).expect(1);
    assert!(
        missing.stderr().contains("no role `nope`"),
        "{}",
        missing.stderr()
    );

    let report = home.run(&["doctor"]).stdout();
    assert!(
        report.contains("ok    roles     3 usable (default, explorer, reader); 1 unusable"),
        "{report}"
    );
    assert!(
        report.contains("ok    roles     role `orphan`: "),
        "{report}"
    );
}
