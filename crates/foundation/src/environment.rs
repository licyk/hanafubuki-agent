//! Representation validation and shared overlays for native child processes.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use parking_lot::RwLock;

/// Ordered environment overlays applied to processes created after the
/// replacement.
///
/// The immutable inherited overlay repairs ambient process state before a
/// caller's explicit environment is applied. The replaceable final overlay is
/// applied after the caller and is reserved for application-wide authorities
/// such as the proxy runtime. This is a domain-neutral child-process primitive:
/// it deliberately does not mutate the current process environment, and
/// existing children retain the environment captured when they were spawned.
#[derive(Clone)]
pub struct ChildProcessEnvironment {
    inherited: Arc<HashMap<String, Option<String>>>,
    entries: Arc<RwLock<HashMap<String, Option<String>>>>,
}

impl Default for ChildProcessEnvironment {
    fn default() -> Self {
        Self {
            inherited: Arc::new(HashMap::new()),
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl ChildProcessEnvironment {
    pub fn with_inherited_overlay(
        inherited: HashMap<String, Option<String>>,
    ) -> Result<Self, NativeEnvironmentError> {
        validate_native_environment(
            inherited
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_deref())),
        )?;
        Ok(Self {
            inherited: Arc::new(inherited),
            entries: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub fn inherited_snapshot(&self) -> HashMap<String, Option<String>> {
        self.inherited.as_ref().clone()
    }

    pub fn replace(
        &self,
        entries: HashMap<String, Option<String>>,
    ) -> Result<(), NativeEnvironmentError> {
        validate_native_environment(
            entries
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_deref())),
        )?;
        *self.entries.write() = entries;
        Ok(())
    }

    pub fn snapshot(&self) -> HashMap<String, Option<String>> {
        self.entries.read().clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeEnvironmentError {
    EmptyName,
    NameContainsEquals { name: String },
    NameContainsNul { name: String },
    ValueContainsNul { name: String },
}

impl fmt::Display for NativeEnvironmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => formatter.write_str("an empty environment variable name"),
            Self::NameContainsEquals { name } => write!(
                formatter,
                "environment variable name {name:?} containing '='"
            ),
            Self::NameContainsNul { name } => write!(
                formatter,
                "environment variable name {name:?} containing NUL"
            ),
            Self::ValueContainsNul { name } => write!(
                formatter,
                "environment variable {name:?} containing NUL in its value"
            ),
        }
    }
}

pub fn validate_native_environment_entry(
    name: &str,
    value: Option<&str>,
) -> Result<(), NativeEnvironmentError> {
    if name.is_empty() {
        return Err(NativeEnvironmentError::EmptyName);
    }
    if name.contains('=') {
        return Err(NativeEnvironmentError::NameContainsEquals {
            name: name.to_string(),
        });
    }
    if name.contains('\0') {
        return Err(NativeEnvironmentError::NameContainsNul {
            name: name.to_string(),
        });
    }
    if value.is_some_and(|value| value.contains('\0')) {
        return Err(NativeEnvironmentError::ValueContainsNul {
            name: name.to_string(),
        });
    }
    Ok(())
}

pub fn validate_native_environment<'a>(
    entries: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
) -> Result<(), NativeEnvironmentError> {
    for (name, value) in entries {
        validate_native_environment_entry(name, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
