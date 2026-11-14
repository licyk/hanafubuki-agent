//! What each plugin row is, for the people who manage it: a summary of what
//! it gives the model or the user, and the options it takes
//! (`[pluginOptions.<row>]`). Every row of the host has exactly one
//! descriptor, a host's own rows included (`AgentHostPorts::plugins`).

use std::collections::BTreeMap;

use hanafubuki_agent_config::{
    AgentSettings, PluginOptionSchema, PluginOptionsSettings, ResolvedPluginOptions, SettingKind,
    SettingSpec, SettingValue,
};
use hanafubuki_agent_plugin_execution::{
    COMMAND_TIMEOUT_MINUTES_LIMITS, DEFAULT_COMMAND_TIMEOUT_MINUTES,
};
use serde::Serialize;

use crate::plugins::subagents_option_schema;

/// One row's descriptor.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPluginDescriptor {
    /// The row's plugin name, as its `PluginDef` names it.
    pub name: &'static str,
    /// What the plugin gives the model or the user, in one line.
    pub summary: &'static str,
    /// The options it takes, a section of the settings catalog that the
    /// settings center edits; `None` for a row without options.
    pub options: Option<PluginOptionSchema>,
}

impl AgentPluginDescriptor {
    /// A row without options.
    pub fn new(name: &'static str, summary: &'static str) -> Self {
        Self {
            name,
            summary,
            options: None,
        }
    }

    pub fn with_options(mut self, options: PluginOptionSchema) -> Self {
        self.options = Some(options);
        self
    }
}

/// The descriptors of the Agent's own rows, the core ones first, in mount
/// order: what `config get` and `config set` check a plugin option against.
pub fn agent_plugin_descriptors() -> Vec<AgentPluginDescriptor> {
    let row = AgentPluginDescriptor::new;
    vec![
        row(
            "agent-host-services",
            "The services the Agent's plugins share",
        ),
        row(
            "agent-kernel",
            "The kernel's registries every plugin contributes to",
        ),
        row("web", "Web search and page fetching for the model"),
        row(
            "workspace-fs",
            "Listing, searching, reading and editing the workspace's files",
        ),
        row(
            "view-image",
            "Showing the model an image from the workspace",
        ),
        row("execution", "Running commands in the sandbox").with_options(execution_option_schema()),
        row("mcp", "The tools and resources of the MCP servers"),
        row("memory", "What the Agent remembers across sessions"),
        row(
            "skills",
            "The Skills catalog and the tools that read Skills",
        ),
        row("titles", "Titles for sessions"),
        row("recap", "Catch-ups on a session"),
        row(
            "goal",
            "A session's goal, which continues the turn until it is met",
        ),
        row(
            "subagents",
            "Sub-agents a conversation spawns and waits for",
        )
        .with_options(subagents_option_schema()),
        row("review", "Code reviews in a read-only session"),
        row("worktree", "Git worktrees under the data directory"),
        row("hooks", "The user's and the plugins' hooks"),
        row("plugins", "The installed plugins and their code"),
        row(
            "workspace-context",
            "The workspace's instructions, roots and environment",
        ),
        row(
            "permission-mode",
            "The permission mode and the sandbox, as the model reads them",
        ),
    ]
}

/// The `execution` row's options. Built here, since the plugin's crate
/// may not depend on the one that defines the schema, from the bounds and
/// default the plugin decodes with.
fn execution_option_schema() -> PluginOptionSchema {
    let (minimum, maximum) = COMMAND_TIMEOUT_MINUTES_LIMITS;
    PluginOptionSchema {
        options: vec![SettingSpec {
            summary: "How long a command may run before it is stopped (minutes)".to_string(),
            default: Some(SettingValue::Integer(
                DEFAULT_COMMAND_TIMEOUT_MINUTES as i64,
            )),
            ..SettingSpec::new(
                "commandTimeoutMinutes",
                "Command timeout",
                SettingKind::Integer {
                    minimum: Some(minimum as i64),
                    maximum: Some(maximum as i64),
                },
            )
        }],
    }
}

/// Every row's descriptor, by name.
pub(crate) struct AgentPluginDescriptors {
    by_name: BTreeMap<&'static str, AgentPluginDescriptor>,
}

impl AgentPluginDescriptors {
    /// The Agent's descriptors and the host's; a name described twice is
    /// refused.
    pub(crate) fn new(host: Vec<AgentPluginDescriptor>) -> Result<Self, String> {
        let mut by_name = BTreeMap::new();
        for descriptor in agent_plugin_descriptors().into_iter().chain(host) {
            let name = descriptor.name;
            if by_name.insert(name, descriptor).is_some() {
                return Err(format!("the Agent plugin `{name}` has two descriptors"));
            }
        }
        Ok(Self { by_name })
    }

    pub(crate) fn get(&self, name: &str) -> Option<&AgentPluginDescriptor> {
        self.by_name.get(name)
    }

    /// Checks that each of the rows named `names` has a descriptor. Each
    /// name has one at most, and the host refuses a name twice.
    pub(crate) fn check_rows<'a>(
        &self,
        mut names: impl Iterator<Item = &'a str>,
    ) -> Result<(), String> {
        match names.find(|name| !self.by_name.contains_key(name)) {
            Some(name) => Err(format!(
                "the Agent plugin `{name}` has no descriptor (a host's row has its own in \
                 AgentHostPorts::plugins)"
            )),
            None => Ok(()),
        }
    }

    /// The options of every row that has some, as `settings` set them.
    pub(crate) fn resolve(
        &self,
        settings: &AgentSettings,
    ) -> BTreeMap<String, ResolvedPluginOptions> {
        self.with_options()
            .map(|(name, schema)| {
                (
                    name.to_string(),
                    settings.plugin_options.resolve(name, schema),
                )
            })
            .collect()
    }

    /// The schema of the row `name`, when it has options.
    pub(crate) fn options(&self, name: &str) -> Option<&PluginOptionSchema> {
        self.get(name)?.options.as_ref()
    }

    /// The schema of every row that has options, by its name.
    pub(crate) fn schemas(&self) -> BTreeMap<String, PluginOptionSchema> {
        self.with_options()
            .map(|(name, schema)| (name.to_string(), schema.clone()))
            .collect()
    }

    /// What the rows' tables set that cannot be used, as configuration
    /// warnings.
    pub(crate) fn issues(&self, options: &PluginOptionsSettings) -> Vec<String> {
        self.with_options()
            .flat_map(|(name, schema)| options.resolve(name, schema).issues)
            .collect()
    }

    fn with_options(&self) -> impl Iterator<Item = (&'static str, &PluginOptionSchema)> {
        self.by_name
            .iter()
            .filter_map(|(name, descriptor)| Some((*name, descriptor.options.as_ref()?)))
    }
}

#[cfg(test)]
mod tests;
