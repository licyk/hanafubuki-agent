use std::fs;

use serde_json::json;

use crate::{
    AgentAlternateScreen, AgentConfig, AgentTuiPreferences, AgentTuiPreferencesPatch, ConfigLayers,
    ConfigOverrides, AGENT_SETTINGS_FILE,
};

fn config_with(user: &str) -> (tempfile::TempDir, AgentConfig) {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(AGENT_SETTINGS_FILE), user).unwrap();
    let config = AgentConfig::layered(root.path(), ConfigLayers::default());
    (root, config)
}

#[test]
fn missing_preferences_are_the_defaults() {
    let (_root, config) = config_with("");
    assert_eq!(
        config.tui_preferences().unwrap(),
        AgentTuiPreferences {
            fullscreen_transcript: true,
            alternate_screen: AgentAlternateScreen::Auto,
            raw_output: false,
        }
    );
}

#[test]
fn each_key_decodes_on_its_own_and_unknown_keys_are_ignored() {
    let (root, config) = config_with(
        "[tui]\nfullscreenTranscript = false\nalternateScreen = \"sometimes\"\n\
         rawOutput = true\ntheme = \"later\"\n",
    );
    assert_eq!(
        config.tui_preferences().unwrap(),
        AgentTuiPreferences {
            fullscreen_transcript: false,
            alternate_screen: AgentAlternateScreen::Auto,
            raw_output: true,
        }
    );
    // The standalone keys are read, never repaired in the user's file.
    let file = fs::read_to_string(root.path().join(AGENT_SETTINGS_FILE)).unwrap();
    assert!(file.contains("theme = \"later\""), "{file}");
    assert!(file.contains("alternateScreen = \"sometimes\""), "{file}");
}

#[test]
fn a_table_that_is_not_a_table_takes_the_defaults() {
    let (_root, config) = config_with("tui = 3\n");
    assert_eq!(
        config.tui_preferences().unwrap(),
        AgentTuiPreferences::default()
    );
}

#[test]
fn a_write_changes_only_the_keys_named_and_reads_back() {
    let (root, config) = config_with(
        "# mine\nlogLevel = \"debug\"\n\n[tui]\nalternateScreen = \"never\"\nlater = 1\n",
    );
    let written = config
        .write_tui_preferences(AgentTuiPreferencesPatch {
            raw_output: Some(true),
            ..AgentTuiPreferencesPatch::default()
        })
        .unwrap();
    assert_eq!(
        written,
        AgentTuiPreferences {
            fullscreen_transcript: true,
            alternate_screen: AgentAlternateScreen::Never,
            raw_output: true,
        }
    );
    let file = fs::read_to_string(root.path().join(AGENT_SETTINGS_FILE)).unwrap();
    for kept in [
        "# mine",
        "logLevel = \"debug\"",
        "later = 1",
        "rawOutput = true",
    ] {
        assert!(file.contains(kept), "{kept}: {file}");
    }
    assert!(!file.contains("fullscreenTranscript"), "{file}");
}

#[test]
fn the_layers_apply_over_the_written_preferences() {
    let root = tempfile::tempdir().unwrap();
    let config = AgentConfig::layered(
        root.path(),
        ConfigLayers {
            overrides: ConfigOverrides::parse(&["tui.fullscreenTranscript=false"]).unwrap(),
            ..ConfigLayers::default()
        },
    );
    let written = config
        .write_tui_preferences(AgentTuiPreferencesPatch {
            fullscreen_transcript: Some(true),
            ..AgentTuiPreferencesPatch::default()
        })
        .unwrap();
    assert!(!written.fullscreen_transcript);
}

#[test]
fn config_set_reaches_the_tui_keys_and_checks_their_values() {
    let (_root, config) = config_with("");
    config
        .set_user_value("tui.alternateScreen", "always")
        .unwrap();
    assert_eq!(
        config.value("tui.alternateScreen").unwrap(),
        Some(json!("always"))
    );
    assert_eq!(
        config.tui_preferences().unwrap().alternate_screen,
        AgentAlternateScreen::Always
    );
    let error = config
        .set_user_value("tui.rawOutput", "\"yes\"")
        .unwrap_err();
    assert!(error.contains("cannot take this value"), "{error}");
    let error = config.set_user_value("tui.nothing", "true").unwrap_err();
    assert!(error.contains("is not a setting"), "{error}");
}
