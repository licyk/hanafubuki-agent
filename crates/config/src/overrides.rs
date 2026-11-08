// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/common/src/config_override.rs` at tag `rust-v0.93.0`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: no `clap` type (the CLI collects the raw
// strings), the overrides are parsed once into one TOML table, and the
// parsing errors are the caller's to report.

//! `-c key=value` overrides: the last layer of the standalone configuration.
//!
//! The key is a dotted path into `config.toml` (`memory.enabled`,
//! `network.proxy`), the value TOML (`true`, `"full_access"`, `[1, 2]`). A
//! value that does not parse as TOML is taken as a string, so
//! `-c logLevel=debug` needs no quotes.

use serde::de::Error as SerdeError;
use toml::{Table, Value};

/// Parsed `-c` overrides, as the document they make: later overrides of the
/// same key replace earlier ones.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConfigOverrides {
    document: Table,
}

impl ConfigOverrides {
    /// Parses each `key=value` in order; fails on the first that has no `=`
    /// or no key.
    pub fn parse<S: AsRef<str>>(raw: &[S]) -> Result<Self, String> {
        let mut document = Value::Table(Table::new());
        for entry in raw {
            let (path, value) = parse_override(entry.as_ref())?;
            apply_single_override(&mut document, &path, value);
        }
        match document {
            Value::Table(document) => Ok(Self { document }),
            _ => Ok(Self::default()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.document.is_empty()
    }

    /// The overrides as a settings document.
    pub(crate) fn document(&self) -> &Table {
        &self.document
    }
}

fn parse_override(raw: &str) -> Result<(String, Value), String> {
    // Only split on the *first* '=' so values are free to contain the
    // character.
    let mut parts = raw.splitn(2, '=');
    let key = match parts.next() {
        Some(key) => key.trim(),
        None => return Err("Override missing key".to_string()),
    };
    let value = parts
        .next()
        .ok_or_else(|| format!("Invalid override (missing '='): {raw}"))?
        .trim();
    if key.is_empty() {
        return Err(format!("Empty key in override: {raw}"));
    }
    // Attempt to parse as TOML. If that fails, treat it as a raw string. This
    // allows convenient usage such as `-c logLevel=debug` without the quotes.
    let value = match parse_toml_value(value) {
        Ok(value) => value,
        Err(_) => {
            // Strip leading/trailing quotes if present
            let trimmed = value.trim().trim_matches(|c| c == '"' || c == '\'');
            Value::String(trimmed.to_string())
        }
    };
    Ok((key.to_string(), value))
}

/// Apply a single override onto `root`, creating intermediate tables as
/// necessary.
fn apply_single_override(root: &mut Value, path: &str, value: Value) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = root;

    for (index, part) in parts.iter().enumerate() {
        let is_last = index == parts.len() - 1;

        if is_last {
            match current {
                Value::Table(table) => {
                    table.insert((*part).to_string(), value);
                }
                _ => {
                    let mut table = Table::new();
                    table.insert((*part).to_string(), value);
                    *current = Value::Table(table);
                }
            }
            return;
        }

        // Traverse or create intermediate table.
        match current {
            Value::Table(table) => {
                current = table
                    .entry((*part).to_string())
                    .or_insert_with(|| Value::Table(Table::new()));
            }
            _ => {
                *current = Value::Table(Table::new());
                if let Value::Table(table) = current {
                    current = table
                        .entry((*part).to_string())
                        .or_insert_with(|| Value::Table(Table::new()));
                }
            }
        }
    }
}

fn parse_toml_value(raw: &str) -> Result<Value, toml::de::Error> {
    let wrapped = format!("_x_ = {raw}");
    let table: Table = toml::from_str(&wrapped)?;
    table
        .get("_x_")
        .cloned()
        .ok_or_else(|| SerdeError::custom("missing sentinel key"))
}

#[cfg(test)]
mod tests;
