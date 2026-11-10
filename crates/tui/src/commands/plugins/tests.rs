//! The plugin browser's pages over hand-built plugins: the tabs wrap, and
//! a plugin's page offers what its state allows.

use hanafubuki_agent::{
    AgentPackageContent, AgentPackageGuest, AgentPackageStatus, GuestState, GuestStatus,
    PackagePermissions, WorkspaceAccess,
};

use super::pages::{self, After, Choice, Page, Prompt, Tab};
use crate::agent_bridge::PluginsCall;

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
        7
    );

    let on = pages::package_page(&package(true, Vec::new()));
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
    // Its settings are a row that opens their form, starting from them.
    assert!(pages::package_page(&code)
        .choices
        .iter()
        .flatten()
        .any(|choice| *choice == Choice::Open(Page::Prompt(Prompt::Settings))));
    assert_eq!(super::code::settings_text(&code), r#"{"board":"B-1"}"#);
}
