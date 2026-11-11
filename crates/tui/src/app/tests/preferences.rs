//! `/keymap`, `/presets`, `/config`, `/warnings` (the roles' issues among
//! them) and `/memories` end to end
//! over the standalone Agent: driven by keys, with what they saved read
//! back from `config.toml` and what they changed seen on the screen.

use hanafubuki_agent::test_support::assistant_events;
use hanafubuki_agent::{
    AgentModelDiscovery, AgentTuiPreferences, AgentTuiPreferencesPatch, AgentTuiPreset,
};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::Harness;
use crate::app::preferences::shadowed;
use crate::history_cell::HistoryRenderMode;
use crate::transcript_mode::TranscriptMode;

impl Harness {
    fn ctrl(&mut self, letter: char) {
        self.key_with(KeyCode::Char(letter), KeyModifiers::CONTROL);
    }

    fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            self.key(KeyCode::Char(character));
        }
    }

    /// Runs `command` and waits for its view's `title`.
    fn open(&mut self, command: &str, title: &str) -> String {
        self.type_and_submit(command);
        self.pump_until_screen_shows(title)
    }

    fn agent_memory(&self) -> hanafubuki_agent::AgentMemorySettings {
        self.agent_settings().memory
    }

    fn agent_settings(&self) -> hanafubuki_agent::AgentSettings {
        let agent = self.runtime.slot().agent().unwrap();
        let call = agent.clone();
        agent
            .runtime()
            .block_on(async move { call.settings_view().await })
            .unwrap()
            .unwrap()
            .settings
    }
}

#[test]
fn keymap_rebinds_ctrl_t_with_a_captured_key_and_refuses_a_conflict() {
    let mut harness = Harness::start(Vec::new());
    harness.open("/keymap", "Pick an action");
    harness.type_text("open_transcript");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Replace with a key");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Press the new key now");
    // Ctrl-O is chat.toggle_thinking's: refused, and the view stays.
    harness.ctrl('o');
    let refused = harness.pump_until_screen_shows("is bound to both");
    assert!(refused.contains("chat.toggle_thinking"), "{refused}");
    // Ctrl-Y is the editor's yank.
    harness.ctrl('y');
    harness.pump_until_screen_shows("and editor.yank");
    harness.ctrl('g');
    harness.pump_until_screen_shows("global.open_transcript: ctrl-g.");
    let file = harness.config_file();
    assert!(file.contains("open_transcript = \"ctrl-g\""), "{file}");

    // Ctrl-T no longer opens the transcript; Ctrl-G does.
    harness.ctrl('t');
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(!harness.screen().contains("T R A N S C R I P T"));
    harness.ctrl('g');
    harness.pump_until_screen_shows("T R A N S C R I P T");
    harness.key(KeyCode::Char('q'));

    // `/keymap debug` lists it as configured.
    harness.type_and_submit("/keymap debug");
    harness.pump_until("the debug listing", |harness| {
        harness.transcript().contains("(configured)")
    });
    let debug = harness.transcript();
    let line = debug
        .lines()
        .find(|line| line.trim_start().starts_with("global.open_transcript"))
        .expect(&debug);
    assert!(
        line.contains("ctrl-g") && line.contains("(configured)"),
        "{line}"
    );
}

#[test]
fn the_leader_chord_starts_select_mode_and_a_chord_can_be_captured() {
    let mut harness = Harness::owned(Vec::new());
    harness.ctrl('x');
    harness.key(KeyCode::Char('['));
    harness.pump_until_screen_shows("Nothing to select yet");

    harness.open("/keymap", "Pick an action");
    harness.type_text("toggle_thinking");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Replace with two keys");
    harness.key(KeyCode::Down);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Press the first key");
    harness.ctrl('g');
    harness.pump_until_screen_shows("First key: ctrl-g");
    harness.key(KeyCode::Char('o'));
    harness.pump_until_screen_shows("chat.toggle_thinking: ctrl-g o.");
    assert!(harness
        .config_file()
        .contains("toggle_thinking = \"ctrl-g o\""));
}

