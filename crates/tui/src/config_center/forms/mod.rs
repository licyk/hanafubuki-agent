// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Building the configuration center's domain forms from typed settings.

mod draft;

pub(super) use draft::draft;

use hanafubuki_agent::{
    AgentAlternateScreen, AgentBackgroundCompactionSettings, AgentCompactionSettings,
    AgentCompactionStrategy, AgentHierarchicalCompactionSettings, AgentLogLevel,
    AgentMemorySettings, AgentNetworkSettings, AgentPermissionMode, AgentProxy, AgentSandboxMode,
    AgentSandboxWorkspaceWrite, AgentSettingsPatch, AgentStandaloneSettingsPatch,
    AgentTuiPreferencesPatch, AgentWebSearchProvider, AgentWindowsSandbox, AgentWindowsSettings,
};

use crate::agent_bridge::ConfigState;
use crate::bottom_pane::{FieldId, FormItem, FormView};

use super::ConfigSection;

const SAVE: FieldId = "save";
const PERMISSION: FieldId = "permission";
const SANDBOX: FieldId = "sandbox";
const WRITABLE_ROOTS: FieldId = "writable_roots";
const NETWORK_ACCESS: FieldId = "network_access";
const WINDOWS_SANDBOX: FieldId = "windows_sandbox";
const PROVIDER: FieldId = "provider";
const SEARXNG_URL: FieldId = "searxng_url";
const JINA_READER: FieldId = "jina_reader";
const STRATEGY: FieldId = "strategy";
const PROXY_MODE: FieldId = "proxy_mode";
const PROXY_URL: FieldId = "proxy_url";
const LOG_LEVEL: FieldId = "log_level";
const FULLSCREEN: FieldId = "fullscreen";
const ALTERNATE_SCREEN: FieldId = "alternate_screen";
const RAW_OUTPUT: FieldId = "raw_output";
const STATUS_COLORS: FieldId = "status_colors";
const OPEN_THEME: FieldId = "open_theme";
const OPEN_STATUS_LINE: FieldId = "open_status_line";
const OPEN_TITLE: FieldId = "open_title";
const OPEN_KEYMAP: FieldId = "open_keymap";
const OPEN_PRESETS: FieldId = "open_presets";
const OPEN_TUI: FieldId = "open_tui";

const PERMISSIONS: [AgentPermissionMode; 3] = [
    AgentPermissionMode::Manual,
    AgentPermissionMode::AutoReview,
    AgentPermissionMode::FullAccess,
];
const SANDBOX_MODES: [Option<AgentSandboxMode>; 4] = [
    None,
    Some(AgentSandboxMode::ReadOnly),
    Some(AgentSandboxMode::WorkspaceWrite),
    Some(AgentSandboxMode::FullAccess),
];
const WINDOWS_SANDBOXES: [Option<AgentWindowsSandbox>; 3] = [
    None,
    Some(AgentWindowsSandbox::Unelevated),
    Some(AgentWindowsSandbox::Elevated),
];
const PROVIDERS: [AgentWebSearchProvider; 3] = [
    AgentWebSearchProvider::Auto,
    AgentWebSearchProvider::Duckduckgo,
    AgentWebSearchProvider::Searxng,
];
const STRATEGIES: [AgentCompactionStrategy; 3] = [
    AgentCompactionStrategy::Standard,
    AgentCompactionStrategy::Background,
    AgentCompactionStrategy::Hierarchical,
];
const LOG_LEVELS: [AgentLogLevel; 6] = [
    AgentLogLevel::Off,
    AgentLogLevel::Error,
    AgentLogLevel::Warn,
    AgentLogLevel::Info,
    AgentLogLevel::Debug,
    AgentLogLevel::Trace,
];
const SCREENS: [AgentAlternateScreen; 3] = [
    AgentAlternateScreen::Auto,
    AgentAlternateScreen::Always,
    AgentAlternateScreen::Never,
];

/// The command a linked editor action opens.
pub(super) fn action_command(action: FieldId) -> Option<&'static str> {
    match action {
        OPEN_THEME => Some("/theme"),
        OPEN_STATUS_LINE => Some("/statusline"),
        OPEN_TITLE => Some("/title"),
        OPEN_KEYMAP => Some("/keymap"),
        OPEN_PRESETS => Some("/presets"),
        OPEN_TUI => Some("/tui"),
        _ => None,
    }
}

/// What a domain's Save button asks the app to write.
#[derive(Clone, Debug)]
pub(crate) enum ConfigChange {
    Agent(AgentSettingsPatch),
    Standalone(AgentStandaloneSettingsPatch),
    Tui(AgentTuiPreferencesPatch),
}

