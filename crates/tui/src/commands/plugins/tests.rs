//! The plugin browser's pages over hand-built plugins: the tabs wrap, are
//! shown as the settings center's tabs are and can be clicked, and a
//! plugin's page offers what its state allows.

use std::sync::mpsc::{channel, Receiver};

use hanafubuki_agent::{
    AgentPackageContent, AgentPackageGuest, AgentPackageStatus, GuestState, GuestStatus,
    PackagePermissions, RuntimeKind, WorkspaceAccess,
};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use super::browser::PluginsBrowser;
use super::options;
use super::pages::{self, After, Choice, Page, Prompt, Tab};
use crate::agent_bridge::{PluginsCall, PluginsReply, PluginsState};
use crate::app_event::{AppEvent, AppEventSender};
use crate::bottom_pane::BottomPaneView;
use crate::commands::CommandReply;
use crate::render::renderable::Renderable;

fn package(enabled: bool, hooks_to_trust: Vec<String>) -> AgentPackageStatus {
    AgentPackageStatus {
        id: "demo@market".to_string(),
        version: "1.0.0".to_string(),
        format: None,
        display_name: "Demo".to_string(),
        description: None,
        source: None,
        enabled,
        content: AgentPackageContent::default(),
        hooks_to_trust,
        guest: None,
        issues: vec!["its manifest has no name".to_string()],
    }
}

#[test]
fn the_tabs_wrap_around() {
    assert_eq!(Tab::Installed.step(-1), Tab::BuiltIn);
    assert_eq!(Tab::BuiltIn.step(1), Tab::Installed);
    assert_eq!(Tab::Installed.step(1), Tab::Marketplaces);
}

/// The browser showing `state`'s first tab, and the events it may send.
fn browser(state: PluginsState) -> (PluginsBrowser, Receiver<AppEvent>) {
    let (sender, events) = channel();
    let mut browser = PluginsBrowser::new(AppEventSender::new(sender));
    let left = browser.take_command_reply(CommandReply::Plugins(PluginsReply::Loaded {
        state: Box::new(state),
        done: None,
    }));
    assert!(left.is_none(), "the browser reads its own state");
    (browser, events)
}

/// The browser drawn at `width`, its height what it asks for.
fn drawn(browser: &PluginsBrowser, width: u16) -> (Buffer, Rect) {
    let area = Rect::new(0, 0, width, browser.desired_height(width));
    let mut buf = Buffer::empty(area);
    browser.render(area, &mut buf);
    (buf, area)
}

/// Where `text` starts in the drawn buffer, row by row.
fn at(buf: &Buffer, area: Rect, text: &str) -> (u16, u16) {
    let wanted: Vec<char> = text.chars().collect();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let found = wanted.iter().enumerate().all(|(offset, character)| {
                let column = x + offset as u16;
                column < area.right() && buf[(column, y)].symbol() == character.to_string()
            });
            if found {
                return (x, y);
            }
        }
    }
    panic!("no `{text}` drawn:\n{}", drawn_text(buf, area));
}

fn drawn_text(buf: &Buffer, area: Rect) -> String {
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn click(browser: &mut PluginsBrowser, column: u16, row: u16) -> Option<KeyCode> {
    let key = browser.pointer_key(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
    });
    // The chat hands what a view takes from the pointer back to it as a key.
    if let Some(key) = key {
        browser.handle_key_event(ratatui::crossterm::event::KeyEvent::new(
            key,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ));
    }
    key
}

/// The tab line is the settings center's: the tab shown in reverse, the
/// others dim, and each one a click away.
#[test]
fn a_tab_is_shown_the_settings_centers_way_and_a_click_shows_it() {
    let (mut browser, _events) = browser(PluginsState::default());
    let (buf, area) = drawn(&browser, 80);
    let (installed, row) = at(&buf, area, "Installed (0)");
    assert_eq!(
        buf[(installed, row)].style().add_modifier,
        Modifier::BOLD | Modifier::REVERSED,
        "the tab shown is in reverse"
    );
    let (marketplaces, _) = at(&buf, area, "Marketplaces (0)");
    assert_eq!(
        buf[(marketplaces, row)].style().add_modifier,
        Modifier::DIM,
        "the others are dim"
    );

    // A click on the second tab shows it, over the third's count.
    assert_eq!(click(&mut browser, marketplaces, row), Some(KeyCode::Null));
    let (buf, area) = drawn(&browser, 80);
    let (marketplaces, row) = at(&buf, area, "Marketplaces (0)");
    assert_eq!(
        buf[(marketplaces, row)].style().add_modifier,
        Modifier::BOLD | Modifier::REVERSED,
        "the tab clicked is the one shown:\n{}",
        drawn_text(&buf, area)
    );
    assert!(
        drawn_text(&buf, area).contains("Add a marketplace…"),
        "its rows:\n{}",
        drawn_text(&buf, area)
    );

    // A click elsewhere in the line, and a key, are the list's.
    assert_eq!(click(&mut browser, 79, 0), None);
}

