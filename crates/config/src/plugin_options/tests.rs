use std::fs;
use std::path::PathBuf;

use hanafubuki_agent_foundation::settings::{SettingKind, SettingSpec, SettingValue};
use serde_json::json;

use super::*;
use crate::{AgentConfig, ConfigLayers, ConfigOverrides, PROJECT_CONFIG_DIR};

fn schema() -> PluginOptionSchema {
    let integer = |key: &str, default| SettingSpec {
        default: Some(SettingValue::Integer(default)),
        ..SettingSpec::new(
            key,
            key,
            SettingKind::Integer {
                minimum: Some(1),
                maximum: Some(16),
            },
        )
    };
    PluginOptionSchema {
        options: vec![
            integer("agents", 3),
            integer("wait", 10),
            integer("limit", 5),
        ],
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    config_dir: PathBuf,
    project: PathBuf,
}

fn setup(user: &str, project: &str) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let config_dir = root.path().join("config");
    let project_dir = root.path().join("project");
    fs::create_dir_all(&config_dir).unwrap();
    fs::create_dir_all(project_dir.join(PROJECT_CONFIG_DIR)).unwrap();
    let trust = format!(
        "[projects.\"{}\"]\ntrustLevel = \"trusted\"\n",
        project_dir.display().to_string().replace('\\', "\\\\")
    );
    fs::write(config_dir.join("config.toml"), format!("{user}\n{trust}")).unwrap();
    fs::write(ConfigLayers::project_file(&project_dir), project).unwrap();
    Fixture {
        _root: root,
        config_dir,
        project: project_dir,
    }
}

impl Fixture {
    fn layered(&self, overrides: &[&str]) -> AgentConfig {
        AgentConfig::layered(
            &self.config_dir,
            ConfigLayers {
                profile: Some("ci".to_string()),
                project: Some(self.project.clone()),
                overrides: ConfigOverrides::parse(overrides).unwrap(),
            },
        )
    }

    fn user_file(&self) -> String {
        fs::read_to_string(self.config_dir.join("config.toml")).unwrap()
    }
}

const USER: &str = r#"
[pluginOptions.subagents]
agents = 4
wait = 12
# kept
legacy = true

[profiles.ci.pluginOptions.subagents]
wait = 14
"#;

#[test]
fn options_are_layered_per_plugin_and_key_and_each_value_names_its_layer() {
    let fixture = setup(USER, "[pluginOptions.subagents]\nlimit = 99\n");
    let config = fixture.layered(&["pluginOptions.subagents.agents=6"]);
    let settings = config.settings().unwrap();
    let resolved = settings.plugin_options.resolve("subagents", &schema());
    assert_eq!(
        serde_json::Value::Object(resolved.to_object()),
        json!({ "agents": 6, "wait": 14, "limit": 16 })
    );
    let layers = resolved
        .values
        .iter()
        .map(|option| option.layer.clone().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(layers[0], "-c");
    assert_eq!(layers[1], "profile `ci`");
    assert!(layers[2].ends_with("config.toml") && layers[2].contains(PROJECT_CONFIG_DIR));
    assert_eq!(resolved.issues.len(), 2, "{:?}", resolved.issues);
    assert!(resolved.issues[0].contains("pluginOptions.subagents.limit is 99"));
    assert!(resolved.issues[1].contains("pluginOptions.subagents.legacy is not an option"));
    // The table is known to the layers: no unknown-key warning for it.
    assert!(
        config.layer_warnings().unwrap().is_empty(),
        "{:?}",
        config.layer_warnings()
    );
}

#[test]
fn a_plugin_no_schema_reads_and_unset_options_keep_their_defaults() {
    let fixture = setup("[pluginOptions.other]\nanything = 1\n[profiles.ci]\n", "");
    let settings = fixture.layered(&[]).settings().unwrap();
    let resolved = settings.plugin_options.resolve("subagents", &schema());
    assert!(resolved.issues.is_empty());
    assert!(resolved.values.iter().all(|option| option.layer.is_none()));
    assert_eq!(
        settings.plugin_options.value("other", "anything"),
        Some(&json!(1))
    );
}

#[test]
fn a_write_is_checked_before_anything_is_written() {
    let fixture = setup(USER, "");
    let config = fixture.layered(&[]);
    let before = fixture.user_file();
    let patch = |value| json!({ "agents": value }).as_object().unwrap().clone();
    let refused = config
        .set_plugin_options("subagents", &schema(), &patch(json!(17)))
        .unwrap_err();
    assert_eq!(refused, "`agents` is between 1 and 16");
    let unknown = json!({ "speed": 1 }).as_object().unwrap().clone();
    assert!(config
        .set_plugin_options("subagents", &schema(), &unknown)
        .unwrap_err()
        .contains("no option `speed`"));
    assert_eq!(fixture.user_file(), before);

    let settings = config
        .set_plugin_options("subagents", &schema(), &patch(json!(8)))
        .unwrap();
    assert_eq!(
        settings.plugin_options.value("subagents", "agents"),
        Some(&json!(8))
    );
    let written = fixture.user_file();
    assert!(written.contains("agents = 8"), "{written}");
    assert!(written.contains("# kept\nlegacy = true"), "{written}");

    config
        .set_plugin_options("subagents", &schema(), &patch(json!(null)))
        .unwrap();
    assert!(!fixture.user_file().contains("agents ="));
}

#[test]
fn config_set_names_a_plugin_option_by_its_key_and_refuses_the_unchecked_path() {
    assert_eq!(
        PluginOptionKey::parse("pluginOptions.subagents.maximumRunningAgents").unwrap(),
        Some(PluginOptionKey {
            row: "subagents".to_string(),
            key: "maximumRunningAgents".to_string(),
        })
    );
    assert_eq!(PluginOptionKey::parse("memory.enabled").unwrap(), None);
    let fixture = setup(USER, "");
    let config = fixture.layered(&[]);
    for key in [
        "pluginOptions.subagents.agents",
        "profiles.ci.pluginOptions.subagents.agents",
    ] {
        let error = config.set_user_value(key, "5").unwrap_err();
        assert!(error.contains("plugin option"), "{key}: {error}");
    }
}
