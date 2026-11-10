//! An installed plugin's hooks: discovered after the user's, keyed by the
//! plugin and the file's path in it, trusted by their definition and their
//! package's content, refused once the package changes, and run with the
//! plugin's environment variables.

use std::fs;
use std::path::{Path, PathBuf};

use hanafubuki_agent_hooks::{
    HookEventName, HookSource, HookSources, HookTrust, HooksEngine, PluginHookDeclaration,
    PluginHookSource,
};
use hanafubuki_agent_process::executor::ExecutorManager;
use serde_json::json;
use tempfile::TempDir;

struct Fixture {
    root: TempDir,
    engine: HooksEngine,
}

impl Fixture {
    fn package(&self) -> PathBuf {
        self.root.path().join("cache/local/demo/1.0.0")
    }

    fn data(&self) -> PathBuf {
        self.root.path().join("data/demo-local")
    }

    fn plugin(&self, declarations: Vec<PluginHookDeclaration>) -> PluginHookSource {
        PluginHookSource {
            id: "demo@local".to_string(),
            root: self.package(),
            data: self.data(),
            declarations,
        }
    }
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let user = root.path().join("config");
    fs::create_dir_all(&user).unwrap();
    let engine = HooksEngine::new(
        HookSources {
            user,
            project: None,
            plugins: Vec::new(),
        },
        root.path().join("hooks-state.json"),
        ExecutorManager::default(),
    );
    Fixture { root, engine }
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn hook_file(command: &str) -> String {
    json!({ "hooks": { "PreToolUse": [{ "hooks": [{ "type": "command", "command": command }] }] } })
        .to_string()
}

#[test]
fn a_plugins_hooks_follow_the_users_keyed_by_the_plugin_and_the_file_in_it() {
    let fixture = fixture();
    let file = fixture.package().join("hooks/hooks.json");
    write(&file, &hook_file("check.sh"));
    let inline = json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "stop.sh" }] }] } });
    fixture.engine.set_plugin_sources(vec![fixture.plugin(vec![
        PluginHookDeclaration::File(file),
        PluginHookDeclaration::Inline {
            manifest: fixture.package().join(".claude-plugin/plugin.json"),
            index: 0,
            file: inline.as_object().unwrap().clone(),
        },
        PluginHookDeclaration::File(fixture.package().join("missing.json")),
    ])]);

    let catalog = fixture.engine.list();

    let hooks = catalog
        .hooks
        .iter()
        .map(|hook| (hook.key.as_str(), hook.event, hook.trust))
        .collect::<Vec<_>>();
    assert_eq!(
        hooks,
        [
            (
                "demo@local:hooks/hooks.json:pre_tool_use:0:0",
                HookEventName::PreToolUse,
                HookTrust::Untrusted
            ),
            (
                "demo@local:plugin.json#hooks[0]:stop:0:0",
                HookEventName::Stop,
                HookTrust::Untrusted
            ),
        ]
    );
    assert_eq!(
        catalog.hooks[0].source,
        HookSource::Plugin {
            id: "demo@local".to_string(),
            root: fixture.package(),
            data: fixture.data(),
        }
    );
    assert!(catalog.hooks[0].current_hash.contains("/sha256:"));
    assert_eq!(catalog.diagnostics.len(), 1);
    assert!(catalog.diagnostics[0].contains("missing.json"));

    fixture.engine.set_plugin_sources(Vec::new());
    assert!(fixture.engine.list().hooks.is_empty());
}

#[test]
fn a_changed_package_takes_its_hooks_trust_back() {
    let fixture = fixture();
    let file = fixture.package().join("hooks/hooks.json");
    write(&file, &hook_file("check.sh"));
    fixture
        .engine
        .set_plugin_sources(vec![fixture.plugin(vec![PluginHookDeclaration::File(file)])]);
    let key = fixture.engine.list().hooks[0].key.clone();
    fixture.engine.trust(std::slice::from_ref(&key)).unwrap();
    assert_eq!(fixture.engine.list().hooks[0].trust, HookTrust::Trusted);
    assert_eq!(fixture.engine.discover().hooks[0].trust, HookTrust::Trusted);

    write(&fixture.package().join("scripts/new.sh"), "echo changed\n");

    assert_eq!(fixture.engine.list().hooks[0].trust, HookTrust::Modified);
    fixture.engine.trust(&[key]).unwrap();
    assert_eq!(fixture.engine.list().hooks[0].trust, HookTrust::Trusted);
}

#[cfg(unix)]
mod runs {
    use hanafubuki_agent_foundation::cancellation::CancellationToken;
    use hanafubuki_agent_hooks::{HookEvent, HookScope};

    use super::*;

    fn pre_tool_use() -> HookEvent {
        HookEvent::PreToolUse {
            tool_name: "exec_command".to_string(),
            tool_input: json!({ "cmd": "ls" }),
            tool_use_id: "call-1".to_string(),
        }
    }

    fn scope(directory: &Path) -> HookScope {
        HookScope {
            session_id: "1".to_string(),
            turn_id: None,
            cwd: directory.to_path_buf(),
        }
    }

    #[test]
    fn a_plugins_hook_runs_with_its_package_and_data_and_not_once_the_package_changed() {
        let fixture = fixture();
        let file = fixture.package().join("hooks/hooks.json");
        write(
            &file,
            &hook_file(
                "cat >/dev/null; echo \"$PLUGIN_ROOT|$CLAUDE_PLUGIN_ROOT|$PLUGIN_DATA|$CLAUDE_PLUGIN_DATA\" >&2; exit 2",
            ),
        );
        fixture
            .engine
            .set_plugin_sources(vec![fixture.plugin(vec![PluginHookDeclaration::File(file)])]);
        let keys = fixture
            .engine
            .list()
            .hooks
            .into_iter()
            .map(|hook| hook.key)
            .collect::<Vec<_>>();
        fixture.engine.trust(&keys).unwrap();
        let catalog = fixture.engine.discover();
        let run = || {
            fixture.engine.run(
                &catalog,
                &pre_tool_use(),
                &scope(fixture.root.path()),
                &CancellationToken::default(),
            )
        };

        let (root, data) = (fixture.package(), fixture.data());
        let (root, data) = (root.display(), data.display());
        assert_eq!(
            run().deny.as_deref(),
            Some(format!("{root}|{root}|{data}|{data}").as_str())
        );

        write(&fixture.package().join("README.md"), "changed\n");
        let refused = run();
        assert_eq!(refused.deny, None);
        assert_eq!(
            refused.failures,
            [(
                keys[0].clone(),
                "plugin contents changed; hook approval required".to_string()
            )]
        );
    }
}
