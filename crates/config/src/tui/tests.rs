use std::collections::BTreeMap;
use std::fs;

use serde_json::json;

use crate::{
    AgentAlternateScreen, AgentConfig, AgentScrollbackMouse, AgentTuiPreferences,
    AgentTuiPreferencesPatch, AgentTuiPreset, ConfigLayers, ConfigOverrides, AGENT_SETTINGS_FILE,
    PROJECT_CONFIG_DIR,
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
            scrollback_mouse: AgentScrollbackMouse::Tail,
            raw_output: false,
            theme: None,
            status_line: vec![
                "model-with-reasoning".to_string(),
                "current-dir".to_string(),
                "session-title".to_string(),
            ],
            status_line_colors: true,
            terminal_title: vec![
                "activity".to_string(),
                "session-title".to_string(),
                "project-name".to_string(),
            ],
            keymap: BTreeMap::new(),
            presets: BTreeMap::new(),
        }
    );
}

#[test]
fn each_key_decodes_on_its_own_and_unknown_keys_are_ignored() {
    let (root, config) = config_with(
        "[tui]\nfullscreenTranscript = false\nalternateScreen = \"sometimes\"\n\
         scrollbackMouse = \"off\"\nrawOutput = true\nkeymap = \"later\"\ntheme = \"nord\"\n\
         statusLine = [\"model\", 3]\nstatusLineColors = false\n\
         terminalTitle = [\"app-name\", \"unknown-item\"]\n",
    );
    assert_eq!(
        config.tui_preferences().unwrap(),
        AgentTuiPreferences {
            fullscreen_transcript: false,
            alternate_screen: AgentAlternateScreen::Auto,
            scrollback_mouse: AgentScrollbackMouse::Off,
            raw_output: true,
            theme: Some("nord".to_string()),
            // Not a list of ids: the default.
            status_line: AgentTuiPreferences::default().status_line,
            status_line_colors: false,
            // The interface leaves out the ids it does not know.
            terminal_title: vec!["app-name".to_string(), "unknown-item".to_string()],
            // Not a table: none.
            keymap: BTreeMap::new(),
            presets: BTreeMap::new(),
        }
    );
    // The standalone keys are read, never repaired in the user's file.
    let file = fs::read_to_string(root.path().join(AGENT_SETTINGS_FILE)).unwrap();
    assert!(file.contains("keymap = \"later\""), "{file}");
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
            ..AgentTuiPreferences::default()
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
    config.set_user_value("tui.scrollbackMouse", "off").unwrap();
    assert_eq!(
        config.tui_preferences().unwrap().scrollback_mouse,
        AgentScrollbackMouse::Off
    );
    let error = config
        .set_user_value("tui.rawOutput", "\"yes\"")
        .unwrap_err();
    assert!(error.contains("cannot take this value"), "{error}");
    let error = config.set_user_value("tui.nothing", "true").unwrap_err();
    assert!(error.contains("is not a setting"), "{error}");
}

#[test]
fn the_theme_and_the_status_surfaces_are_written_and_read_back() {
    let (root, config) = config_with("");
    let written = config
        .write_tui_preferences(AgentTuiPreferencesPatch {
            theme: Some("dracula".to_string()),
            status_line: Some(vec!["git-branch".to_string(), "model".to_string()]),
            status_line_colors: Some(false),
            terminal_title: Some(Vec::new()),
            ..AgentTuiPreferencesPatch::default()
        })
        .unwrap();
    assert_eq!(written.theme.as_deref(), Some("dracula"));
    assert_eq!(written.status_line, ["git-branch", "model"]);
    assert!(!written.status_line_colors);
    assert!(written.terminal_title.is_empty());
    let file = fs::read_to_string(root.path().join(AGENT_SETTINGS_FILE)).unwrap();
    for kept in [
        "theme = \"dracula\"",
        "statusLine = [\"git-branch\", \"model\"]",
        "statusLineColors = false",
        "terminalTitle = []",
    ] {
        assert!(file.contains(kept), "{kept}: {file}");
    }
}

#[test]
fn config_set_reaches_the_theme_and_the_item_lists() {
    let (_root, config) = config_with("");
    config.set_user_value("tui.theme", "nord").unwrap();
    config
        .set_user_value("tui.statusLine", "[\"model\", \"goal\"]")
        .unwrap();
    config.set_user_value("tui.terminalTitle", "[]").unwrap();
    let preferences = config.tui_preferences().unwrap();
    assert_eq!(preferences.theme.as_deref(), Some("nord"));
    assert_eq!(preferences.status_line, ["model", "goal"]);
    assert!(preferences.terminal_title.is_empty());
    let error = config
        .set_user_value("tui.statusLine", "\"model\"")
        .unwrap_err();
    assert!(error.contains("cannot take this value"), "{error}");
}

fn preset(input: &str, label: Option<&str>) -> AgentTuiPreset {
    AgentTuiPreset {
        input: input.to_string(),
        label: label.map(str::to_string),
    }
}

