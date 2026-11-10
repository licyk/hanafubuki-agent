// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The plugin browser's pages (Codex's `chatwidget/plugin_catalog.rs`):
//! the three tabs (the installed plugins, the marketplaces, the built-in
//! plugins), a marketplace's plugins, a plugin's details with its actions,
//! and the confirmations. Each page is a list whose rows each carry what
//! choosing it does; a row that only informs does nothing.

use hanafubuki_agent::{
    AgentMarketplace, AgentMarketplacePlugin, AgentPackageSource, AgentPackageStatus,
    AgentPluginState, AgentPluginStatus, MarketplaceSourceType, PackageSource,
};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::agent_bridge::{PluginsCall, PluginsState};
use crate::bottom_pane::{SelectionItem, SelectionViewParams};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tab {
    Installed,
    Marketplaces,
    BuiltIn,
}

impl Tab {
    const ALL: [Tab; 3] = [Tab::Installed, Tab::Marketplaces, Tab::BuiltIn];

    /// The tab `by` places to the right, wrapping around.
    pub(super) fn step(self, by: isize) -> Tab {
        let index = Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0) as isize;
        Self::ALL[(index + by).rem_euclid(Self::ALL.len() as isize) as usize]
    }
}

/// Where the browser is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Page {
    Tab(Tab),
    Package(String),
    Marketplace(String),
    /// A marketplace's plugin that is not installed.
    Offered {
        marketplace: String,
        plugin: String,
    },
    ConfirmUninstall(String),
    ConfirmRemoveMarketplace(String),
    /// After an install: its hooks, each `(key, command)`, to trust.
    TrustHooks {
        id: String,
        hooks: Vec<(String, String)>,
    },
    Prompt(Prompt),
}

/// A source to type, or a code plugin's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Prompt {
    AddMarketplace,
    InstallDirectory,
    /// The settings of the plugin whose page opened it.
    Settings,
}

/// Where the browser goes once a call's answer is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum After {
    Stay,
    /// Back to a tab, the pages opened since forgotten.
    Tab(Tab),
}

/// What choosing a row does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Choice {
    Open(Page),
    Back,
    Call {
        call: PluginsCall,
        /// Said while it runs.
        busy: String,
        after: After,
    },
}

/// A page: its list, and each row's choice.
pub(super) struct PageView {
    pub(super) params: SelectionViewParams,
    pub(super) choices: Vec<Option<Choice>>,
}

pub(super) fn call(call: PluginsCall, busy: &str, after: After) -> Choice {
    Choice::Call {
        call,
        busy: busy.to_string(),
        after,
    }
}

pub(super) fn tab_page(state: &PluginsState, tab: Tab) -> PageView {
    let (rows, hint, subtitle) = match tab {
        Tab::Installed => (
            installed_rows(state),
            "enter details · space on/off · ←/→ tabs · esc close",
            "Plugins installed from a marketplace or a directory.",
        ),
        Tab::Marketplaces => (
            marketplace_rows(state),
            "enter plugins · ctrl+u upgrade · ctrl+r remove · ←/→ tabs · esc close",
            "The marketplaces plugins are installed from.",
        ),
        Tab::BuiltIn => (
            builtin_rows(state),
            "space/enter on/off · r retry · ←/→ tabs · esc close",
            "The plugins compiled into the Agent (disabledPlugins).",
        ),
    };
    let mut header = vec![Line::from("Plugins".bold()), tabs_line(state, tab)];
    header.push(Line::from(subtitle.dim()));
    if state.safe_mode {
        header.push(Line::from(
            "Safe mode: no installed plugin is loaded, and only the core runs.".yellow(),
        ));
    }
    let (items, choices) = rows.into_iter().unzip();
    PageView {
        params: SelectionViewParams {
            header: Box::new(Paragraph::new(header).wrap(Wrap { trim: false })),
            footer_hint: Some(Line::from(hint)),
            items,
            ..Default::default()
        },
        choices,
    }
}

fn tabs_line(state: &PluginsState, active: Tab) -> Line<'static> {
    let mut spans = Vec::new();
    for tab in Tab::ALL {
        let label = match tab {
            Tab::Installed => format!("Installed ({})", state.packages.len()),
            Tab::Marketplaces => format!("Marketplaces ({})", state.marketplaces.len()),
            Tab::BuiltIn => format!("Built-in ({})", state.builtins.len()),
        };
        if !spans.is_empty() {
            spans.push(Span::from("   "));
        }
        spans.push(if tab == active {
            format!("[{label}]").cyan().bold()
        } else {
            label.dim()
        });
    }
    Line::from(spans)
}

pub(super) type Row = (SelectionItem, Option<Choice>);

