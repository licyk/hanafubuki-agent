use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

/// Typed per-turn data, keyed by type.
///
/// The turn admission contributors decide what a turn carries (today the
/// `skills` plugin's frozen Skill catalog and the [`crate::SelectableSkills`]
/// the kernel reads); every tool call and prompt contributor of the turn
/// reads the value of the type it knows with [`TurnExtensions::get`]. The
/// same map carries the values prompt contributors prepare for a sampling
/// step ([`crate::PromptRequest::prepared`], with the
/// [`crate::AgentMemoryStep`] the kernel reads). Values are shared, never
/// copied per call, and a domain type travels here without this crate
/// depending on its domain.
#[derive(Clone, Default)]
pub struct TurnExtensions {
    values: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl TurnExtensions {
    /// Adds `value`, replacing a value of the same type.
    pub fn insert<T: Any + Send + Sync>(&mut self, value: Arc<T>) {
        self.values.insert(TypeId::of::<T>(), value);
    }

    pub fn with<T: Any + Send + Sync>(mut self, value: Arc<T>) -> Self {
        self.insert(value);
        self
    }

    /// Adds every value of `other`, unless `self` already holds a value of
    /// one of its types: then nothing is added and the result is `false`.
    pub fn merge_disjoint(&mut self, other: TurnExtensions) -> bool {
        if other.values.keys().any(|key| self.values.contains_key(key)) {
            return false;
        }
        self.values.extend(other.values);
        true
    }

    pub fn get<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.values
            .get(&TypeId::of::<T>())
            .and_then(|value| value.downcast_ref::<T>())
    }

    pub fn get_arc<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.values
            .get(&TypeId::of::<T>())
            .cloned()
            .and_then(|value| value.downcast::<T>().ok())
    }
}

impl std::fmt::Debug for TurnExtensions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TurnExtensions")
            .field("values", &self.values.len())
            .finish()
    }
}

#[cfg(test)]
mod tests;