#[test]
fn presets_are_written_run_edited_and_deleted() {
    let mut harness = Harness::start(Vec::new());
    harness.open("/presets", "No presets saved");
    harness.key(KeyCode::Char('n'));
    harness.pump_until_screen_shows("New preset");
    // Slot, Input, Label, Save.
    harness.key(KeyCode::Tab);
    harness.type_text("!ls");
    harness.key(KeyCode::Tab);
    harness.type_text("Status");
    harness.key(KeyCode::Tab);
    harness.key(KeyCode::Enter);
    // The error takes the focus to the input.
    harness.pump_until_screen_shows("not a shell command");
    harness.ctrl('u');
    harness.type_text("/status");
    harness.key(KeyCode::Tab);
    harness.key(KeyCode::Tab);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved preset 1.");
    let file = harness.config_file();
    assert!(file.contains("[tui.presets.1]"), "{file}");
    assert!(file.contains("input = \"/status\""), "{file}");

    // A digit runs it through the slash commands.
    harness.open("/presets", "1  Status");
    harness.key(KeyCode::Char('1'));
    harness.pump_until_screen_shows("Permissions:");

    // `e` edits the selected slot; the label emptied is removed.
    harness.open("/presets", "1  Status");
    harness.key(KeyCode::Char('e'));
    harness.pump_until_screen_shows("Edit preset 1");
    harness.key(KeyCode::Tab);
    harness.ctrl('e');
    harness.ctrl('u');
    harness.key(KeyCode::Tab);
    harness.key(KeyCode::Enter);
    harness.pump_until("the label removed", |harness| {
        harness.app.bridge.tui_preferences().presets["1"]
            == AgentTuiPreset {
                input: "/status".to_string(),
                label: None,
            }
    });

    // `d` asks, `y` deletes.
    harness.open("/presets", "1  /status");
    harness.key(KeyCode::Char('d'));
    harness.pump_until_screen_shows("Delete preset 1?");
    harness.key(KeyCode::Char('y'));
    harness.pump_until_screen_shows("Deleted preset 1.");
    assert!(harness.app.bridge.tui_preferences().presets.is_empty());
}

#[test]
fn a_message_preset_is_sent_and_one_starts_from_the_current_settings() {
    let mut harness = Harness::start(vec![assistant_events("Preset answered.")]);
    harness
        .app
        .bridge
        .write_tui_preferences(AgentTuiPreferencesPatch {
            presets: [(
                "3".to_string(),
                Some(AgentTuiPreset {
                    input: "Say hello".to_string(),
                    label: None,
                }),
            )]
            .into(),
            ..AgentTuiPreferencesPatch::default()
        })
        .map(|preferences| harness.app.apply_preferences(preferences))
        .unwrap();
    // The keymap's chat.open_presets: the leader, then p.
    harness.ctrl('x');
    harness.key(KeyCode::Char('p'));
    harness.pump_until_screen_shows("3  Say hello");
    harness.key(KeyCode::Char('3'));
    harness.pump_until_screen_shows("Preset answered.");
    assert_eq!(harness.server.requests().len(), 1);
    harness.pump_until_turn_ends();

    harness.open("/presets", "3  Say hello");
    harness.key(KeyCode::Char('s'));
    let choices = harness.pump_until_screen_shows("Start a preset from the current settings");
    assert!(
        choices.contains("/title activity session-title project-name"),
        "{choices}"
    );
    assert!(choices.contains("/model takes no argument"), "{choices}");
    assert!(choices.contains("/permissions manual"), "{choices}");
    // The terminal title's, under the permissions'.
    harness.key(KeyCode::Down);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("New preset");
    // Its slot is the first free one; Save.
    harness.key(KeyCode::BackTab);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved preset 1.");
    assert_eq!(
        harness.app.bridge.tui_preferences().presets["1"].input,
        "/title activity session-title project-name"
    );
}