fn installed_rows(state: &PluginsState) -> Vec<Row> {
    let mut rows: Vec<Row> = state
        .packages
        .iter()
        .map(|package| {
            let mut description = format!("{} · {}", package_state(package), package.version);
            if !package.hooks_to_trust.is_empty() {
                description.push_str(&format!(
                    " · {} hook(s) need review",
                    package.hooks_to_trust.len()
                ));
            }
            if !package.issues.is_empty() {
                description.push_str(&format!(" · {} issue(s)", package.issues.len()));
            }
            if let Some(text) = &package.description {
                description.push_str(&format!(" · {text}"));
            }
            let item = SelectionItem {
                name: format!("{} ({})", package.display_name, package.id),
                description: Some(description),
                ..Default::default()
            };
            (item, Some(Choice::Open(Page::Package(package.id.clone()))))
        })
        .collect();
    rows.push((
        SelectionItem {
            name: "Install from a directory…".to_string(),
            description: Some("a plugin's directory on this machine".to_string()),
            ..Default::default()
        },
        Some(Choice::Open(Page::Prompt(Prompt::InstallDirectory))),
    ));
    rows
}

fn marketplace_rows(state: &PluginsState) -> Vec<Row> {
    let mut rows: Vec<Row> = state
        .marketplaces
        .iter()
        .map(|marketplace| {
            let installed = marketplace
                .plugins
                .iter()
                .filter(|plugin| plugin.installed)
                .count();
            // The source comes last: a long path is what a narrow screen cuts.
            let mut description = format!(
                "{} · {installed} of {} installed",
                source_kind(marketplace),
                marketplace.plugins.len()
            );
            if !marketplace.enabled {
                description.push_str(" · off");
            }
            if let Some(error) = &marketplace.error {
                description.push_str(&format!(" · {error}"));
            }
            description.push_str(&format!(" · {}", marketplace.source));
            let item = SelectionItem {
                name: marketplace_name(marketplace),
                description: Some(description),
                ..Default::default()
            };
            (
                item,
                Some(Choice::Open(Page::Marketplace(marketplace.name.clone()))),
            )
        })
        .collect();
    rows.push((
        SelectionItem {
            name: "Add a marketplace…".to_string(),
            description: Some("owner/repo, a git URL, or a local marketplace path".to_string()),
            ..Default::default()
        },
        Some(Choice::Open(Page::Prompt(Prompt::AddMarketplace))),
    ));
    rows
}

fn builtin_rows(state: &PluginsState) -> Vec<Row> {
    state
        .builtins
        .iter()
        .map(|plugin| {
            let mut description = builtin_state(plugin).to_string();
            if plugin.lock.is_some() {
                description.push_str(" · always on");
            }
            if let Some(error) = plugin.error.as_ref().or(plugin.warning.as_ref()) {
                description.push_str(&format!(" · {error}"));
            }
            let item = SelectionItem {
                name: plugin.name.clone(),
                description: Some(description),
                ..Default::default()
            };
            (item, builtin_toggle(plugin))
        })
        .collect()
}

/// Turning a built-in plugin on or off; a core one cannot be.
pub(super) fn builtin_toggle(plugin: &AgentPluginStatus) -> Option<Choice> {
    if plugin.lock.is_some() {
        return None;
    }
    let enabled = !plugin.enabled;
    Some(call(
        PluginsCall::SetBuiltinEnabled {
            name: plugin.name.clone(),
            enabled,
        },
        if enabled {
            "Turning it on…"
        } else {
            "Turning it off…"
        },
        After::Stay,
    ))
}

