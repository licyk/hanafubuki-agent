//! The settings of `config.toml` by key: their specs against the structs
//! that hold them, and the writes and provenance built on them.

mod sections;
mod write;

use std::fs;
use std::path::{Path, PathBuf};

use hanafubuki_agent_config::{
    AgentConfig, ConfigLayers, ConfigOverrides, SettingChange, SettingWriteOp, SettingsWrite,
    AGENT_SETTINGS_FILE, PROJECT_CONFIG_DIR,
};
use serde_json::Value;

/// A configuration directory with the user's `config.toml`, and a project
/// with its own file.
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
    fs::write(config_dir.join(AGENT_SETTINGS_FILE), user).unwrap();
    fs::write(ConfigLayers::project_file(&project_dir), project).unwrap();
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

    fn trust_project(&self, config: &AgentConfig) {
        config
            .set_user_value(&project_trust_key(&self.project), "trusted")
            .unwrap();
    }
}

fn project_trust_key(path: &Path) -> String {
    format!("projects.'{}'.trustLevel", path.display())
}

fn set(key: &str, value: Value) -> SettingChange {
    SettingChange {
        key: key.to_string(),
        op: SettingWriteOp::Set(value),
    }
}

fn reset(key: &str) -> SettingChange {
    SettingChange {
        key: key.to_string(),
        op: SettingWriteOp::Reset,
    }
}

fn write(changes: Vec<SettingChange>) -> SettingsWrite {
    SettingsWrite { changes }
}
