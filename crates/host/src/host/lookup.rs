//! Whether a row is available, and the service it provides.
//!
//! A row that the application resolves (a module whose commands and
//! consumers call its service) provides that service under its own name
//! (`Context::provide_arc_as(name, ..)`, declared in its `provides`). A
//! caller outside kardia reaches it from the host's root, which reads the
//! live binding: the service of the row's current instance, while the row
//! is active. A retried row provides a new instance, so a caller resolves
//! per call and never keeps one.

use std::any::Any;
use std::sync::Arc;

use super::{HostProfile, HostRoot, ModuleHost};
use crate::status::ModuleState;

impl<K: HostProfile> ModuleHost<K> {
    /// `Ok` while the row `name` is active; otherwise why it is not, as
    /// "the <name> <noun> is unavailable: <reason>".
    pub fn available(&self, name: &str) -> Result<(), String> {
        let state = self.state();
        let row = state
            .rows
            .iter()
            .find(|row| row.name == name)
            .ok_or_else(|| format!("{} {name} does not exist", K::NOUN))?;
        let status = row.status(Vec::new(), None);
        let reason = match (&state.root, status.state) {
            (HostRoot::NotStarted, _) => "it has not started yet".to_string(),
            (HostRoot::Running(_) | HostRoot::Stopping(_), ModuleState::Active) => return Ok(()),
            (HostRoot::Stopped | HostRoot::Stopping(_), _) | (_, ModuleState::Disposed) => {
                format!("{} is stopping", K::HOST)
            }
            (HostRoot::Running(_), ModuleState::Pending) => "it is starting".to_string(),
            (HostRoot::Running(_), ModuleState::Disabled) => "it is turned off".to_string(),
            (HostRoot::Running(_), ModuleState::Failed) => {
                status.error.unwrap_or_else(|| "it failed".to_string())
            }
        };
        Err(unavailable::<K>(name, &reason))
    }

    /// The service of type `T` the active row `name` provides under its
    /// name; why it is unavailable otherwise ([`Self::available`]).
    pub fn service<T: Any + Send + Sync>(&self, name: &str) -> Result<Arc<T>, String> {
        self.available(name)?;
        let root = match &self.state().root {
            HostRoot::Running(root) | HostRoot::Stopping(root) => root.clone(),
            HostRoot::NotStarted | HostRoot::Stopped => {
                return Err(unavailable::<K>(name, &format!("{} is stopping", K::HOST)))
            }
        };
        root.live_as::<T>(name)
            .map(|service| service.arc().clone())
            .ok_or_else(|| unavailable::<K>(name, "it provides no such service"))
    }
}

fn unavailable<K: HostProfile>(name: &str, reason: &str) -> String {
    format!("the {name} {} is unavailable: {reason}", K::NOUN)
}