#[test]
fn the_config_center_lists_searches_and_opens_the_quick_editor() {
    let mut harness = Harness::start(Vec::new());
    let screen = harness.open("/config", "Agent settings");
    assert!(screen.contains("General & permissions"), "{screen}");
    assert!(screen.contains("Compaction"), "{screen}");
    assert!(screen.contains("1 layers · 0 warnings"), "{screen}");

    harness.key(KeyCode::Char('/'));
    harness.type_text("compact");
    harness.pump_until("the search to filter the domains", |harness| {
        let screen = harness.screen();
        screen.contains("Compaction") && !screen.contains("General & permissions")
    });
    let screen = harness.screen();
    assert!(screen.contains("Compaction"), "{screen}");
    assert!(!screen.contains("General & permissions"), "{screen}");

    harness.key(KeyCode::Enter);
    harness.key(KeyCode::Enter);
    let screen = harness.pump_until_screen_shows("Compaction");
    assert!(screen.contains("Strategy"), "{screen}");
    harness.key(KeyCode::Right);
    for _ in 0..4 {
        harness.key(KeyCode::Tab);
    }
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved.");
    assert!(harness.config_file().contains("strategy = \"background\""));
}

#[test]
fn the_general_form_saves_the_persisted_permission_mode_and_model_discovery() {
    let mut harness = Harness::start(Vec::new());
    harness.open("/config general", "General & permissions");
    let discovery = harness.agent_settings().model_metadata.discovery;
    harness.key(KeyCode::Right);
    harness.key(KeyCode::Tab);
    harness.key(KeyCode::Right);
    harness.key(KeyCode::Tab);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved.");
    let file = harness.config_file();
    assert!(file.contains("permissionMode = \"auto_review\""), "{file}");
    let toggled = match discovery {
        AgentModelDiscovery::Auto => AgentModelDiscovery::Off,
        AgentModelDiscovery::Off => AgentModelDiscovery::Auto,
    };
    assert_eq!(harness.agent_settings().model_metadata.discovery, toggled);
    let written = match toggled {
        AgentModelDiscovery::Auto => "discovery = \"auto\"",
        AgentModelDiscovery::Off => "discovery = \"off\"",
    };
    assert!(file.contains(written), "{file}");
}

#[test]
fn the_memory_form_saves_whether_memory_is_enabled() {
    let mut harness = Harness::start(Vec::new());
    harness.open("/config memory", "Memory");
    harness.key(KeyCode::Char(' '));
    for _ in 0..5 {
        harness.key(KeyCode::Tab);
    }
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved.");
    assert!(harness.config_file().contains("[memory]"));
    assert!(harness.config_file().contains("enabled = true"));
    assert!(harness.agent_memory().enabled);
}

#[test]
fn the_web_form_saves_the_search_provider_and_endpoint_together() {
    let mut harness = Harness::start(Vec::new());
    harness.open("/config web", "Web tools");
    harness.key(KeyCode::Right);
    harness.key(KeyCode::Right);
    harness.key(KeyCode::Tab);
    harness.type_text("https://search.example.test/");
    harness.key(KeyCode::Tab);
    harness.key(KeyCode::Tab);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved.");
    let file = harness.config_file();
    assert!(file.contains("webSearchProvider = \"searxng\""), "{file}");
    assert!(
        file.contains("searxngBaseUrl = \"https://search.example.test\""),
        "{file}"
    );
}

#[test]
fn the_network_form_saves_standalone_keys_and_keeps_the_proxy_secret() {
    let mut harness = Harness::start(Vec::new());
    let screen = harness.open("/config network", "Network & logging");
    assert!(!screen.contains("user:password"), "{screen}");
    harness.key(KeyCode::Tab);
    harness.key(KeyCode::Right);
    harness.key(KeyCode::Tab);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved.");
    let file = harness.config_file();
    assert!(file.contains("logLevel = \"debug\""), "{file}");
    assert!(file.contains("proxy = \"system\""), "{file}");
}

#[test]
fn the_interface_form_saves_and_applies_its_scalar_preferences() {
    let mut harness = Harness::start(Vec::new());
    harness.open("/config interface", "Interface");
    harness.key(KeyCode::Char(' '));
    for _ in 0..10 {
        harness.key(KeyCode::Tab);
    }
    harness.key(KeyCode::Enter);
    let screen = harness.pump_until_screen_shows("Saved.");
    assert!(!screen.contains("higher layer"), "{screen}");
    assert!(harness
        .config_file()
        .contains("fullscreenTranscript = false"));
}

#[test]
fn the_configuration_center_shows_the_layers_and_opens_the_linked_editors() {
    let mut harness = Harness::start(Vec::new());
    let screen = harness.open("/config layers", "Layers & files");
    assert!(screen.contains("config.toml"), "{screen}");
    assert!(screen.contains("Warnings ("), "{screen}");
    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("Agent settings");
    harness.key(KeyCode::Esc);

    harness.open("/config interface", "Interface");
    for _ in 0..4 {
        harness.key(KeyCode::Tab);
    }
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Select Syntax Theme");
}

