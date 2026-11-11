// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/config`'s full-screen center: every configuration domain in one
//! searchable list, with the effective value that domain holds and the
//! layers around it. Opening a domain is delegated to the command that owns
//! it until that domain gains its own form.

mod forms;
mod layers;
mod render;
#[cfg(test)]
mod tests;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::Stylize;

use crate::agent_bridge::ConfigState;
use crate::bottom_pane::{FormEvent, FormView};
pub(crate) use forms::ConfigChange;

/// A configuration domain the center lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfigSection {
    General,
    Sandbox,
    Memory,
    Web,
    Compaction,
    Network,
    Interface,
    Providers,
    Mcp,
    Plugins,
    Hooks,
    Layers,
}

impl ConfigSection {
    pub(crate) const ALL: [Self; 12] = [
        Self::General,
        Self::Sandbox,
        Self::Memory,
        Self::Web,
        Self::Compaction,
        Self::Network,
        Self::Interface,
        Self::Providers,
        Self::Mcp,
        Self::Plugins,
        Self::Hooks,
        Self::Layers,
    ];

    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().replace('_', "-").as_str() {
            "" | "general" | "permissions" => Some(Self::General),
            "sandbox" => Some(Self::Sandbox),
            "memory" => Some(Self::Memory),
            "web" => Some(Self::Web),
            "compaction" => Some(Self::Compaction),
            "network" => Some(Self::Network),
            "interface" => Some(Self::Interface),
            "providers" => Some(Self::Providers),
            "mcp" => Some(Self::Mcp),
            "plugins" => Some(Self::Plugins),
            "hooks" => Some(Self::Hooks),
            "layers" => Some(Self::Layers),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Sandbox => "sandbox",
            Self::Memory => "memory",
            Self::Web => "web",
            Self::Compaction => "compaction",
            Self::Network => "network",
            Self::Interface => "interface",
            Self::Providers => "providers",
            Self::Mcp => "mcp",
            Self::Plugins => "plugins",
            Self::Hooks => "hooks",
            Self::Layers => "layers",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::General => "General & permissions",
            Self::Sandbox => "Sandbox",
            Self::Memory => "Memory",
            Self::Web => "Web tools",
            Self::Compaction => "Compaction",
            Self::Network => "Network & logging",
            Self::Interface => "Interface",
            Self::Providers => "Providers & model policy",
            Self::Mcp => "MCP servers",
            Self::Plugins => "Plugins & packages",
            Self::Hooks => "Hooks",
            Self::Layers => "Layers & files",
        }
    }

    /// The command that owns the domain's editor today. Domains without a
    /// form yet keep their row read-only in the center.
    fn command(self) -> Option<&'static str> {
        match self {
            Self::General | Self::Sandbox => Some("/permissions"),
            Self::Memory => Some("/memories"),
            Self::Compaction | Self::Interface => Some("/config quick"),
            Self::Providers => Some("/providers policy"),
            Self::Mcp => Some("/mcp"),
            Self::Plugins => Some("/plugins"),
            Self::Hooks => Some("/hooks"),
            Self::Web | Self::Network | Self::Layers => None,
        }
    }

    fn summary(self, state: &ConfigState) -> String {
        let settings = &state.agent.settings;
        match self {
            Self::General => permission_name(settings.permission_mode).to_string(),
            Self::Sandbox => settings
                .sandbox_mode
                .map_or("platform default".to_string(), sandbox_name),
            Self::Memory => if settings.memory.enabled { "on" } else { "off" }.to_string(),
            Self::Web => settings.web_search_provider.as_str().to_string(),
            Self::Compaction => settings.compaction.strategy.as_str().to_string(),
            Self::Network => format!(
                "{} · {}",
                proxy_name(&state.standalone.network.proxy),
                log_level_name(state.standalone.log_level)
            ),
            Self::Interface => format!(
                "{} · {}",
                if state.standalone.tui.fullscreen_transcript {
                    "fullscreen"
                } else {
                    "scrollback"
                },
                if state.standalone.tui.raw_output {
                    "raw"
                } else {
                    "rich"
                }
            ),
            Self::Providers => "model Providers and policy".to_string(),
            Self::Mcp => "servers and credentials".to_string(),
            Self::Plugins => format!("{} disabled", settings.disabled_plugins.len()),
            Self::Hooks => "trust and enablement".to_string(),
            Self::Layers => format!(
                "{} layers · {} warnings",
                state.layers.len(),
                state.warnings.len()
            ),
        }
    }
}

