//! Discovery from `hooks.json` and `[hooks]`, read tolerantly; trust by the
//! hash of a hook's definition; enablement; and running the trusted hooks
//! of an event through the executor.

use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use hanafubuki_agent_foundation::cancellation::CancellationToken;
#[cfg(unix)]
use hanafubuki_agent_hooks::{HookEvent, HookScope};
use hanafubuki_agent_hooks::{
    HookEventName, HookHandler, HookSource, HookSources, HookTrust, HooksEngine,
};
use hanafubuki_agent_process::executor::ExecutorManager;
use serde_json::json;
use tempfile::TempDir;

struct Fixture {
    _root: TempDir,
    user: PathBuf,
    project: PathBuf,
    engine: HooksEngine,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let user = root.path().join("config");
    let project = root.path().join("project/.hanafubuki-agent");
    fs::create_dir_all(&user).unwrap();
    fs::create_dir_all(&project).unwrap();
    let engine = HooksEngine::new(
        HookSources {
            user: user.clone(),
            project: Some(project.clone()),
            plugins: Vec::new(),
        },
        root.path().join("data/hooks-state.json"),
        ExecutorManager::default(),
    );
    Fixture {
        _root: root,
        user,
        project,
        engine,
    }
}

fn write_hooks_json(directory: &Path, hooks: serde_json::Value) {
    fs::write(
        directory.join("hooks.json"),
        serde_json::to_vec(&json!({ "hooks": hooks })).unwrap(),
    )
    .unwrap();
}

