//! `config path|get|set|edit`: the user's `config.toml`, read with every
//! layer applied and written through the configuration's own writer. A
//! plugin option (`pluginOptions.<plugin>.<option>`) is read and checked
//! against its plugin's descriptor.

use std::process::Command;

use hanafubuki_agent::{
    agent_plugin_descriptors, parse_config_value, AgentConfig, AgentRuntimeHost, PluginOptionKey,
    PluginOptionSchema, AGENT_MCP_FILE, AGENT_PROVIDERS_FILE, AGENT_SETTINGS_FILE,
};
use serde_json::{Map, Value};

use crate::agent::Standalone;
use crate::cli::{ConfigCommand, GlobalArgs};
use crate::exit::Failure;

pub(crate) fn run(global: &GlobalArgs, command: ConfigCommand) -> Result<(), Failure> {
    let standalone = Standalone::open(global, AgentRuntimeHost::default())?;
    let result = match command {
        ConfigCommand::Path => {
            path(&standalone);
            Ok(())
        }
        ConfigCommand::Get { key } => get(standalone.runtime.config(), key.as_deref()),
        ConfigCommand::Set { key, value } => set(standalone.runtime.config(), &key, &value),
        ConfigCommand::Edit => edit(&standalone),
    };
    standalone.shutdown();
    result
}

fn path(standalone: &Standalone) {
    let config = &standalone.dirs.config_dir;
    let storage = standalone.runtime.storage_paths();
    for (name, path) in [
        ("config", config.join(AGENT_SETTINGS_FILE)),
        ("providers", config.join(AGENT_PROVIDERS_FILE)),
        ("mcp", config.join(AGENT_MCP_FILE)),
        ("data", storage.root.clone()),
        (
            "log",
            storage
                .logs_dir
                .join(hanafubuki_agent::logging::LOG_FILE_NAME),
        ),
    ] {
        println!("{name:<10}{}", path.display());
    }
}

fn get(config: &AgentConfig, key: Option<&str>) -> Result<(), Failure> {
    if let Some(option) = key.map(plugin_option).transpose()?.flatten() {
        let schema = option_schema(&option.row)?;
        let settings = config.settings().map_err(Failure::new)?;
        let resolved = settings.plugin_options.resolve(&option.row, &schema);
        let value = resolved
            .value(&option.key)
            .ok_or_else(|| unknown_option(&option))?;
        println!("{}", render(value.to_json())?);
        return Ok(());
    }
    let value = match key {
        Some(key) => config.value(key).map_err(Failure::new)?,
        None => Some(Value::Object(
            config.resolved_document().map_err(Failure::new)?,
        )),
    };
    if let Some(value) = value {
        println!("{}", render(without_credentials(value))?);
    }
    Ok(())
}

fn set(config: &AgentConfig, key: &str, raw: &str) -> Result<(), Failure> {
    let Some(option) = plugin_option(key)? else {
        return config.set_user_value(key, raw).map_err(Failure::new);
    };
    let schema = option_schema(&option.row)?;
    let value = parse_config_value(raw).map_err(Failure::new)?;
    let patch = Map::from_iter([(option.key.clone(), value)]);
    config
        .set_plugin_options(&option.row, &schema, &patch)
        .map(drop)
        .map_err(Failure::new)
}

fn plugin_option(key: &str) -> Result<Option<PluginOptionKey>, Failure> {
    PluginOptionKey::parse(key).map_err(Failure::new)
}

/// The options of the built-in plugin `row`, as its descriptor has them.
fn option_schema(row: &str) -> Result<PluginOptionSchema, Failure> {
    agent_plugin_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.name == row)
        .and_then(|descriptor| descriptor.options)
        .ok_or_else(|| Failure::new(format!("the plugin `{row}` has no options")))
}

fn unknown_option(option: &PluginOptionKey) -> Failure {
    Failure::new(format!(
        "the plugin `{}` has no option `{}`",
        option.row, option.key
    ))
}

/// The value with the credentials of a URL in it (a proxy's
/// `user:password@`) replaced by `***`.
fn without_credentials(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact_userinfo(&text)),
        Value::Object(table) => Value::Object(
            table
                .into_iter()
                .map(|(key, value)| (key, without_credentials(value)))
                .collect(),
        ),
        value => value,
    }
}

fn redact_userinfo(text: &str) -> String {
    let Some((scheme, rest)) = text.split_once("://") else {
        return text.to_string();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    match authority.rsplit_once('@') {
        Some((_, host)) => format!("{scheme}://***@{host}{}", &rest[authority.len()..]),
        None => text.to_string(),
    }
}

/// A string as it is, a table as a TOML document, anything else as a TOML
/// value.
fn render(value: Value) -> Result<String, Failure> {
    let unprintable = |error: toml::ser::Error| Failure::new(format!("the value: {error}"));
    match value {
        Value::String(text) => Ok(text),
        Value::Object(table) => toml::to_string(&without_nulls(table))
            .map(|text| text.trim_end().to_string())
            .map_err(unprintable),
        value => toml::Value::try_from(value)
            .map(|value| value.to_string())
            .map_err(unprintable),
    }
}

/// TOML has no null: an unset key is left out.
fn without_nulls(table: Map<String, Value>) -> Map<String, Value> {
    table
        .into_iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(key, value)| match value {
            Value::Object(table) => (key, Value::Object(without_nulls(table))),
            value => (key, value),
        })
        .collect()
}

/// Opens the user's file in `$VISUAL` or `$EDITOR`.
fn edit(standalone: &Standalone) -> Result<(), Failure> {
    let file = standalone.dirs.config_dir.join(AGENT_SETTINGS_FILE);
    let editor = ["VISUAL", "EDITOR"]
        .into_iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .ok_or_else(|| Failure::new("set $VISUAL or $EDITOR to the editor to open"))?;
    // An editor with arguments (`code --wait`) is split on whitespace.
    let mut words = editor.split_whitespace();
    let program = words.next().unwrap_or_default();
    std::fs::create_dir_all(&standalone.dirs.config_dir).map_err(|error| {
        Failure::new(format!(
            "create {}: {error}",
            standalone.dirs.config_dir.display()
        ))
    })?;
    let status = Command::new(program)
        .args(words)
        .arg(&file)
        .status()
        .map_err(|error| Failure::new(format!("run {editor}: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Failure::new(format!("{editor} exited with {status}")))
    }
}
