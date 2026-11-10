// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugin-common/src/plugin_id.rs` (the `<plugin>@<marketplace>`
// identity and its segment rules) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the identity is ordered and displayed as
// its key; the error is a plain message.

use std::fmt;

/// An installed plugin's identity, `<plugin>@<marketplace>`: its key in
/// `[plugins]`, its directory in the cache and its owner in every source it
/// contributes to. A plugin name allows ASCII letters, digits, `.`, `_` and
/// `-` with dots only between non-empty segments; a marketplace name has no
/// dots.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PluginId {
    pub plugin_name: String,
    pub marketplace_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginIdError(pub String);

impl fmt::Display for PluginIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PluginIdError {}

impl PluginId {
    pub fn new(plugin_name: &str, marketplace_name: &str) -> Result<Self, PluginIdError> {
        validate_plugin_name(plugin_name)?;
        validate_marketplace_name(marketplace_name)?;
        Ok(Self {
            plugin_name: plugin_name.to_string(),
            marketplace_name: marketplace_name.to_string(),
        })
    }

    pub fn parse(key: &str) -> Result<Self, PluginIdError> {
        let Some((plugin_name, marketplace_name)) = key
            .rsplit_once('@')
            .filter(|(plugin, marketplace)| !plugin.is_empty() && !marketplace.is_empty())
        else {
            return Err(PluginIdError(format!(
                "invalid plugin key `{key}`; expected <plugin>@<marketplace>"
            )));
        };
        Self::new(plugin_name, marketplace_name)
            .map_err(|PluginIdError(message)| PluginIdError(format!("{message} in `{key}`")))
    }

    pub fn key(&self) -> String {
        format!("{}@{}", self.plugin_name, self.marketplace_name)
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}@{}", self.plugin_name, self.marketplace_name)
    }
}

pub fn validate_plugin_name(name: &str) -> Result<(), PluginIdError> {
    validate_segment(name, "plugin name", true)
}

pub fn validate_marketplace_name(name: &str) -> Result<(), PluginIdError> {
    validate_segment(name, "marketplace name", false)
}

fn validate_segment(segment: &str, kind: &str, allow_dots: bool) -> Result<(), PluginIdError> {
    let invalid = |message: String| Err(PluginIdError(format!("invalid {kind}: {message}")));
    if segment.is_empty() {
        return invalid("must not be empty".to_string());
    }
    if allow_dots && matches!(segment, "." | "..") {
        return invalid("path traversal is not allowed".to_string());
    }
    if allow_dots && (segment.starts_with('.') || segment.ends_with('.') || segment.contains(".."))
    {
        return invalid("dots must separate non-empty name segments".to_string());
    }
    let allowed = |ch: char| {
        ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') || (allow_dots && ch == '.')
    };
    if !segment.chars().all(allowed) {
        let characters = if allow_dots {
            "ASCII letters, digits, `.`, `_`, and `-`"
        } else {
            "ASCII letters, digits, `_`, and `-`"
        };
        return invalid(format!("only {characters} are allowed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
