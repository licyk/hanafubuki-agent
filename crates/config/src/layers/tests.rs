use std::fs;

use super::*;
use crate::{AgentConfig, AgentLogLevel, AgentPermissionMode, AgentProxy, AgentSettingsPatch};

/// A configuration directory and a project directory, with the user's
/// `config.toml` and the project's file as given (`None` for none).
struct Fixture {
    _root: tempfile::TempDir,
    config_dir: PathBuf,
    project: PathBuf,
}

fn setup(user: &str, project: Option<&str>) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let config_dir = root.path().join("config");
    let project_dir = root.path().join("project");
    fs::create_dir_all(&config_dir).unwrap();
    fs::create_dir_all(project_dir.join(PROJECT_CONFIG_DIR)).unwrap();
    let user = user.replace(
        "<PROJECT>",
        &project_dir.display().to_string().replace('\\', "\\\\"),
    );
    fs::write(config_dir.join(AGENT_SETTINGS_FILE), user).unwrap();
    if let Some(project) = project {
        fs::write(ConfigLayers::project_file(&project_dir), project).unwrap();
    }
    Fixture {
        _root: root,
        config_dir,
        project: project_dir,
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

const TRUSTED_USER: &str = r#"
permissionMode = "auto_review"
logLevel = "warn"

[memory]
enabled = true
maxUnusedDays = 40

[profiles.work]
permissionMode = "full_access"
logLevel = "debug"
memory.maxUnusedDays = 20

[projects."<PROJECT>"]
trustLevel = "trusted"
"#;

const PROJECT: &str = r#"
logLevel = "error"
webFetchUseJinaReader = true

[memory]
maxUnusedDays = 10
"#;

#[test]
fn each_layer_overrides_the_one_below_key_by_key() {
    let fixture = setup(TRUSTED_USER, Some(PROJECT));

    // The user's file alone.
    let user = AgentConfig::new(&fixture.config_dir).settings().unwrap();
    assert_eq!(user.permission_mode, AgentPermissionMode::AutoReview);
    assert_eq!(user.memory.max_unused_days, 40);
    assert!(!user.web_fetch_use_jina_reader);

    // The profile over the user's file, the project over the profile.
    let config = fixture.layered(Some("work"), &[]);
    let settings = config.settings().unwrap();
    assert_eq!(settings.permission_mode, AgentPermissionMode::FullAccess);
    assert!(settings.memory.enabled, "kept from the user's file");
    assert_eq!(settings.memory.max_unused_days, 10, "the project's");
    assert!(settings.web_fetch_use_jina_reader, "the project's");
    assert_eq!(
        config.standalone_settings().unwrap().log_level,
        AgentLogLevel::Error
    );

    // `-c` over everything.
    let config = fixture.layered(
        Some("work"),
        &[
            "memory.maxUnusedDays=5",
            "logLevel=trace",
            "permissionMode=manual",
        ],
    );
    let settings = config.settings().unwrap();
    assert_eq!(settings.memory.max_unused_days, 5);
    assert_eq!(settings.permission_mode, AgentPermissionMode::Manual);
    assert!(settings.memory.enabled);
    assert_eq!(
        config.standalone_settings().unwrap().log_level,
        AgentLogLevel::Trace
    );

    // Without a profile the user's file is the base of the project.
    let settings = fixture.layered(None, &[]).settings().unwrap();
    assert_eq!(settings.permission_mode, AgentPermissionMode::AutoReview);
    assert_eq!(settings.memory.max_unused_days, 10);
}

#[test]
fn an_untrusted_project_is_not_read() {
    let untrusted = TRUSTED_USER.replace("\"trusted\"", "\"untrusted\"");
    let fixture = setup(&untrusted, Some(PROJECT));
    let config = fixture.layered(None, &[]);
    let settings = config.settings().unwrap();
    assert_eq!(settings.memory.max_unused_days, 40);
    assert!(!settings.web_fetch_use_jina_reader);
    assert_eq!(
        config.standalone_settings().unwrap().log_level,
        AgentLogLevel::Warn
    );

    // Nor is one that no entry names.
    let fixture = setup(
        "[projects.\"/elsewhere\"]\ntrustLevel = \"trusted\"\n",
        Some(PROJECT),
    );
    assert!(
        !fixture
            .layered(None, &[])
            .settings()
            .unwrap()
            .web_fetch_use_jina_reader
    );
}

#[test]
fn trust_compares_locations_not_spellings() {
    let fixture = setup("", Some(PROJECT));
    let spelled = fixture.project.join("sub").join("..");
    fs::create_dir_all(fixture.project.join("sub")).unwrap();
    fs::write(
        fixture.config_dir.join(AGENT_SETTINGS_FILE),
        format!(
            "[projects.'{}']\ntrustLevel = \"trusted\"\n",
            spelled.display()
        ),
    )
    .unwrap();
    assert!(
        fixture
            .layered(None, &[])
            .settings()
            .unwrap()
            .web_fetch_use_jina_reader
    );
}

#[test]
fn a_value_a_layer_cannot_use_keeps_the_one_below() {
    let fixture = setup(
        TRUSTED_USER,
        Some("permissionMode = \"everything\"\n[network]\nproxy = \"ftp://proxy.test:21\"\n"),
    );
    let config = fixture.layered(Some("work"), &["memory.maxUnusedDays=-1"]);
    let settings = config.settings().unwrap();
    assert_eq!(settings.permission_mode, AgentPermissionMode::FullAccess);
    assert_eq!(settings.memory.max_unused_days, 20);
    assert_eq!(
        config.standalone_settings().unwrap().network.proxy,
        AgentProxy::System
    );
    // A layer above the user's file is never repaired on disk.
    assert!(
        fs::read_to_string(ConfigLayers::project_file(&fixture.project))
            .unwrap()
            .contains("everything")
    );
}

#[test]
fn the_network_section_names_the_proxy() {
    for (value, proxy) in [
        ("system", AgentProxy::System),
        ("disabled", AgentProxy::Disabled),
        (
            "socks5h://user:secret@127.0.0.1:1080",
            AgentProxy::Explicit("socks5h://user:secret@127.0.0.1:1080".into()),
        ),
    ] {
        let fixture = setup(&format!("[network]\nproxy = \"{value}\"\n"), None);
        let config = fixture.layered(None, &[]);
        assert_eq!(config.standalone_settings().unwrap().network.proxy, proxy);
    }
    let fixture = setup("[network]\nproxy = \"disabled\"\n", None);
    let config = fixture.layered(None, &["network.proxy=http://127.0.0.1:8080"]);
    assert_eq!(
        config.standalone_settings().unwrap().network.proxy,
        AgentProxy::Explicit("http://127.0.0.1:8080".into())
    );
    for unusable in ["http://127.0.0.1:8080/?query", "file:///proxy", "nonsense"] {
        let fixture = setup(&format!("[network]\nproxy = \"{unusable}\"\n"), None);
        assert_eq!(
            fixture
                .layered(None, &[])
                .standalone_settings()
                .unwrap()
                .network
                .proxy,
            AgentProxy::System,
            "{unusable}"
        );
    }
}

#[test]
fn a_write_changes_the_users_file_and_none_of_the_layers() {
    let fixture = setup(TRUSTED_USER, Some(PROJECT));
    let config = fixture.layered(Some("work"), &["memory.maxUnusedDays=5"]);
    let written = config
        .write_settings(AgentSettingsPatch {
            web_fetch_use_jina_reader: Some(false),
            ..AgentSettingsPatch::default()
        })
        .unwrap();
    // The project still sets it, and `-c` still wins.
    assert!(written.web_fetch_use_jina_reader);
    assert_eq!(written.memory.max_unused_days, 5);
    assert_eq!(written.permission_mode, AgentPermissionMode::FullAccess);

    let user = fixture.user_file();
    assert!(user.contains("permissionMode = \"auto_review\""), "{user}");
    assert!(user.contains("maxUnusedDays = 40"), "{user}");
    assert!(user.contains("[profiles.work]"), "{user}");
    assert!(user.contains("webFetchUseJinaReader = false"), "{user}");
    assert!(!user.contains("logLevel = \"trace\""), "{user}");
    assert_eq!(
        AgentConfig::new(&fixture.config_dir)
            .settings()
            .unwrap()
            .permission_mode,
        AgentPermissionMode::AutoReview
    );
}

#[test]
fn a_profile_the_file_does_not_have_is_an_error() {
    let fixture = setup(TRUSTED_USER, None);
    let error = fixture.layered(Some("home"), &[]).settings().unwrap_err();
    assert!(error.contains("no profile `home`"), "{error}");
}

#[test]
fn a_hosts_configuration_has_no_layers_and_the_standalone_defaults() {
    let fixture = setup(TRUSTED_USER, Some(PROJECT));
    let config = AgentConfig::new(&fixture.config_dir);
    assert_eq!(
        config.settings().unwrap().permission_mode,
        AgentPermissionMode::AutoReview
    );
    assert_eq!(
        config.standalone_settings().unwrap(),
        AgentStandaloneSettings::default()
    );
}