/// The tab shown stays in the line when they do not all fit.
#[test]
fn the_tab_shown_keeps_its_room_in_a_narrow_window() {
    let (mut browser, _events) = browser(PluginsState::default());
    let (buf, area) = drawn(&browser, 80);
    let (built_in, row) = at(&buf, area, "Built-in (0)");
    assert_eq!(click(&mut browser, built_in, row), Some(KeyCode::Null));
    let (buf, area) = drawn(&browser, 26);
    let text = drawn_text(&buf, area);
    assert!(text.contains("Built-in (0)"), "{text}");
    assert!(
        !text.contains("Installed (0)"),
        "the left one gives way:\n{text}"
    );
}

/// Away from a tab page the tab line is gone, and so are its cells: a
/// click there belongs to the page.
#[test]
fn a_page_without_the_tab_line_takes_no_click_for_a_tab() {
    let state = PluginsState {
        packages: vec![package(true, Vec::new())],
        ..Default::default()
    };
    let (mut browser, _events) = browser(state);
    let (buf, area) = drawn(&browser, 80);
    let (_, row) = at(&buf, area, "Demo (demo@market)");
    click(&mut browser, 2, row);
    let (buf, area) = drawn(&browser, 80);
    let text = drawn_text(&buf, area);
    assert!(text.contains("demo@market"), "its page opened:\n{text}");
    assert!(
        !text.contains("Marketplaces (0)"),
        "no tab line here:\n{text}"
    );
    // The cells the tab line had are the page's now.
    assert_eq!(click(&mut browser, 2, 1), None);
}

#[test]
fn a_plugins_page_offers_what_its_state_allows() {
    let off = pages::package_page(&package(false, vec!["hook".to_string()]));
    let choices: Vec<&Choice> = off.choices.iter().flatten().collect();
    assert_eq!(*choices[0], Choice::Back);
    assert!(matches!(
        choices[1],
        Choice::Call {
            call: PluginsCall::SetEnabled { enabled: true, .. },
            after: After::Stay,
            ..
        }
    ));
    assert!(matches!(
        choices[2],
        Choice::Call { call: PluginsCall::TrustHooks(keys), .. } if keys == &["hook"]
    ));
    assert_eq!(
        *choices.last().unwrap(),
        &Choice::Open(Page::ConfirmUninstall("demo@market".to_string()))
    );
    // The informing rows (source, version, format, content, the issue)
    // choose nothing.
    assert_eq!(
        off.choices.iter().filter(|choice| choice.is_none()).count(),
        8
    );

    let mut on = package(true, Vec::new());
    on.content.roles = vec!["demo:reviewer".to_string(), "demo:planner".to_string()];
    let on = pages::package_page(&on);
    let roles = on
        .params
        .items
        .iter()
        .find(|item| item.name == "Roles")
        .and_then(|item| item.description.as_deref());
    assert_eq!(roles, Some("demo:reviewer, demo:planner"));
    assert!(on.choices.iter().flatten().any(|choice| matches!(
        choice,
        Choice::Call {
            call: PluginsCall::SetEnabled { enabled: false, .. },
            ..
        }
    )));
    assert!(!on.choices.iter().flatten().any(|choice| matches!(
        choice,
        Choice::Call {
            call: PluginsCall::TrustHooks(_),
            ..
        }
    )));
}