#[test]
fn the_keymap_and_the_presets_decode_entry_by_entry() {
    let (_root, config) = config_with(
        "[tui.keymap.global]\nopen_transcript = \"ctrl-y\"\nfind_transcript = [\"f3\", \"ctrl-f\"]\n\
         select_transcript = []\nbroken = 3\n\
         [tui.keymap.chat]\nnothing = { a = 1 }\n\
         [tui.keymap]\nnot_a_context = \"x\"\n\
         [tui.presets]\n1 = { input = \"/status\", label = \"Status\", later = true }\n\
         2 = { input = \"!ls\" }\n12 = { input = \"hello\" }\n3 = \"hello\"\n\
         0 = { input = \"hello\" }\n",
    );
    let preferences = config.tui_preferences().unwrap();
    let global = &preferences.keymap["global"];
    assert_eq!(global["open_transcript"], ["ctrl-y"]);
    assert_eq!(global["find_transcript"], ["f3", "ctrl-f"]);
    assert!(global["select_transcript"].is_empty());
    assert!(!global.contains_key("broken"));
    // A context left with nothing is not kept.
    assert_eq!(preferences.keymap.keys().collect::<Vec<_>>(), ["global"]);
    assert_eq!(
        preferences.presets,
        BTreeMap::from([
            ("0".to_string(), preset("hello", None)),
            ("1".to_string(), preset("/status", Some("Status"))),
        ])
    );
    let warnings = config.layer_warnings().unwrap();
    for wanted in [
        "tui.keymap.global.broken",
        "tui.keymap.chat.nothing",
        "tui.keymap.not_a_context",
        "tui.presets.2",
        "tui.presets.12",
        "tui.presets.3",
    ] {
        assert!(
            warnings.iter().any(|warning| warning.contains(wanted)),
            "{wanted}: {warnings:#?}"
        );
    }
}

#[test]
fn keymap_entries_and_presets_are_written_and_removed() {
    let (root, config) = config_with(
        "[tui.keymap.global]\nfind_transcript = \"f3\"\n[tui.presets.4]\ninput = \"/diff\"\nlabel = \"Diff\"\n",
    );
    let written = config
        .write_tui_preferences(AgentTuiPreferencesPatch {
            keymap: BTreeMap::from([(
                "global".to_string(),
                BTreeMap::from([
                    (
                        "open_transcript".to_string(),
                        Some(vec!["ctrl-y".to_string()]),
                    ),
                    ("select_transcript".to_string(), Some(Vec::new())),
                    ("find_transcript".to_string(), None),
                ]),
            )]),
            presets: BTreeMap::from([
                ("1".to_string(), Some(preset("/status", Some("Status")))),
                ("4".to_string(), Some(preset("/diff", None))),
            ]),
            ..AgentTuiPreferencesPatch::default()
        })
        .unwrap();
    let global = &written.keymap["global"];
    assert_eq!(global["open_transcript"], ["ctrl-y"]);
    assert!(global["select_transcript"].is_empty());
    assert!(!global.contains_key("find_transcript"));
    // The label left out is removed from the file.
    assert_eq!(written.presets["4"], preset("/diff", None));
    assert_eq!(written.presets["1"], preset("/status", Some("Status")));
    let file = fs::read_to_string(root.path().join(AGENT_SETTINGS_FILE)).unwrap();
    assert!(file.contains("open_transcript = \"ctrl-y\""), "{file}");
    assert!(!file.contains("Diff"), "{file}");

    let written = config
        .write_tui_preferences(AgentTuiPreferencesPatch {
            presets: BTreeMap::from([("1".to_string(), None)]),
            ..AgentTuiPreferencesPatch::default()
        })
        .unwrap();
    assert_eq!(written.presets.keys().collect::<Vec<_>>(), ["4"]);
    assert!(config.layer_warnings().unwrap().is_empty());
}

#[test]
fn a_layer_merges_its_keymap_entries_and_presets_over_the_users() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    fs::create_dir_all(project.join(PROJECT_CONFIG_DIR)).unwrap();
    fs::write(
        root.path().join(AGENT_SETTINGS_FILE),
        format!(
            "[tui.keymap.global]\nopen_transcript = \"ctrl-y\"\nfind_transcript = \"f5\"\n\
             [tui.presets]\n1 = {{ input = \"/status\" }}\n\
             [projects.{:?}]\ntrustLevel = \"trusted\"\n",
            project.display().to_string()
        ),
    )
    .unwrap();
    fs::write(
        ConfigLayers::project_file(&project),
        "[tui.keymap.global]\nfind_transcript = \"f6\"\n[tui.presets]\n2 = { input = \"/diff\" }\nnope = 1\n",
    )
    .unwrap();
    let config = AgentConfig::layered(
        root.path(),
        ConfigLayers {
            project: Some(project),
            ..ConfigLayers::default()
        },
    );
    let preferences = config.tui_preferences().unwrap();
    assert_eq!(preferences.keymap["global"]["open_transcript"], ["ctrl-y"]);
    assert_eq!(preferences.keymap["global"]["find_transcript"], ["f6"]);
    assert_eq!(preferences.presets.keys().collect::<Vec<_>>(), ["1", "2"]);
    let warnings = config.layer_warnings().unwrap();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains(PROJECT_CONFIG_DIR)
                && warning.contains("tui.presets.nope")),
        "{warnings:#?}"
    );
}