#[test]
fn a_scalar_preference_a_higher_layer_replaces_is_shadowed() {
    let preferences = AgentTuiPreferences::default();
    let same = AgentTuiPreferencesPatch {
        raw_output: Some(preferences.raw_output),
        ..AgentTuiPreferencesPatch::default()
    };
    assert!(!shadowed(&same, &preferences));

    let replaced = AgentTuiPreferencesPatch {
        raw_output: Some(!preferences.raw_output),
        ..AgentTuiPreferencesPatch::default()
    };
    assert!(shadowed(&replaced, &preferences));
}

#[test]
fn config_toggles_and_choices_write_the_settings_and_its_entries_open_the_editors() {
    let mut harness = Harness::start(Vec::new());
    harness.open("/config quick", "Settings");
    // Raw output, at once and saved.
    harness.key(KeyCode::Char(' '));
    harness.pump_until_screen_shows("Raw output on");
    assert_eq!(harness.app.render_mode, HistoryRenderMode::Raw);
    // The alternate screen, a choice.
    for _ in 0..3 {
        harness.key(KeyCode::Down);
    }
    harness.key(KeyCode::Right);
    harness.pump_until_screen_shows("Saved the alternate screen: always");
    let file = harness.config_file();
    assert!(file.contains("rawOutput = true"), "{file}");
    assert!(file.contains("alternateScreen = \"always\""), "{file}");
    // Fullscreen, for the next launch.
    harness.key(KeyCode::Up);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved the interface mode: Scrollback");
    assert!(harness
        .config_file()
        .contains("fullscreenTranscript = false"));
    // The compaction strategy, a choice saved into the Agent's settings.
    for _ in 0..2 {
        harness.key(KeyCode::Down);
    }
    harness.key(KeyCode::Right);
    harness.pump_until_screen_shows("Saved the compaction strategy: background");
    assert!(harness.config_file().contains("strategy = \"background\""));
    harness.key(KeyCode::Left);
    harness.pump_until_screen_shows("Saved the compaction strategy: standard");
    // Theme ›, the fourth entry.
    for _ in 0..4 {
        harness.key(KeyCode::Down);
    }
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Select Syntax Theme");
}

#[test]
fn warnings_are_merged_counted_deduplicated_and_dismissed() {
    let mut harness = Harness::start(Vec::new());
    // A key the configuration does not know, read again after a write.
    let mut file = harness.config_file();
    file.push_str("\nunknownSetting = 1\n");
    std::fs::write(harness.home.path().join("config.toml"), file).unwrap();
    harness
        .app
        .bridge
        .write_tui_preferences(AgentTuiPreferencesPatch::default())
        .unwrap();
    let config = harness.app.bridge.config_warnings();
    assert!(
        config
            .iter()
            .any(|warning| warning.text.contains("unknownSetting")),
        "{config:?}"
    );
    harness.app.chat_widget.add_startup_warnings(
        vec![
            "tui.theme: no theme `missing`".to_string(),
            "tui.theme: no theme `missing`".to_string(),
        ],
        config,
    );
    harness.pump_until_screen_shows("2 warnings: /warnings");

    let list = harness.open("/warnings", "d dismiss");
    assert!(list.contains("1 configuration · 1 interface"), "{list}");
    assert!(
        list.contains("[interface] tui.theme: no theme `missing`"),
        "{list}"
    );
    // Newest first: the configuration's, read again for the view.
    let config = list.find("unknownSetting").expect(&list);
    assert!(config < list.find("[interface]").unwrap(), "{list}");
    harness.key(KeyCode::Char('d'));
    harness.pump_until_screen_shows("1 interface");
    harness.key(KeyCode::Esc);

    let list = harness.open("/warnings", "d dismiss");
    assert!(!list.contains("unknownSetting"), "{list}");
    harness.key(KeyCode::Char('d'));
    harness.pump_until("the view to close", |harness| {
        harness.app.chat_widget.no_modal_or_popup_active()
    });
    harness.type_and_submit("/warnings");
    harness.pump_until_screen_shows("No warnings.");
}

