//! Shared environment overlays for native child processes.

use std::collections::HashMap;
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
    pub fn with_inherited_overlay(inherited: HashMap<String, Option<String>>) -> Self {
        Self {
            inherited: Arc::new(inherited),
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn inherited_snapshot(&self) -> HashMap<String, Option<String>> {
        self.inherited.as_ref().clone()
    }

    pub fn replace(&self, entries: HashMap<String, Option<String>>) {
        *self.entries.write() = entries;
    }

    pub fn snapshot(&self) -> HashMap<String, Option<String>> {
        self.entries.read().clone()
    }
}

#[cfg(test)]
mod tests;