/// An installed plugin: what can be done with it, then what it is.
pub(super) fn package_page(package: &AgentPackageStatus) -> PageView {
    let id = &package.id;
    let header = vec![
        Line::from("Plugins".bold()),
        Line::from(
            format!(
                "{} · {} · {}",
                package.display_name,
                package_state(package),
                id
            )
            .bold(),
        ),
        Line::from(package.description.clone().unwrap_or_default().dim()),
    ];
    let toggle = if package.enabled {
        ("Disable plugin", "Turn it off; it stays installed")
    } else {
        ("Enable plugin", "Turn it on")
    };
    let mut rows: Vec<Row> = vec![
        action("Back to plugins", "Return to the list", Some(Choice::Back)),
        action(
            toggle.0,
            toggle.1,
            Some(call(
                PluginsCall::SetEnabled {
                    id: id.clone(),
                    enabled: !package.enabled,
                },
                "Saving…",
                After::Stay,
            )),
        ),
    ];
    if !package.hooks_to_trust.is_empty() {
        rows.push(action(
            &format!("Trust its {} hook(s)", package.hooks_to_trust.len()),
            "Each runs its command on its event from now on (/hooks shows them)",
            Some(call(
                PluginsCall::TrustHooks(package.hooks_to_trust.clone()),
                "Trusting…",
                After::Stay,
            )),
        ));
    }
    rows.extend(super::code::grant_row(package));
    rows.extend([
        action(
            "Update plugin",
            "Install it again from its marketplace or directory",
            Some(call(
                PluginsCall::Update(id.clone()),
                "Updating…",
                After::Stay,
            )),
        ),
        action(
            "Retry plugin",
            "Load it again",
            Some(call(
                PluginsCall::Retry(id.clone()),
                "Loading…",
                After::Stay,
            )),
        ),
        action(
            "Uninstall plugin",
            "Remove it, keeping its data or not",
            Some(Choice::Open(Page::ConfirmUninstall(id.clone()))),
        ),
    ]);
    let list = |items: &[String], none: &str| match items {
        [] => none.to_string(),
        items => items.join(", "),
    };
    rows.extend(super::code::state_row(package));
    rows.extend(super::code::settings_row(package));
    rows.extend([
        info("Source", &source_text(package.source.as_ref())),
        info("Version", &package.version),
        info(
            "Format",
            &package
                .format
                .map_or("unreadable".to_string(), |format| format!("{format:?}")),
        ),
        info("Skills", &list(&package.content.skills, "No plugin Skills")),
        info(
            "MCP servers",
            &list(&package.content.mcp_servers, "No plugin MCP servers"),
        ),
        info("Hooks", &hooks_text(package)),
    ]);
    rows.extend(package.issues.iter().map(|issue| info("Issue", issue)));
    page(header, "enter select · esc back", rows)
}

/// A marketplace's plugins, each opening its details.
pub(super) fn marketplace_page(marketplace: &AgentMarketplace) -> PageView {
    let mut header = vec![
        Line::from("Plugins".bold()),
        Line::from(format!("{} marketplace", marketplace_name(marketplace)).bold()),
        Line::from(format!("{} · {}", source_kind(marketplace), marketplace.source).dim()),
    ];
    if let Some(error) = &marketplace.error {
        header.push(Line::from(error.clone().red()));
    }
    header.extend(
        marketplace
            .issues
            .iter()
            .map(|issue| Line::from(format!("  · {issue}").dim())),
    );
    let mut rows: Vec<Row> = marketplace
        .plugins
        .iter()
        .map(|plugin| {
            let mut description = offered_state(plugin).to_string();
            if let Some(text) = &plugin.description {
                description.push_str(&format!(" · {text}"));
            }
            let item = SelectionItem {
                name: plugin.display_name.clone(),
                description: Some(description),
                ..Default::default()
            };
            let choice = if plugin.installed {
                Page::Package(plugin.id.clone())
            } else {
                Page::Offered {
                    marketplace: marketplace.name.clone(),
                    plugin: plugin.name.clone(),
                }
            };
            (item, Some(Choice::Open(choice)))
        })
        .collect();
    if rows.is_empty() {
        rows.push(info("No plugins", "This marketplace offers none"));
    }
    page(header, "enter details · esc back", rows)
}

/// A marketplace's plugin that is not installed, and its install.
pub(super) fn offered_page(
    marketplace: &AgentMarketplace,
    plugin: &AgentMarketplacePlugin,
) -> PageView {
    let header = vec![
        Line::from("Plugins".bold()),
        Line::from(
            format!(
                "{} · {} · {}",
                plugin.display_name,
                offered_state(plugin),
                marketplace_name(marketplace)
            )
            .bold(),
        ),
        Line::from(plugin.description.clone().unwrap_or_default().dim()),
    ];
    let install = plugin.installable.then(|| {
        call(
            PluginsCall::Install(AgentPackageSource::Marketplace {
                marketplace: marketplace.name.clone(),
                plugin: plugin.name.clone(),
            }),
            "Installing…",
            After::Stay,
        )
    });
    let rows = vec![
        action(
            "Back to plugins",
            "Return to the marketplace",
            Some(Choice::Back),
        ),
        match install {
            Some(install) => action("Install plugin", "Install it and turn it on", Some(install)),
            None => info(
                "Install plugin",
                "This marketplace does not let it be installed",
            ),
        },
        info("Plugin", &plugin.id),
        info("Version", plugin.version.as_deref().unwrap_or("not given")),
    ];
    page(header, "enter select · esc back", rows)
}

pub(super) fn confirm_uninstall(id: &str) -> PageView {
    let header = vec![
        Line::from("Plugins".bold()),
        Line::from(format!("Uninstall {id}?").bold()),
        Line::from("Its Skills, MCP servers and hooks go at once.".dim()),
    ];
    let uninstall = |purge_data: bool, busy: &str| {
        Some(call(
            PluginsCall::Uninstall {
                id: id.to_string(),
                purge_data,
            },
            busy,
            After::Tab(Tab::Installed),
        ))
    };
    let rows = vec![
        action("Cancel", "Keep it installed", Some(Choice::Back)),
        action(
            "Uninstall, keep its data",
            "Its data directory stays for a later install",
            uninstall(false, "Uninstalling…"),
        ),
        action(
            "Uninstall and delete its data",
            "Its data directory is deleted too",
            uninstall(true, "Uninstalling…"),
        ),
    ];
    page(header, "enter select · esc back", rows)
}

