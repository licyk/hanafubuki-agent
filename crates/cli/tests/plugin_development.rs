//! The plugin author's commands, run as the built binary: `plugins new`
//! (what it writes, and that `examples/plugins/` is its output),
//! `plugins validate` on a good and a bad plugin, and `plugins grant` and
//! `plugins settings` on an installed one.

mod support;

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use support::Home;

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The files under `root`, relative and sorted.
fn files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    found.sort();
    found
}

/// A program in the package: what a code plugin's runtime names.
fn program(path: &Path) {
    write(path, "#!/bin/sh\nexit 0\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// A code plugin `name` at `root` whose manifest is `manifest` over a
/// valid one.
fn code_plugin(root: &Path, name: &str, changes: Value) -> PathBuf {
    let mut manifest = json!({
        "name": name,
        "version": "1.0.0",
        "runtime": { "type": "process", "command": ["./guest"], "protocol": "kardia-guest/2" },
        "contributes": {
            "tools": [{ "name": "lookup", "description": "Looks up.", "inputSchema": { "type": "object" }, "timeout": 30 }],
            "contextSections": [{ "kind": "lookup_board" }],
            "prompt": { "slot": "worldState" },
            "toolLifecycleHooks": true,
            "continuationHooks": true,
            "observers": ["turn_finished", "session_archived"],
        },
        "permissions": { "network": true, "workspace": "write", "env": ["LOOKUP_TOKEN"] },
        "configSchema": { "type": "object", "properties": { "greeting": { "type": "string" } } },
    });
    merge(&mut manifest, changes);
    write(
        &root.join(".hanafubuki-plugin/plugin.json"),
        &manifest.to_string(),
    );
    program(&root.join("guest"));
    root.to_path_buf()
}

fn merge(target: &mut Value, changes: Value) {
    match (target, changes) {
        (Value::Object(target), Value::Object(changes)) => {
            for (key, change) in changes {
                merge(target.entry(key).or_insert(Value::Null), change);
            }
        }
        (target, change) => *target = change,
    }
}

#[test]
fn new_scaffolds_a_rust_or_python_plugin_and_refuses_a_bad_name_or_a_full_directory() {
    let home = Home::new();
    let rust = home.project.join("demo-plugin");
    let created = home
        .run(&[
            "plugins",
            "new",
            "demo-plugin",
            "--dir",
            rust.to_str().unwrap(),
        ])
        .expect(0)
        .stdout();
    assert!(
        created.contains("Created plugin `demo-plugin`"),
        "{created}"
    );
    assert_eq!(
        files(&rust),
        [
            ".gitignore",
            "Cargo.toml",
            "README.md",
            "package/.hanafubuki-plugin/plugin.json",
            "src/main.rs",
        ]
        .map(PathBuf::from)
    );
    let manifest: Value =
        serde_json::from_str(&read(&rust.join("package/.hanafubuki-plugin/plugin.json"))).unwrap();
    assert_eq!(manifest["name"], "demo-plugin");
    assert_eq!(manifest["runtime"]["command"], json!(["./demo-plugin"]));
    assert_eq!(
        manifest["contributes"]["contextSections"][0]["kind"],
        "demo_plugin_status"
    );
    assert_eq!(manifest["contributes"]["tools"][0]["name"], "greet");
    assert_eq!(
        manifest["contributes"]["observers"],
        json!(["turn_finished"])
    );
    assert!(manifest["configSchema"].is_object());
    let source = read(&rust.join("src/main.rs"));
    assert!(source.contains(r#"Meta::plugin("demo-plugin").provide("plugin")"#));
    assert!(read(&rust.join("Cargo.toml")).contains("kardia-guest = \"0.2.2\""));
    assert!(!read(&rust.join("README.md")).contains("{{"));

    let python = home.project.join("py");
    home.run(&[
        "plugins",
        "new",
        "py-demo",
        "--lang",
        "python",
        "--dir",
        python.to_str().unwrap(),
    ])
    .expect(0);
    assert_eq!(
        files(&python),
        [
            ".gitignore",
            "README.md",
            "package/.hanafubuki-plugin/plugin.json",
            "plugin.py",
        ]
        .map(PathBuf::from)
    );
    let manifest: Value = serde_json::from_str(&read(
        &python.join("package/.hanafubuki-plugin/plugin.json"),
    ))
    .unwrap();
    assert_eq!(
        manifest["runtime"]["command"],
        json!(["python3", "./py-demo.pyz"])
    );
    assert_eq!(manifest["runtime"]["interpreter"], true);
    assert_eq!(manifest["contributes"]["prompt"]["slot"], "capabilities");
    assert!(read(&python.join("plugin.py")).contains(r#"Meta.plugin("py-demo").provide("plugin")"#));

    home.run(&["plugins", "new", "Not_A_Name"]).expect(2);
    let full = home
        .run(&[
            "plugins",
            "new",
            "demo-plugin",
            "--dir",
            rust.to_str().unwrap(),
        ])
        .expect(1);
    assert!(full.stderr().contains("is not empty"), "{}", full.stderr());
}

/// `examples/plugins/` is `plugins new`'s output, so the examples and a
/// new plugin never drift apart.
#[test]
fn the_example_plugins_are_what_new_writes() {
    let home = Home::new();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins");
    for (name, lang) in [("hello-rust", "rust"), ("hello-python", "python")] {
        let fresh = home.project.join(name);
        home.run(&[
            "plugins",
            "new",
            name,
            "--lang",
            lang,
            "--dir",
            fresh.to_str().unwrap(),
        ])
        .expect(0);
        for file in files(&fresh) {
            assert_eq!(
                read(&fresh.join(&file)),
                read(&examples.join(name).join(&file)),
                "examples/plugins/{name}/{} is not what `plugins new` writes",
                file.display()
            );
        }
    }
}

#[test]
fn validate_accepts_a_good_plugin_and_names_what_a_bad_one_gets_wrong() {
    let home = Home::new();
    let good = code_plugin(&home.project.join("good"), "good", json!({}));
    let run = home
        .run(&["plugins", "validate", good.to_str().unwrap()])
        .expect(0);
    assert!(run.stdout().contains("No issues."), "{}", run.stdout());

    let bad = code_plugin(
        &home.project.join("bad"),
        "bad",
        json!({
            "runtime": { "command": ["./missing"], "protocol": "kardia-guest/1" },
            "contributes": {
                "tools": [
                    { "name": "has space", "inputSchema": { "type": "object" } },
                    { "name": "slow", "inputSchema": { "type": "object" }, "timeout": 0 },
                    { "name": "slow", "inputSchema": { "type": "object" } },
                ],
                "contextSections": [{ "kind": "Bad-Kind" }, { "kind": "workspace_roots" }],
                "prompt": { "slot": "sidebar" },
                "observers": ["turn_completed"],
                "widgets": true,
            },
            "configSchema": { "type": "string" },
        }),
    );
    let run = home
        .run(&["plugins", "validate", bad.to_str().unwrap(), "--json"])
        .expect(1);
    let report: Value = serde_json::from_str(&run.stdout()).unwrap();
    let issues = report["validation"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|issue| issue.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let has = |text: &str| {
        assert!(
            issues.iter().any(|issue| issue.contains(text)),
            "no issue with `{text}` in {issues:#?}"
        );
    };
    has("runtime.command: `./missing`");
    has("runtime.protocol `kardia-guest/1`");
    has("the tool `has space` would be left out");
    has("contributes.tools[1].timeout");
    has("the tool `slow` is declared twice");
    has("the context section `Bad-Kind` would be left out");
    has("the context section `workspace_roots` would be left out");
    has("its prompt's slot `sidebar`");
    has("`turn_completed`");
    has("contributes.widgets");
    has("configSchema must describe an object");
    assert_eq!(report["validation"]["code"], true);
    assert_eq!(report["started"], Value::Null);

    let empty = home.project.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let run = home
        .run(&["plugins", "validate", empty.to_str().unwrap()])
        .expect(1);
    assert!(run.stderr().contains("holds no plugin"), "{}", run.stderr());
}

#[test]
fn grant_and_settings_change_an_installed_plugins_code() {
    let home = Home::new();
    let source = code_plugin(&home.project.join("lookup"), "lookup", json!({}));
    let added = home
        .run(&["plugins", "add", source.to_str().unwrap()])
        .expect(0)
        .stdout();
    assert!(added.contains("`plugins grant lookup@local`"), "{added}");
    let info = home
        .run(&["plugins", "info", "lookup@local"])
        .expect(0)
        .stdout();
    assert!(info.contains("code:        blocked"), "{info}");

    let granted = home
        .run(&["plugins", "grant", "lookup@local"])
        .expect(0)
        .stdout();
    assert!(
        granted.contains(
            "Granted plugin `lookup@local` writing the workspace, the network and the environment variables LOOKUP_TOKEN."
        ),
        "{granted}"
    );
    let info = home
        .run(&["plugins", "info", "lookup@local", "--json"])
        .expect(0)
        .stdout();
    let info: Value = serde_json::from_str(&info).unwrap();
    assert_eq!(
        info["guest"]["grantedPermissions"],
        json!({ "network": true, "workspace": "write", "env": ["LOOKUP_TOKEN"] })
    );
    assert_ne!(info["guest"]["status"]["state"], "blocked");

    let refused = home
        .run(&[
            "plugins",
            "settings",
            "lookup@local",
            "--set",
            r#"{"greeting": 7}"#,
        ])
        .expect(1);
    assert!(
        refused.stderr().contains("configSchema"),
        "{}",
        refused.stderr()
    );
    home.run(&["plugins", "settings", "lookup@local", "--set", "{"])
        .expect(2);
    let set = home
        .run(&[
            "plugins",
            "settings",
            "lookup@local",
            "--set",
            r#"{"greeting": "Hi"}"#,
        ])
        .expect(0)
        .stdout();
    assert!(set.contains("Set the settings"), "{set}");
    let shown = home
        .run(&["plugins", "settings", "lookup@local", "--get"])
        .expect(0)
        .stdout();
    let shown: Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(shown["settings"], json!({ "greeting": "Hi" }));
    assert_eq!(shown["configSchema"]["type"], "object");
    assert!(home.file("config.toml").contains("greeting = \"Hi\""));
    home.run(&["plugins", "settings", "lookup@local", "--set", "null"])
        .expect(0);
    let shown = home
        .run(&["plugins", "settings", "lookup@local"])
        .expect(0)
        .stdout();
    let shown: Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(shown["settings"], json!({}));

    let content = home.project.join("content");
    write(
        &content.join(".claude-plugin/plugin.json"),
        &json!({ "name": "content" }).to_string(),
    );
    home.run(&["plugins", "add", content.to_str().unwrap()])
        .expect(0);
    let run = home.run(&["plugins", "grant", "content@local"]).expect(1);
    assert!(run.stderr().contains("has no code"), "{}", run.stderr());
}