#[test]
fn hooks_are_discovered_from_both_files_of_both_directories() {
    let fixture = fixture();
    write_hooks_json(
        &fixture.user,
        json!({ "PreToolUse": [{ "matcher": "Bash", "hooks": [
            { "type": "command", "command": "check.sh", "timeout": 5, "statusMessage": "checking" }
        ]}]}),
    );
    fs::write(
        fixture.project.join("config.toml"),
        "webSearchProvider = \"duckduckgo\"\n\n[[hooks.Stop]]\n[[hooks.Stop.hooks]]\ntype = \"command\"\ncommand = \"stop.sh\"\n",
    )
    .unwrap();

    let catalog = fixture.engine.discover();

    assert_eq!(catalog.diagnostics, Vec::<String>::new());
    let hooks = catalog
        .hooks
        .iter()
        .map(|hook| {
            (
                hook.event,
                hook.source.clone(),
                hook.matcher.clone(),
                &hook.handler,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        hooks,
        [
            (
                HookEventName::PreToolUse,
                HookSource::User,
                Some("Bash".to_string()),
                &HookHandler::Command {
                    command: "check.sh".to_string(),
                    timeout_sec: 5,
                    status_message: Some("checking".to_string()),
                }
            ),
            (
                HookEventName::Stop,
                HookSource::Project,
                None,
                &HookHandler::Command {
                    command: "stop.sh".to_string(),
                    timeout_sec: 600,
                    status_message: None,
                }
            ),
        ]
    );
    assert!(catalog.hooks[0]
        .key
        .ends_with("hooks.json:pre_tool_use:0:0"));
    assert!(catalog.hooks[1].key.ends_with("config.toml:stop:0:0"));
    assert!(catalog
        .hooks
        .iter()
        .all(|hook| hook.trust == HookTrust::Untrusted && hook.enabled && !hook.runnable()));
}

#[test]
fn one_bad_entry_is_skipped_with_a_diagnostic_and_the_rest_load() {
    let fixture = fixture();
    write_hooks_json(
        &fixture.user,
        json!({
            "PreToolUse": [
                "not a group",
                { "matcher": "(", "hooks": [{ "type": "command", "command": "a.sh" }] },
                { "hooks": [
                    { "type": "command" },
                    { "type": "mcp_tool", "server": "s", "tool": "t" },
                    { "type": "command", "command": "kept.sh", "unknownField": true },
                    { "type": "command", "command": "later.sh", "async": true },
                ]},
            ],
            "PermissionRequest": [],
            "NoSuchEvent": [],
        }),
    );

    let catalog = fixture.engine.discover();

    assert_eq!(catalog.hooks.len(), 1);
    let kept = &catalog.hooks[0];
    assert!(kept.key.ends_with(":pre_tool_use:2:2"), "{}", kept.key);
    assert!(matches!(&kept.handler, HookHandler::Command { command, .. } if command == "kept.sh"));
    let diagnostics = catalog.diagnostics.join("\n");
    for expected in [
        "PermissionRequest hooks never run",
        "unknown hook event `NoSuchEvent`",
        "group 0 is not a table",
        "group 1 has an invalid matcher",
        "hook 0 cannot be read",
        "`mcp_tool` hooks are not supported",
        "async command hooks are not supported",
    ] {
        assert!(diagnostics.contains(expected), "{expected}: {diagnostics}");
    }
}

#[test]
fn a_file_that_does_not_read_is_a_diagnostic() {
    let fixture = fixture();
    fs::write(fixture.user.join("hooks.json"), "{ not json").unwrap();

    let catalog = fixture.engine.discover();

    assert!(catalog.hooks.is_empty());
    assert_eq!(catalog.diagnostics.len(), 1);
}

#[test]
fn trust_is_by_the_hash_of_the_definition() {
    let fixture = fixture();
    let hook = |command: &str| json!({ "PreToolUse": [{ "hooks": [{ "type": "command", "command": command }] }]});
    write_hooks_json(&fixture.user, hook("check.sh"));
    let key = fixture.engine.discover().hooks[0].key.clone();

    fixture.engine.trust(std::slice::from_ref(&key)).unwrap();
    let trusted = fixture.engine.discover().hooks[0].clone();
    assert_eq!(trusted.trust, HookTrust::Trusted);
    assert!(trusted.runnable());

    // The same definition in the other file of the directory keeps its
    // hash; a changed one loses the trust.
    write_hooks_json(&fixture.user, hook("check.sh --strict"));
    let modified = fixture.engine.discover().hooks[0].clone();
    assert_eq!(modified.key, key);
    assert_eq!(modified.trust, HookTrust::Modified);
    assert_ne!(modified.current_hash, trusted.current_hash);
    assert!(!modified.runnable());

    write_hooks_json(&fixture.user, hook("check.sh"));
    fixture.engine.set_enabled(&key, false).unwrap();
    let disabled = fixture.engine.discover().hooks[0].clone();
    assert_eq!(disabled.trust, HookTrust::Trusted);
    assert!(!disabled.enabled && !disabled.runnable());
    fixture.engine.set_enabled(&key, true).unwrap();
    assert!(fixture.engine.discover().hooks[0].runnable());

    assert!(fixture.engine.trust(&["no-such-hook".to_string()]).is_err());
    assert!(fixture.engine.set_enabled("no-such-hook", false).is_err());
}

#[test]
fn the_same_definition_hashes_alike_in_either_file() {
    let fixture = fixture();
    write_hooks_json(
        &fixture.user,
        json!({ "Stop": [{ "hooks": [{ "type": "command", "command": "stop.sh" }] }]}),
    );
    let from_json = fixture.engine.discover().hooks[0].current_hash.clone();
    fs::remove_file(fixture.user.join("hooks.json")).unwrap();
    fs::write(
        fixture.user.join("config.toml"),
        "[[hooks.Stop]]\n[[hooks.Stop.hooks]]\ncommand = \"stop.sh\"\ntype = \"command\"\n",
    )
    .unwrap();

    assert_eq!(fixture.engine.discover().hooks[0].current_hash, from_json);
}

#[test]
fn a_state_file_that_does_not_read_leaves_every_hook_untrusted() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("hooks-state.json");
    fs::write(&state, "[not a state").unwrap();
    let engine = HooksEngine::new(
        HookSources {
            user: root.path().to_path_buf(),
            project: None,
            plugins: Vec::new(),
        },
        state.clone(),
        ExecutorManager::default(),
    );
    write_hooks_json(
        root.path(),
        json!({ "Stop": [{ "hooks": [{ "type": "command", "command": "stop.sh" }] }]}),
    );

    assert_eq!(engine.discover().hooks[0].trust, HookTrust::Untrusted);
    assert!(!state.exists(), "the unreadable state is quarantined");
}

#[cfg(unix)]
fn scope(directory: &Path) -> HookScope {
    HookScope {
        session_id: "1".to_string(),
        turn_id: Some("2".to_string()),
        cwd: directory.to_path_buf(),
    }
}

#[cfg(unix)]
#[test]
fn a_trusted_pre_tool_use_hook_denies_and_one_past_its_timeout_fails() {
    let fixture = fixture();
    write_hooks_json(
        &fixture.user,
        json!({ "PreToolUse": [
            { "matcher": "Bash", "hooks": [{
                "type": "command",
                "command": "grep -q '\"tool_name\":\"exec_command\"' && echo 'no shell' >&2 && exit 2",
            }]},
            { "matcher": "read_file", "hooks": [{
                "type": "command", "command": "sleep 5", "timeout": 1,
            }]},
        ]}),
    );
    let keys = fixture
        .engine
        .discover()
        .hooks
        .into_iter()
        .map(|hook| hook.key)
        .collect::<Vec<_>>();
    fixture.engine.trust(&keys).unwrap();
    let catalog = fixture.engine.discover();
    let call = |tool: &str| HookEvent::PreToolUse {
        tool_name: tool.to_string(),
        tool_input: json!({ "cmd": "ls" }),
        tool_use_id: "call-1".to_string(),
    };

    let denied = fixture.engine.run(
        &catalog,
        &call("exec_command"),
        &scope(&fixture.user),
        &CancellationToken::default(),
    );
    assert_eq!(denied.deny.as_deref(), Some("no shell"));

    let started = std::time::Instant::now();
    let timed_out = fixture.engine.run(
        &catalog,
        &call("read_file"),
        &scope(&fixture.user),
        &CancellationToken::default(),
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(4));
    assert_eq!(timed_out.deny, None);
    assert_eq!(timed_out.failures.len(), 1);
    assert!(timed_out.failures[0].1.contains("timed out"));

    let unmatched = fixture.engine.run(
        &catalog,
        &call("list_directory"),
        &scope(&fixture.user),
        &CancellationToken::default(),
    );
    assert_eq!(unmatched, Default::default());
}