#[test]
fn a_code_plugins_page_offers_to_grant_what_it_asks_for_until_granted() {
    let asked = PackagePermissions {
        network: true,
        workspace: WorkspaceAccess::Read,
        env: vec!["TOKEN".to_string()],
    };
    let mut code = package(true, Vec::new());
    code.guest = Some(AgentPackageGuest {
        runtime: RuntimeKind::Process,
        permissions: asked.clone(),
        granted_permissions: None,
        status: GuestStatus {
            state: GuestState::Blocked,
            error: Some("its permissions are not granted".to_string()),
            last_exit: None,
            stderr_tail: String::new(),
            restarts: 0,
            process_id: None,
            issues: Vec::new(),
        },
        settings: serde_json::Map::from_iter([("board".to_string(), serde_json::json!("B-1"))]),
        config_schema: None,
        options: None,
    });
    let grants = |page: &pages::PageView| {
        page.choices.iter().flatten().any(|choice| {
            matches!(
                choice,
                Choice::Call { call: PluginsCall::GrantPermissions { id, permissions }, .. }
                    if id == "demo@market" && *permissions == asked
            )
        })
    };
    assert!(grants(&pages::package_page(&code)));
    code.guest.as_mut().unwrap().granted_permissions = Some(asked.clone());
    assert!(!grants(&pages::package_page(&code)));
    // A WASM guest needs no grant.
    let guest = code.guest.as_mut().unwrap();
    (guest.runtime, guest.granted_permissions) = (RuntimeKind::Wasm, None);
    assert!(!grants(&pages::package_page(&code)));
    // Its settings are a row that opens their form, starting from them.
    assert!(pages::package_page(&code)
        .choices
        .iter()
        .flatten()
        .any(|choice| *choice == Choice::Open(Page::Prompt(Prompt::Settings))));
    assert_eq!(super::code::settings_text(&code), r#"{"board":"B-1"}"#);
}

/// `o` opens a code plugin's options when its `configSchema` lowers to
/// some; saving writes what changed by key, so the keys the options do not
/// describe stay as they are.
#[test]
fn a_code_plugins_options_save_what_changed_by_key() {
    use hanafubuki_agent::{
        AgentPluginOptionsStatus, PluginOptionSchema, ResolvedPluginOption, SettingKind,
        SettingSpec, SettingValue,
    };
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut code = package(true, Vec::new());
    assert_eq!(super::options::of_package(&code), None);
    code.guest = Some(AgentPackageGuest {
        runtime: RuntimeKind::Wasm,
        permissions: PackagePermissions::default(),
        granted_permissions: None,
        status: GuestStatus {
            state: GuestState::Running,
            error: None,
            last_exit: None,
            stderr_tail: String::new(),
            restarts: 0,
            process_id: None,
            issues: Vec::new(),
        },
        settings: serde_json::Map::from_iter([("nested".to_string(), serde_json::json!({}))]),
        config_schema: None,
        options: Some(AgentPluginOptionsStatus {
            schema: PluginOptionSchema {
                options: vec![SettingSpec {
                    summary: "How loud".to_string(),
                    default: Some(SettingValue::Integer(2)),
                    ..SettingSpec::new(
                        "level",
                        "Level",
                        SettingKind::Integer {
                            minimum: Some(1),
                            maximum: Some(5),
                        },
                    )
                }],
            },
            values: vec![ResolvedPluginOption {
                key: "level".to_string(),
                value: Some(SettingValue::Integer(2)),
                layer: None,
            }],
            issues: Vec::new(),
        }),
    });
    let target = super::options::of_package(&code).unwrap();
    let state = crate::agent_bridge::PluginsState {
        packages: vec![code],
        ..Default::default()
    };
    let mut form = super::options::form(&state, &target).unwrap();
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    form.handle_key(key(KeyCode::Backspace));
    form.handle_key(key(KeyCode::Char('4')));
    assert_eq!(
        super::options::save(&mut form, &state, &target),
        Some(PluginsCall::SetPackageOptions {
            id: "demo@market".to_string(),
            options: serde_json::Map::from_iter([("level".to_string(), serde_json::json!(4))]),
        })
    );
}

/// The generic editor covers every option of every descriptor: each has
/// its field, an untouched form saves nothing, and an edited field saves
/// what the write checks.
#[test]
fn every_option_of_every_descriptor_has_the_generic_editor() {
    use hanafubuki_agent::{agent_plugin_descriptors, ResolvedPluginOption};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut described = 0;
    for descriptor in agent_plugin_descriptors() {
        let Some(schema) = descriptor.options else {
            continue;
        };
        let values = schema
            .options
            .iter()
            .map(|option| ResolvedPluginOption {
                key: option.key.clone(),
                value: option.default.clone(),
                layer: None,
            })
            .collect::<Vec<_>>();
        let mut form = options::options_form(descriptor.name, &schema, &values);
        for index in 0..schema.options.len() {
            let id = options::option_field(index);
            form.set_error(&id, "probe");
            assert_eq!(
                form.error(&id),
                Some("probe"),
                "{}",
                schema.options[index].key
            );
            described += 1;
        }
        assert_eq!(
            options::changed_options(&mut form, &schema, &values),
            Some(serde_json::Map::new())
        );
        if descriptor.name == "web" {
            let index = schema
                .options
                .iter()
                .position(|option| option.key == "browserFetch")
                .unwrap();
            assert!(index >= 64, "exercise an option beyond the old form limit");
            form.set_error(&options::option_field(index), "focus");
            form.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
            assert_eq!(
                options::changed_options(&mut form, &schema, &values),
                Some(serde_json::Map::from_iter([(
                    "browserFetch".to_string(),
                    serde_json::json!(false),
                )]))
            );
        }
    }
    assert!(described > 64, "includes the full web plugin options");

    let schema = agent_plugin_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.name == "subagents")
        .and_then(|descriptor| descriptor.options)
        .unwrap();
    let values = schema
        .options
        .iter()
        .map(|option| ResolvedPluginOption {
            key: option.key.clone(),
            value: option.default.clone(),
            layer: None,
        })
        .collect::<Vec<_>>();
    let mut form = options::options_form("subagents", &schema, &values);
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    form.handle_key(key(KeyCode::Backspace));
    form.handle_key(key(KeyCode::Char('5')));
    assert_eq!(
        options::changed_options(&mut form, &schema, &values),
        Some(serde_json::Map::from_iter([(
            "maximumRunningAgents".to_string(),
            serde_json::json!(5)
        )]))
    );
    form.handle_key(key(KeyCode::Char('0')));
    assert_eq!(options::changed_options(&mut form, &schema, &values), None);
    assert_eq!(
        form.error(&options::option_field(0)),
        Some("`maximumRunningAgents` is between 1 and 16")
    );
}
