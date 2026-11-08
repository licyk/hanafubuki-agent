use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::{
    AgentConfig, AgentLogLevel, AgentPermissionMode, AgentProxy, ConfigLayerSource, ConfigLayers,
    ConfigOverrides, AGENT_SETTINGS_FILE,
};

struct Fixture {
    _root: tempfile::TempDir,
    config_dir: PathBuf,
    project: PathBuf,
}

fn setup(user: &str) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let config_dir = root.path().join("config");
    let project = root.path().join("project");
    fs::create_dir_all(&config_dir).unwrap();
    fs::create_dir_all(project.join(crate::PROJECT_CONFIG_DIR)).unwrap();
    fs::write(config_dir.join(AGENT_SETTINGS_FILE), user).unwrap();
    fs::write(
        ConfigLayers::project_file(&project),
        "webFetchUseJinaReader = true\n",
    )
    .unwrap();
    Fixture {
        _root: root,
        config_dir,
        project,
    }
}

impl Fixture {
    fn layered(&self, profile: Option<&str>, overrides: &[&str]) -> AgentConfig {
        AgentConfig::layered(
            &self.config_dir,
            ConfigLayers {
                profile: profile.map(str::to_string),
                project: Some(self.project.clone()),
                overrides: ConfigOverrides::parse(overrides).unwrap(),
            },
        )
    }

    fn user_file(&self) -> String {
        fs::read_to_string(self.config_dir.join(AGENT_SETTINGS_FILE)).unwrap()
    }
}

/// `key` naming `path` as a quoted segment, as a person writes it.
fn project_trust_key(path: &Path) -> String {
    format!("projects.'{}'.trustLevel", path.display())
}

const USER: &str = r#"# The user's own comment.
permissionMode = "auto_review"

[memory]
enabled = true

[profiles.ci]
permissionMode = "full_access"
"#;

#[test]
fn get_reads_the_layered_value_and_the_users_tables() {
    let fixture = setup(USER);
    let config = fixture.layered(Some("ci"), &["logLevel=debug"]);
    assert_eq!(
        config.value("permissionMode").unwrap(),
        Some(json!("full_access")),
        "the profile's"
    );
    assert_eq!(config.value("logLevel").unwrap(), Some(json!("debug")));
    assert_eq!(config.value("memory.enabled").unwrap(), Some(json!(true)));
    assert_eq!(config.value("searxngBaseUrl").unwrap(), None);
    assert_eq!(
        config.value("profiles.ci.permissionMode").unwrap(),
        Some(json!("full_access"))
    );
    assert_eq!(config.value("profiles.other.logLevel").unwrap(), None);
    let document = config.resolved_document().unwrap();
    assert_eq!(document["network"]["proxy"], json!("system"));
    assert!(config
        .value("memory.unknown")
        .unwrap_err()
        .contains("not a setting"));
}

#[test]
fn set_writes_one_key_of_the_users_file_and_keeps_the_rest() {
    let fixture = setup(USER);
    // A layer above does not reach the file.
    let config = fixture.layered(Some("ci"), &["memory.enabled=false"]);
    config.set_user_value("logLevel", "warn").unwrap();
    config
        .set_user_value("network.proxy", "http://127.0.0.1:7890")
        .unwrap();
    config.set_user_value("memory.maxUnusedDays", "12").unwrap();
    config
        .set_user_value("disabledPlugins", r#"["titles"]"#)
        .unwrap();

    let file = fixture.user_file();
    assert!(file.starts_with("# The user's own comment.\npermissionMode = \"auto_review\"\n"));
    assert!(file.contains("logLevel = \"warn\""), "{file}");
    assert!(file.contains("maxUnusedDays = 12"), "{file}");
    assert!(file.contains("enabled = true"), "the override is not written: {file}");

    // The layers apply over the written file, read again.
    let standalone = config.standalone_settings().unwrap();
    assert_eq!(standalone.log_level, AgentLogLevel::Warn);
    assert_eq!(
        standalone.network.proxy,
        AgentProxy::Explicit("http://127.0.0.1:7890".into())
    );
    let settings = config.settings().unwrap();
    assert_eq!(settings.memory.max_unused_days, 12);
    assert!(!settings.memory.enabled, "-c still applies");
    assert!(settings.disabled_plugins.contains("titles"));
}

#[test]
fn set_refuses_unknown_keys_and_unusable_values() {
    let fixture = setup(USER);
    let config = fixture.layered(None, &[]);
    let before = fixture.user_file();
    for (key, value, expected) in [
        ("logLevle", "debug", "not a setting"),
        ("memory.maxUnusedDays", "0", "cannot take this value"),
        ("permissionMode", "everything", "cannot take this value"),
        ("network.proxy", "ftp://proxy", "cannot take this value"),
        ("webSearchProvider", "searxng", "cannot take this value"),
        ("profiles.ci.logLevel", "loud", "profile `ci`"),
        ("profiles.ci", "1", "names no setting"),
        ("projects.'/a'.trust", "trusted", "names no setting"),
        ("projects.'/a'.trustLevel", "maybe", "\"trusted\" or \"untrusted\""),
        ("memory..enabled", "true", "not a key"),
    ] {
        let error = config.set_user_value(key, value).unwrap_err();
        assert!(error.contains(expected), "{key}={value}: {error}");
    }
    assert_eq!(fixture.user_file(), before);

    // A value the file already could not use does not block another key.
    fs::write(
        fixture.config_dir.join(AGENT_SETTINGS_FILE),
        "logLevel = \"loud\"\n",
    )
    .unwrap();
    config.set_user_value("permissionMode", "manual").unwrap();
}

#[test]
fn set_writes_profiles_and_trust_which_the_layers_then_read() {
    let fixture = setup(USER);
    let config = fixture.layered(None, &[]);
    assert!(!config.settings().unwrap().web_fetch_use_jina_reader);
    assert_eq!(
        config.layer_sources().unwrap()[1],
        ConfigLayerSource::Project {
            path: ConfigLayers::project_file(&fixture.project),
            trusted: false
        }
    );

    let key = project_trust_key(&fixture.project);
    config.set_user_value(&key, "trusted").unwrap();
    assert_eq!(config.value(&key).unwrap(), Some(json!("trusted")));
    assert!(config.settings().unwrap().web_fetch_use_jina_reader);
    assert!(matches!(
        config.layer_sources().unwrap()[1],
        ConfigLayerSource::Project { trusted: true, .. }
    ));

    config
        .set_user_value("profiles.review.permissionMode", "manual")
        .unwrap();
    let review = fixture.layered(Some("review"), &["logLevel=trace"]);
    assert_eq!(
        review.settings().unwrap().permission_mode,
        AgentPermissionMode::Manual
    );
    assert_eq!(
        review.layer_sources().unwrap(),
        vec![
            ConfigLayerSource::User {
                path: fixture.config_dir.join(AGENT_SETTINGS_FILE),
                present: true
            },
            ConfigLayerSource::Profile {
                name: "review".into()
            },
            ConfigLayerSource::Project {
                path: ConfigLayers::project_file(&fixture.project),
                trusted: true
            },
            ConfigLayerSource::Overrides,
        ]
    );
    let file = fixture.user_file();
    assert!(file.contains("[profiles.review]"), "{file}");
    assert!(file.contains("trustLevel = \"trusted\""), "{file}");
}