/// What the app does with a key in the center.
#[derive(Debug)]
pub(crate) enum ConfigAction {
    Close,
    Replay(String),
    Save(Box<ConfigChange>),
}

pub(crate) struct ConfigCenter {
    state: Result<ConfigState, String>,
    query: String,
    search_focused: bool,
    selected: usize,
    scroll_top: usize,
    view_rows: Option<usize>,
    form: Option<(ConfigSection, FormView)>,
    layers_open: bool,
}

impl ConfigCenter {
    pub(crate) fn new(state: Result<ConfigState, String>) -> Self {
        Self {
            state,
            query: String::new(),
            search_focused: false,
            selected: 0,
            scroll_top: 0,
            view_rows: None,
            form: None,
            layers_open: false,
        }
    }

    pub(crate) fn open(&mut self, section: ConfigSection, direct: bool) -> Option<ConfigAction> {
        self.select(section);
        if direct {
            match section {
                section if section.editable() => self.open_form(section),
                ConfigSection::Layers => self.open_layers(),
                _ => {
                    return section
                        .command()
                        .map(|command| ConfigAction::Replay(command.to_string()));
                }
            }
        }
        None
    }

    fn select(&mut self, section: ConfigSection) {
        self.selected = ConfigSection::ALL
            .into_iter()
            .position(|candidate| candidate == section)
            .unwrap_or_default();
        self.scroll_top = 0;
    }

    fn open_form(&mut self, section: ConfigSection) {
        if let Ok(state) = &self.state {
            self.form = Some((section, forms::section_form(section, state)));
        }
    }

    pub(crate) fn save_finished(&mut self, saved: Result<ConfigState, String>, shadowed: bool) {
        let Some((_, form)) = self.form.as_mut() else {
            return;
        };
        match saved {
            Ok(state) => {
                self.state = Ok(state);
                let message = if shadowed {
                    "Saved to the user's configuration; a higher layer still applies."
                } else {
                    "Saved."
                };
                form.set_status(vec![ratatui::text::Line::from(message.green())]);
            }
            Err(error) => {
                form.set_status(vec![ratatui::text::Line::from(format!("✗ {error}").red())])
            }
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Option<ConfigAction> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            return Some(ConfigAction::Close);
        }
        if self.layers_open {
            return match key.code {
                KeyCode::Esc => {
                    self.layers_open = false;
                    None
                }
                KeyCode::Char('q') => Some(ConfigAction::Close),
                _ => None,
            };
        }
        if let Some((section, form)) = self.form.as_mut() {
            return match form.handle_key(key) {
                FormEvent::Cancel => {
                    self.form = None;
                    None
                }
                FormEvent::Action(action) => {
                    if let Some(command) = forms::action_command(action) {
                        return Some(ConfigAction::Replay(command.to_string()));
                    }
                    let current = self.state.as_ref().ok()?;
                    forms::draft(*section, form, current)
                        .map(|change| ConfigAction::Save(Box::new(change)))
                }
                FormEvent::None => None,
            };
        }
        if self.search_focused {
            match key.code {
                KeyCode::Esc => {
                    if self.query.is_empty() {
                        self.search_focused = false;
                    } else {
                        self.query.clear();
                        self.reset_selection();
                    }
                }
                KeyCode::Enter => self.search_focused = false,
                KeyCode::Backspace => {
                    self.query.pop();
                    self.reset_selection();
                }
                KeyCode::Char(character)
                    if key.modifiers == KeyModifiers::NONE
                        || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.query.push(character);
                    self.reset_selection();
                }
                _ => {}
            }
            return None;
        }

        let rows = self.rows();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Some(ConfigAction::Close),
            KeyCode::Char('/') => {
                self.search_focused = true;
                self.query.clear();
                self.reset_selection();
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_by(&rows, -1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(&rows, 1),
            KeyCode::PageUp => self.move_by(&rows, -self.page_step()),
            KeyCode::PageDown => self.move_by(&rows, self.page_step()),
            KeyCode::Home => self.move_by(&rows, isize::MIN),
            KeyCode::End => self.move_by(&rows, isize::MAX),
            KeyCode::Enter => {
                let section = rows.get(self.selected).copied()?;
                if section.editable() {
                    if self.state.is_ok() {
                        self.open_form(section);
                    }
                    return None;
                }
                if section == ConfigSection::Layers {
                    if self.state.is_ok() {
                        self.open_layers();
                    }
                    return None;
                }
                let command = section.command()?;
                return Some(ConfigAction::Replay(command.to_string()));
            }
            _ => {}
        }
        None
    }

