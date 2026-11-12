// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Drafting a domain form into the typed patch that saves it whole.

use super::*;

pub(crate) fn draft(
    section: ConfigSection,
    form: &mut FormView,
    current: &ConfigState,
) -> Option<ConfigChange> {
    form.clear_errors();
    let change = match section {
        ConfigSection::General => ConfigChange::Agent(AgentSettingsPatch {
            permission_mode: Some(PERMISSIONS.get(form.choice(PERMISSION)).copied()?),
            model_metadata: Some(AgentModelMetadataSettings {
                discovery: DISCOVERIES.get(form.choice(DISCOVERY)).copied()?,
                models_dev: form.toggle(MODELS_DEV),
            }),
            ..AgentSettingsPatch::default()
        }),
        ConfigSection::Sandbox => {
            let roots = split_paths(form.text(WRITABLE_ROOTS));
            if roots.iter().any(|root| !root.is_absolute()) {
                form.set_error(WRITABLE_ROOTS, "Every writable root must be absolute.");
                return None;
            }
            ConfigChange::Agent(AgentSettingsPatch {
                sandbox_mode: Some(SANDBOX_MODES.get(form.choice(SANDBOX)).copied().flatten()),
                sandbox_workspace_write: Some(AgentSandboxWorkspaceWrite {
                    writable_roots: roots,
                    network_access: form.toggle(NETWORK_ACCESS),
                }),
                windows: Some(AgentWindowsSettings {
                    sandbox: WINDOWS_SANDBOXES
                        .get(form.choice(WINDOWS_SANDBOX))
                        .copied()
                        .flatten(),
                }),
                ..AgentSettingsPatch::default()
            })
        }
        ConfigSection::Memory => {
            let max_source = number(form, "max_source_age_days", 1, 3650)?;
            let idle = number(form, "minimum_idle_hours", 0, 8760)?;
            let sessions = number(form, "max_sessions", 1, 64)?;
            let stage1 = number(form, "max_stage1", 1, 4096)?;
            let unused = number(form, "max_unused_days", 1, 3650)?;
            ConfigChange::Agent(AgentSettingsPatch {
                memory: Some(AgentMemorySettings {
                    enabled: form.toggle("enabled"),
                    use_memories: form.toggle("use"),
                    generate_memories: form.toggle("generate"),
                    exclude_external_context: form.toggle("exclude_external"),
                    extraction_model: optional_text(form.text("extraction_model")),
                    consolidation_model: optional_text(form.text("consolidation_model")),
                    max_source_age_days: max_source,
                    minimum_idle_hours: idle,
                    max_sessions_per_pass: sessions,
                    max_stage1_outputs: stage1,
                    max_unused_days: unused,
                }),
                ..AgentSettingsPatch::default()
            })
        }
        ConfigSection::Web => {
            let provider = PROVIDERS.get(form.choice(PROVIDER)).copied()?;
            let url = optional_text(form.text(SEARXNG_URL));
            if provider == AgentWebSearchProvider::Searxng && url.is_none() {
                form.set_error(SEARXNG_URL, "SearXNG needs its base URL.");
                return None;
            }
            ConfigChange::Agent(AgentSettingsPatch {
                web_search_provider: Some(provider),
                searxng_base_url: Some(url),
                web_fetch_use_jina_reader: Some(form.toggle(JINA_READER)),
                web_fetch_resolve_through_proxy: Some(form.toggle(RESOLVE_THROUGH_PROXY)),
                ..AgentSettingsPatch::default()
            })
        }
        ConfigSection::Compaction => {
            let strategy = STRATEGIES.get(form.choice(STRATEGY)).copied()?;
            let background = AgentBackgroundCompactionSettings {
                threshold_percent: number(form, "background_threshold", 1, 100)?,
                child_sessions: form.toggle("child_sessions"),
            };
            let min_summary = number(form, "min_summary", 1, u64::MAX)?;
            let max_summary = number(form, "max_summary", 1, u64::MAX)?;
            if min_summary > max_summary {
                form.set_error("min_summary", "The minimum is above the maximum.");
                return None;
            }
            ConfigChange::Agent(AgentSettingsPatch {
                compaction: Some(AgentCompactionSettings {
                    strategy,
                    background,
                    hierarchical: AgentHierarchicalCompactionSettings {
                        max_context_limit_percent: number(form, "max_context", 1, 100)?,
                        emergency_threshold_percent: number(form, "emergency", 1, 100)?,
                        nudge_growth_tokens: number(form, "nudge_growth", 1, u64::MAX)?,
                        preserve_recent_messages: number(form, "preserve_messages", 0, u64::MAX)?,
                        preserve_recent_tokens: number(form, "preserve_tokens", 0, u64::MAX)?,
                        min_compress_range_chars: number(form, "min_compress_range", 1, u64::MAX)?,
                        min_summary_length: min_summary,
                        max_summary_length: max_summary,
                        tiers: form.toggle("tiers"),
                        protected_tools: split_strings(form.text("protected_tools")),
                        message_timestamps: form.toggle("timestamps"),
                        absorb: form.toggle("absorb"),
                        content_store: form.toggle("content_store"),
                        crush: form.toggle("crush"),
                        rules: form.toggle("rules"),
                    },
                }),
                ..AgentSettingsPatch::default()
            })
        }
        ConfigSection::Network => {
            let proxy = match form.choice(PROXY_MODE) {
                0 => AgentProxy::System,
                1 => AgentProxy::Disabled,
                _ => match AgentProxy::try_from(form.text(PROXY_URL).trim().to_string()) {
                    Ok(proxy) => proxy,
                    Err(error) => {
                        form.set_error(PROXY_URL, error);
                        return None;
                    }
                },
            };
            ConfigChange::Standalone(AgentStandaloneSettingsPatch {
                network: Some(AgentNetworkSettings { proxy }),
                log_level: Some(LOG_LEVELS.get(form.choice(LOG_LEVEL)).copied()?),
            })
        }
        ConfigSection::Interface => ConfigChange::Tui(AgentTuiPreferencesPatch {
            fullscreen_transcript: Some(form.toggle(FULLSCREEN)),
            alternate_screen: Some(SCREENS.get(form.choice(ALTERNATE_SCREEN)).copied()?),
            raw_output: Some(form.toggle(RAW_OUTPUT)),
            status_line_colors: Some(form.toggle(STATUS_COLORS)),
            ..AgentTuiPreferencesPatch::default()
        }),
        // The center drafts a plugin's options itself (`options.rs`).
        ConfigSection::Plugins => return None,
        ConfigSection::Providers
        | ConfigSection::Role
        | ConfigSection::Mcp
        | ConfigSection::Hooks
        | ConfigSection::Layers => unreachable!("no form"),
    };
    let _ = current;
    Some(change)
}

fn number<T: TryFrom<u64>>(
    form: &mut FormView,
    id: FieldId,
    minimum: u64,
    maximum: u64,
) -> Option<T> {
    let value = match form.text(id).trim().parse::<u64>() {
        Ok(value) if value >= minimum && value <= maximum => value,
        _ => {
            form.set_error(
                id,
                format!("Enter a whole number from {minimum} to {maximum}."),
            );
            return None;
        }
    };
    T::try_from(value).ok()
}

fn optional_text(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn split_strings(text: &str) -> Vec<String> {
    text.split(',')
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn split_paths(text: &str) -> Vec<std::path::PathBuf> {
    split_strings(text)
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect()
}