impl ConfigChange {
    /// Whether a layer above the user's file changed the value this form
    /// wrote. A same-valued write is not called shadowed; it already holds.
    pub(crate) fn is_shadowed_by(&self, state: &ConfigState) -> bool {
        let settings = &state.agent.settings;
        match self {
            Self::Agent(patch) => {
                if let Some(value) = patch.permission_mode {
                    if settings.permission_mode != value {
                        return true;
                    }
                }
                if let Some(value) = patch.sandbox_mode {
                    if settings.sandbox_mode != value {
                        return true;
                    }
                }
                if let Some(value) = &patch.sandbox_workspace_write {
                    if settings.sandbox_workspace_write != *value {
                        return true;
                    }
                }
                if let Some(value) = &patch.windows {
                    if settings.windows != *value {
                        return true;
                    }
                }
                if let Some(value) = &patch.memory {
                    if settings.memory != *value {
                        return true;
                    }
                }
                if let Some(value) = patch.web_search_provider {
                    if settings.web_search_provider != value {
                        return true;
                    }
                }
                if let Some(value) = &patch.searxng_base_url {
                    let same = match (settings.searxng_base_url.as_deref(), value.as_deref()) {
                        (Some(effective), Some(desired)) => {
                            effective.trim_end_matches('/') == desired.trim_end_matches('/')
                        }
                        (None, None) => true,
                        _ => false,
                    };
                    if !same {
                        return true;
                    }
                }
                if let Some(value) = patch.web_fetch_use_jina_reader {
                    if settings.web_fetch_use_jina_reader != value {
                        return true;
                    }
                }
                patch
                    .compaction
                    .as_ref()
                    .is_some_and(|value| settings.compaction != *value)
            }
            Self::Standalone(patch) => {
                if let Some(value) = &patch.network {
                    if state.standalone.network.proxy != value.proxy {
                        return true;
                    }
                }
                patch
                    .log_level
                    .is_some_and(|value| state.standalone.log_level != value)
            }
            Self::Tui(patch) => {
                let preferences = &state.standalone.tui;
                patch
                    .fullscreen_transcript
                    .is_some_and(|value| preferences.fullscreen_transcript != value)
                    || patch
                        .alternate_screen
                        .is_some_and(|value| preferences.alternate_screen != value)
                    || patch
                        .raw_output
                        .is_some_and(|value| preferences.raw_output != value)
                    || patch
                        .status_line_colors
                        .is_some_and(|value| preferences.status_line_colors != value)
            }
        }
    }
}
impl ConfigSection {
    pub(super) fn editable(self) -> bool {
        matches!(
            self,
            Self::General
                | Self::Sandbox
                | Self::Memory
                | Self::Web
                | Self::Compaction
                | Self::Network
                | Self::Interface
        )
    }
}