pub(super) fn confirm_remove_marketplace(name: &str) -> PageView {
    let header = vec![
        Line::from("Plugins".bold()),
        Line::from(format!("Remove the marketplace {name}?").bold()),
        Line::from("The plugins installed from it stay installed.".dim()),
    ];
    let rows = vec![
        action("Cancel", "Keep it", Some(Choice::Back)),
        action(
            "Remove marketplace",
            "Remove it from the available plugins",
            Some(call(
                PluginsCall::RemoveMarketplace(name.to_string()),
                "Removing…",
                After::Tab(Tab::Marketplaces),
            )),
        ),
    ];
    page(header, "enter select · esc back", rows)
}

/// After an install: its hooks, which run only once trusted.
pub(super) fn trust_hooks_page(id: &str, hooks: &[(String, String)]) -> PageView {
    let mut header = vec![
        Line::from("Plugins".bold()),
        Line::from(format!("Installed {id}.").bold()),
        Line::from(
            format!(
                "Its {} hook(s) run their commands only once trusted:",
                hooks.len()
            )
            .dim(),
        ),
    ];
    header.extend(
        hooks
            .iter()
            .map(|(_, command)| Line::from(format!("  · {command}"))),
    );
    let rows = vec![
        action(
            "Trust them",
            "Each runs its command on its event from now on",
            Some(call(
                PluginsCall::TrustHooks(hooks.iter().map(|(key, _)| key.clone()).collect()),
                "Trusting…",
                After::Stay,
            )),
        ),
        action(
            "Not now",
            "Review them later in /hooks or here",
            Some(Choice::Open(Page::Package(id.to_string()))),
        ),
    ];
    page(header, "enter select · esc back", rows)
}

fn page(header: Vec<Line<'static>>, hint: &str, rows: Vec<Row>) -> PageView {
    let (items, choices) = rows.into_iter().unzip();
    PageView {
        params: SelectionViewParams {
            header: Box::new(Paragraph::new(header).wrap(Wrap { trim: false })),
            footer_hint: Some(Line::from(hint.to_string())),
            items,
            ..Default::default()
        },
        choices,
    }
}

pub(super) fn action(name: &str, description: &str, choice: Option<Choice>) -> Row {
    let item = SelectionItem {
        name: name.to_string(),
        description: Some(description.to_string()),
        ..Default::default()
    };
    (item, choice)
}

pub(super) fn info(name: &str, value: &str) -> Row {
    let item = SelectionItem {
        name: name.to_string(),
        description: Some(value.to_string()),
        is_disabled: true,
        ..Default::default()
    };
    (item, None)
}

pub(super) fn package_state(package: &AgentPackageStatus) -> &'static str {
    if package.enabled {
        "Installed"
    } else {
        "Disabled"
    }
}

fn offered_state(plugin: &AgentMarketplacePlugin) -> &'static str {
    match (plugin.installed, plugin.enabled) {
        (true, true) => "Installed",
        (true, false) => "Disabled",
        _ if !plugin.installable => "Not installable",
        _ => "Available",
    }
}

fn builtin_state(plugin: &AgentPluginStatus) -> &'static str {
    match plugin.state {
        AgentPluginState::Pending => "starting",
        AgentPluginState::Active => "on",
        AgentPluginState::Failed => "failed",
        AgentPluginState::Disabled => "off",
        AgentPluginState::Disposed => "stopped",
    }
}

fn hooks_text(package: &AgentPackageStatus) -> String {
    let count = package.content.hooks.len();
    match (count, package.hooks_to_trust.len()) {
        (0, _) => "No plugin hooks".to_string(),
        (count, 0) => format!("{count}, trusted"),
        (count, waiting) => format!("{count}, {waiting} need review"),
    }
}

fn marketplace_name(marketplace: &AgentMarketplace) -> String {
    marketplace
        .display_name
        .clone()
        .unwrap_or_else(|| marketplace.name.clone())
}

fn source_kind(marketplace: &AgentMarketplace) -> &'static str {
    match marketplace.source_type {
        MarketplaceSourceType::Git => "git",
        MarketplaceSourceType::Local => "local",
    }
}

fn source_text(source: Option<&PackageSource>) -> String {
    match source {
        None => "unknown".to_string(),
        Some(PackageSource::Local { path }) => format!("Local · {}", path.display()),
        Some(PackageSource::Git { url, ref_name, .. }) => match ref_name {
            Some(ref_name) => format!("Git · {url}@{ref_name}"),
            None => format!("Git · {url}"),
        },
    }
}
