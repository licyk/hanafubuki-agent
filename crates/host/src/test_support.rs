//! Helpers for other crates' tests.

use std::sync::Arc;

use crate::RegistryEntry;

/// An entry that belongs to no registry, so it is never retired: for tests
/// that plan from an entry without mounting a plugin.
pub fn detached_entry<T>(owner: impl Into<String>, item: T) -> Arc<RegistryEntry<T>> {
    RegistryEntry::detached(owner.into(), item)
}