pub(super) fn section_form(section: ConfigSection, state: &ConfigState) -> FormView {
    let settings = &state.agent.settings;
    match section {
        ConfigSection::General => FormView::new(
            "General & permissions",
            vec![
                FormItem::choice(
                    PERMISSION,
                    "Permission mode",
                    &["manual", "auto-review", "full-access"],
                    PERMISSIONS
                        .iter()
                        .position(|mode| *mode == settings.permission_mode)
                        .unwrap_or_default(),
                ),
                FormItem::action(SAVE, "Save"),
            ],
        )
        .with_subtitle("The plugins that are turned off are managed by /plugins."),
        ConfigSection::Sandbox => {
            let windows = settings.windows.sandbox;
            FormView::new(
                "Sandbox",
                vec![
                    FormItem::choice(
                        SANDBOX,
                        "Sandbox mode",
                        &[
                            "platform default",
                            "read-only",
                            "workspace-write",
                            "full-access",
                        ],
                        SANDBOX_MODES
                            .iter()
                            .position(|mode| *mode == settings.sandbox_mode)
                            .unwrap_or_default(),
                    ),
                    FormItem::text(
                        WRITABLE_ROOTS,
                        "Writable roots",
                        &settings
                            .sandbox_workspace_write
                            .writable_roots
                            .iter()
                            .map(|root| root.display().to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                    .placeholder(r"D:\work, E:\cache")
                    .advanced(),
                    FormItem::toggle(
                        NETWORK_ACCESS,
                        "Workspace-write network access",
                        settings.sandbox_workspace_write.network_access,
                    )
                    .advanced(),
                    FormItem::choice(
                        WINDOWS_SANDBOX,
                        "Windows sandbox",
                        &["unset", "unelevated", "elevated"],
                        WINDOWS_SANDBOXES
                            .iter()
                            .position(|mode| *mode == windows)
                            .unwrap_or_default(),
                    )
                    .advanced(),
                    FormItem::action(SAVE, "Save"),
                ],
            )
            .with_subtitle(
                "A mode the platform cannot enforce refuses a command; it never runs unconfined.",
            )
        }
        ConfigSection::Memory => memory_form(&settings.memory),
        ConfigSection::Web => FormView::new(
            "Web tools",
            vec![
                FormItem::choice(
                    PROVIDER,
                    "Search provider",
                    &["auto", "duckduckgo", "searxng"],
                    PROVIDERS
                        .iter()
                        .position(|provider| *provider == settings.web_search_provider)
                        .unwrap_or_default(),
                ),
                FormItem::text(
                    SEARXNG_URL,
                    "SearXNG base URL",
                    settings.searxng_base_url.as_deref().unwrap_or_default(),
                )
                .placeholder("https://searxng.example.com")
                .shown_if(PROVIDER, 2),
                FormItem::toggle(
                    JINA_READER,
                    "Fetch through Jina Reader after a direct failure",
                    settings.web_fetch_use_jina_reader,
                ),
                FormItem::action(SAVE, "Save"),
            ],
        ),
        ConfigSection::Compaction => compaction_form(&settings.compaction),
        ConfigSection::Network => {
            let (proxy_mode, proxy_url) = match &state.standalone.network.proxy {
                AgentProxy::System => (0, String::new()),
                AgentProxy::Disabled => (1, String::new()),
                AgentProxy::Explicit(url) => (2, url.clone()),
            };
            FormView::new(
                "Network & logging",
                vec![
                    FormItem::choice(
                        PROXY_MODE,
                        "Proxy",
                        &["system", "disabled", "custom"],
                        proxy_mode,
                    ),
                    FormItem::text(PROXY_URL, "Proxy URL", &proxy_url)
                        .secret()
                        .placeholder("http://user:password@127.0.0.1:7890")
                        .shown_if(PROXY_MODE, 2),
                    FormItem::choice(
                        LOG_LEVEL,
                        "Log level",
                        &["off", "error", "warn", "info", "debug", "trace"],
                        LOG_LEVELS
                            .iter()
                            .position(|level| *level == state.standalone.log_level)
                            .unwrap_or_default(),
                    ),
                    FormItem::action(SAVE, "Save"),
                ],
            )
            .with_subtitle("The proxy applies now; the log level applies at the next launch. Trace logs can include model content.")
        }
        ConfigSection::Interface => FormView::new(
            "Interface",
            vec![
                FormItem::toggle(
                    FULLSCREEN,
                    "Fullscreen transcript (next launch)",
                    state.standalone.tui.fullscreen_transcript,
                ),
                FormItem::choice(
                    ALTERNATE_SCREEN,
                    "Alternate screen (next launch)",
                    &["auto", "always", "never"],
                    SCREENS
                        .iter()
                        .position(|screen| *screen == state.standalone.tui.alternate_screen)
                        .unwrap_or_default(),
                ),
                FormItem::toggle(RAW_OUTPUT, "Raw output", state.standalone.tui.raw_output),
                FormItem::toggle(
                    STATUS_COLORS,
                    "Status line colors",
                    state.standalone.tui.status_line_colors,
                ),
                FormItem::action(OPEN_THEME, "Theme ›"),
                FormItem::action(OPEN_STATUS_LINE, "Status line ›"),
                FormItem::action(OPEN_TITLE, "Terminal title ›"),
                FormItem::action(OPEN_KEYMAP, "Keyboard ›"),
                FormItem::action(OPEN_PRESETS, "Presets ›"),
                FormItem::action(OPEN_TUI, "Interface mode ›"),
                FormItem::action(SAVE, "Save"),
            ],
        )
        .with_subtitle("The linked editors save their own [tui] keys."),
        ConfigSection::Providers
        | ConfigSection::Mcp
        | ConfigSection::Plugins
        | ConfigSection::Hooks
        | ConfigSection::Layers => {
            unreachable!("the overview opens these domains in their own editors")
        }
    }
}

fn memory_form(memory: &AgentMemorySettings) -> FormView {
    let number = |id: FieldId, label: &'static str, value: u64| {
        FormItem::text(id, label, &value.to_string()).advanced()
    };
    FormView::new(
        "Memory",
        vec![
            FormItem::toggle("enabled", "Enabled", memory.enabled),
            FormItem::toggle("use", "Use memories", memory.use_memories),
            FormItem::toggle("generate", "Generate memories", memory.generate_memories),
            FormItem::toggle(
                "exclude_external",
                "Exclude sessions that read external context",
                memory.exclude_external_context,
            ),
            FormItem::text(
                "extraction_model",
                "Extraction model",
                memory.extraction_model.as_deref().unwrap_or_default(),
            )
            .placeholder("the model policy's model")
            .advanced(),
            FormItem::text(
                "consolidation_model",
                "Consolidation model",
                memory.consolidation_model.as_deref().unwrap_or_default(),
            )
            .placeholder("the model policy's model")
            .advanced(),
            number(
                "max_source_age_days",
                "Maximum source age (days)",
                memory.max_source_age_days.into(),
            ),
            number(
                "minimum_idle_hours",
                "Minimum idle (hours)",
                memory.minimum_idle_hours.into(),
            ),
            number(
                "max_sessions",
                "Maximum sessions per pass",
                memory.max_sessions_per_pass as u64,
            ),
            number(
                "max_stage1",
                "Maximum stage-1 outputs",
                memory.max_stage1_outputs as u64,
            ),
            number(
                "max_unused_days",
                "Maximum unused days",
                memory.max_unused_days.into(),
            ),
            FormItem::action(SAVE, "Save"),
        ],
    )
}

fn compaction_form(settings: &AgentCompactionSettings) -> FormView {
    let strategy = STRATEGIES
        .iter()
        .position(|strategy| *strategy == settings.strategy)
        .unwrap_or_default();
    let number = |id: FieldId, label: &'static str, value: u64| {
        FormItem::text(id, label, &value.to_string())
    };
    FormView::new(
        "Compaction",
        vec![
            FormItem::choice(
                STRATEGY,
                "Strategy",
                &["standard", "background", "hierarchical"],
                strategy,
            ),
            number(
                "background_threshold",
                "Background threshold (%)",
                settings.background.threshold_percent.into(),
            )
            .shown_if(STRATEGY, 1),
            FormItem::toggle(
                "child_sessions",
                "Child sessions compact in background",
                settings.background.child_sessions,
            )
            .shown_if(STRATEGY, 1),
            number(
                "max_context",
                "Maximum context limit (%)",
                settings.hierarchical.max_context_limit_percent.into(),
            )
            .shown_if(STRATEGY, 2),
            number(
                "emergency",
                "Emergency threshold (%)",
                settings.hierarchical.emergency_threshold_percent.into(),
            )
            .shown_if(STRATEGY, 2),
            number(
                "nudge_growth",
                "Nudge growth (tokens)",
                settings.hierarchical.nudge_growth_tokens,
            )
            .shown_if(STRATEGY, 2),
            number(
                "preserve_messages",
                "Preserve recent messages",
                settings.hierarchical.preserve_recent_messages.into(),
            )
            .shown_if(STRATEGY, 2),
            number(
                "preserve_tokens",
                "Preserve recent tokens",
                settings.hierarchical.preserve_recent_tokens,
            )
            .shown_if(STRATEGY, 2),
            number(
                "min_compress_range",
                "Minimum compress range (chars)",
                settings.hierarchical.min_compress_range_chars,
            )
            .shown_if(STRATEGY, 2),
            number(
                "min_summary",
                "Minimum summary length",
                settings.hierarchical.min_summary_length,
            )
            .shown_if(STRATEGY, 2),
            number(
                "max_summary",
                "Maximum summary length",
                settings.hierarchical.max_summary_length,
            )
            .shown_if(STRATEGY, 2),
            FormItem::toggle(
                "tiers",
                "Distill older summaries in tiers",
                settings.hierarchical.tiers,
            )
            .shown_if(STRATEGY, 2),
            FormItem::text(
                "protected_tools",
                "Protected tools",
                &settings.hierarchical.protected_tools.join(", "),
            )
            .shown_if(STRATEGY, 2)
            .advanced(),
            FormItem::toggle(
                "timestamps",
                "Message timestamps",
                settings.hierarchical.message_timestamps,
            )
            .shown_if(STRATEGY, 2),
            FormItem::toggle(
                "absorb",
                "Absorb large tool results",
                settings.hierarchical.absorb,
            )
            .shown_if(STRATEGY, 2),
            FormItem::toggle(
                "content_store",
                "Content store",
                settings.hierarchical.content_store,
            )
            .shown_if(STRATEGY, 2),
            FormItem::toggle("crush", "Crush before absorb", settings.hierarchical.crush)
                .shown_if(STRATEGY, 2),
            FormItem::toggle("rules", "Rules", settings.hierarchical.rules).shown_if(STRATEGY, 2),
            FormItem::action(SAVE, "Save"),
        ],
    )
    .with_subtitle("A session takes a new strategy at its next turn or compaction.")
}