#[test]
fn warnings_list_the_issues_of_the_roles() {
    let mut harness = Harness::start(Vec::new());
    let agents = harness.home.path().join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("orphan.toml"),
        "description = \"Lost.\"\nextends = \"nobody\"\n",
    )
    .unwrap();
    // A settings publication reads the roles again.
    let agent = harness.runtime.slot().agent().unwrap();
    harness
        .runtime
        .slot()
        .agent()
        .unwrap()
        .runtime()
        .block_on(async move { agent.reload_packages().await })
        .unwrap()
        .unwrap();

    let list = harness.open("/warnings", "d dismiss");
    // The file's path is long on some systems, so the line wraps.
    let flat = list.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("[roles] Role `orphan`:") && flat.contains("which no source defines"),
        "{list}"
    );
}

#[test]
fn memories_turns_memory_on_toggles_its_settings_and_resets_after_asking() {
    let mut harness = Harness::start(Vec::new());
    harness.open("/memories", "Turn memory on");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("[x] Use memories");
    assert!(harness.agent_memory().enabled);
    // Without a session, its contribution cannot change.
    assert!(harness.screen().contains("no session yet"));

    harness.key(KeyCode::Char(' '));
    harness.pump_until_screen_shows("[ ] Use memories");
    assert!(!harness.agent_memory().use_memories);
    harness.key(KeyCode::Down);
    harness.key(KeyCode::Char(' '));
    harness.pump_until_screen_shows("[ ] Generate memories");
    assert!(!harness.agent_memory().generate_memories);
    let file = harness.config_file();
    assert!(file.contains("useMemories = false"), "{file}");

    for _ in 0..2 {
        harness.key(KeyCode::Down);
    }
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Reset memory?");
    harness.key(KeyCode::Down);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Memory was reset");
}

#[test]
fn the_contribution_follows_the_session() {
    let mut harness = Harness::start(vec![assistant_events("Hello.")]);
    harness.type_and_submit("Hi");
    harness.pump_until_screen_shows("Hello.");
    harness.pump_until_turn_ends();
    harness.open("/memories", "Turn memory on");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("This session contributes");
    let screen = harness.screen();
    assert!(!screen.contains("no session yet"), "{screen}");
    let session = harness.app.bridge.session_id().unwrap();
    let agent = harness.runtime.slot().agent().unwrap();
    let (before, _) = agent.memory_session_contribution(session).unwrap();
    for _ in 0..2 {
        harness.key(KeyCode::Down);
    }
    harness.key(KeyCode::Char(' '));
    harness.pump_until("the contribution to change", |_| {
        agent.memory_session_contribution(session).unwrap().0 != before
    });
}

#[test]
fn a_rebound_key_takes_effect_in_owned_mode_and_the_old_one_types_nothing() {
    let mut harness = Harness::start_with(Vec::new(), true, TranscriptMode::Owned);
    harness
        .app
        .chat_widget
        .add_info_message("before".to_string());
    let preferences = harness
        .app
        .bridge
        .write_tui_preferences(AgentTuiPreferencesPatch {
            keymap: [(
                "global".to_string(),
                [("open_transcript".to_string(), Some(vec!["f6".to_string()]))].into(),
            )]
            .into(),
            ..AgentTuiPreferencesPatch::default()
        })
        .unwrap();
    assert!(harness.app.apply_preferences(preferences).is_empty());
    harness.key(KeyCode::F(6));
    assert!(harness.app.transcript_view.is_detailed());
    harness.ctrl('t');
    assert!(harness.app.transcript_view.is_detailed());
}

#[test]
fn a_save_does_not_repeat_what_the_keymap_cannot_use() {
    let mut harness = Harness::start(Vec::new());
    let mut file = harness.config_file();
    file.push_str("\n[tui.keymap.chat]\nnothing = \"ctrl-q\"\n");
    std::fs::write(harness.home.path().join("config.toml"), file).unwrap();
    harness.type_and_submit("/raw");
    harness.pump_until_screen_shows("Raw output on");
    harness.type_and_submit("/raw");
    harness.pump_until_screen_shows("Raw output off");
    let transcript = harness.transcript();
    assert!(!transcript.contains("no such action"), "{transcript}");
}