    fn rows(&self) -> Vec<ConfigSection> {
        if let Err(error) = &self.state {
            let _ = error;
            return Vec::new();
        }
        let needle = self.query.to_lowercase();
        ConfigSection::ALL
            .into_iter()
            .filter(|section| {
                needle.is_empty()
                    || section.label().to_lowercase().contains(&needle)
                    || section.key().contains(&needle)
                    || section
                        .summary(self.state.as_ref().expect("checked above"))
                        .to_lowercase()
                        .contains(&needle)
            })
            .collect()
    }

    fn reset_selection(&mut self) {
        self.selected = 0;
        self.scroll_top = 0;
    }

    fn move_by(&mut self, rows: &[ConfigSection], delta: isize) {
        if rows.is_empty() {
            return;
        }
        let last = rows.len() - 1;
        self.selected = match delta {
            isize::MIN => 0,
            isize::MAX => last,
            delta => self.selected.saturating_add_signed(delta).min(last),
        };
        self.ensure_selected_visible(rows.len());
    }

    fn ensure_selected_visible(&mut self, count: usize) {
        let capacity = self.view_rows.unwrap_or(count).max(1);
        if self.selected < self.scroll_top {
            self.scroll_top = self.selected;
        } else if self.selected >= self.scroll_top + capacity {
            self.scroll_top = self.selected + 1 - capacity;
        }
        self.scroll_top = self.scroll_top.min(count.saturating_sub(capacity));
    }

    fn page_step(&self) -> isize {
        isize::try_from(self.view_rows.unwrap_or(10).max(1)).unwrap_or(10)
    }

    fn state_summary(&self) -> Result<String, String> {
        self.state
            .as_ref()
            .map(|state| {
                format!(
                    "{} layers · {} warnings",
                    state.layers.len(),
                    state.warnings.len()
                )
            })
            .map_err(|error| error.clone())
    }

    fn section_summary(&self, section: ConfigSection) -> String {
        self.state
            .as_ref()
            .map(|state| section.summary(state))
            .unwrap_or_default()
    }
}

fn permission_name(mode: hanafubuki_agent::AgentPermissionMode) -> &'static str {
    match mode {
        hanafubuki_agent::AgentPermissionMode::Manual => "manual",
        hanafubuki_agent::AgentPermissionMode::AutoReview => "auto-review",
        hanafubuki_agent::AgentPermissionMode::FullAccess => "full-access",
    }
}

fn sandbox_name(mode: hanafubuki_agent::AgentSandboxMode) -> String {
    match mode {
        hanafubuki_agent::AgentSandboxMode::ReadOnly => "read-only".to_string(),
        hanafubuki_agent::AgentSandboxMode::WorkspaceWrite => "workspace-write".to_string(),
        hanafubuki_agent::AgentSandboxMode::FullAccess => "full-access".to_string(),
    }
}

fn proxy_name(proxy: &hanafubuki_agent::AgentProxy) -> String {
    match proxy {
        hanafubuki_agent::AgentProxy::System => "system".to_string(),
        hanafubuki_agent::AgentProxy::Disabled => "disabled".to_string(),
        hanafubuki_agent::AgentProxy::Explicit(url) => redact_userinfo(url),
    }
}

/// The authority of a proxy URL without its credentials.
fn redact_userinfo(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    match authority.rsplit_once('@') {
        Some((_, host)) => format!("{scheme}://***@{host}{}", &rest[authority.len()..]),
        None => url.to_string(),
    }
}

fn log_level_name(level: hanafubuki_agent::AgentLogLevel) -> &'static str {
    match level {
        hanafubuki_agent::AgentLogLevel::Off => "off",
        hanafubuki_agent::AgentLogLevel::Error => "error",
        hanafubuki_agent::AgentLogLevel::Warn => "warn",
        hanafubuki_agent::AgentLogLevel::Info => "info",
        hanafubuki_agent::AgentLogLevel::Debug => "debug",
        hanafubuki_agent::AgentLogLevel::Trace => "trace",
    }
}
