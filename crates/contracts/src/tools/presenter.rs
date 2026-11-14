use std::sync::Arc;

use hanafubuki_agent_tool_contracts::ToolOutput;
use kardia_macros::Service;
use serde_json::Value;

use super::AgentToolDisplay;
use crate::{ExtensionPoint, RegistrationError, Registry, RegistryEntry, RegistryService};

/// The characters a call's subject keeps.
pub const MAX_TOOL_SUBJECT_CHARS: usize = 200;

/// How an interface shows the calls of the tools a plugin owns: the
/// header's subject from the call's arguments, and the finished call's
/// display. Not model-visible.
///
/// `arguments` are the call's arguments normalized against the tool's
/// schema, as its handler received them, when the tool is registered; the
/// model's own otherwise (a resumed session whose tool is gone). Decode them
/// tolerantly and return `None` for what you cannot read: the call then
/// shows no subject, or the fallback display. Both are synchronous, pure
/// and bounded, and never panic.
pub trait AgentToolPresenter: Send + Sync + 'static {
    /// The flattened names of the tools this presenter presents.
    fn tools(&self) -> &[&'static str];

    /// What the call acts on: the command line, the path.
    fn subject(&self, _tool: &str, _arguments: &Value) -> Option<String> {
        None
    }

    /// What the finished call shows, from its committed output.
    fn display(
        &self,
        _tool: &str,
        _arguments: &Value,
        _output: &ToolOutput,
    ) -> Option<AgentToolDisplay> {
        None
    }
}

/// A presenter for tools whose output the fallback already shows: each
/// call's subject is one string argument, by tool.
pub struct ArgumentSubjects {
    subjects: &'static [(&'static str, &'static str)],
    tools: Vec<&'static str>,
}

impl ArgumentSubjects {
    /// `subjects` pairs each tool name with the argument its subject is.
    pub fn new(subjects: &'static [(&'static str, &'static str)]) -> Self {
        Self {
            subjects,
            tools: subjects.iter().map(|(tool, _)| *tool).collect(),
        }
    }
}

impl AgentToolPresenter for ArgumentSubjects {
    fn tools(&self) -> &[&'static str] {
        &self.tools
    }

    fn subject(&self, tool: &str, arguments: &Value) -> Option<String> {
        let (_, key) = self.subjects.iter().find(|(name, _)| *name == tool)?;
        arguments.get(key)?.as_str().map(str::to_string)
    }
}

/// The tool presenters, one per tool name.
#[derive(Service, Default)]
#[service(name = "agent-tool-presenters")]
pub struct ToolPresenterRegistry {
    registry: Registry<Arc<dyn AgentToolPresenter>>,
}

impl ToolPresenterRegistry {
    /// An empty registry whose entries report their faults to `faults`.
    pub fn with_faults(faults: Arc<crate::PluginFaults>) -> Self {
        Self {
            registry: Registry::with_faults(faults),
        }
    }

    /// The subject of a call of `tool`: its first line, at most
    /// [`MAX_TOOL_SUBJECT_CHARS`]. `None` without a presenter, or when it
    /// panicked (a fault of its plugin).
    pub fn subject(&self, tool: &str, arguments: &Value) -> Option<String> {
        let entry = self.presenter(tool)?;
        let subject = entry
            .guard(ExtensionPoint::ToolPresenter, |presenter| {
                presenter.subject(tool, arguments)
            })
            .ok()
            .flatten()?;
        bounded_subject(&subject)
    }

    /// What a finished call of `tool` shows: its presenter's display, or
    /// [`AgentToolDisplay::fallback`] without one, when it returns none or
    /// when it panicked; with what the handler's `metadata` adds to it
    /// ([`AgentToolDisplay::with_metadata`]).
    pub fn display(
        &self,
        tool: &str,
        arguments: &Value,
        output: &ToolOutput,
        metadata: &super::AgentToolDisplayMetadata,
    ) -> AgentToolDisplay {
        self.presenter(tool)
            .and_then(|entry| {
                entry
                    .guard(ExtensionPoint::ToolPresenter, |presenter| {
                        presenter.display(tool, arguments, output)
                    })
                    .ok()
                    .flatten()
            })
            .unwrap_or_else(|| AgentToolDisplay::fallback(output))
            .with_metadata(metadata)
    }

    fn presenter(&self, tool: &str) -> Option<Arc<RegistryEntry<Arc<dyn AgentToolPresenter>>>> {
        self.registry
            .snapshot()
            .entries()
            .iter()
            .find(|entry| !entry.is_retired() && entry.item().tools().contains(&tool))
            .cloned()
    }
}

impl RegistryService for ToolPresenterRegistry {
    type Item = Arc<dyn AgentToolPresenter>;

    fn registry(&self) -> &Registry<Self::Item> {
        &self.registry
    }

    /// Rejects a presenter of a tool another one presents.
    fn admit(existing: &[&Self::Item], candidate: &Self::Item) -> Result<(), RegistrationError> {
        let taken = candidate.tools().iter().find(|tool| {
            existing
                .iter()
                .any(|presenter| presenter.tools().contains(tool))
        });
        match taken {
            Some(tool) => Err(RegistrationError::Duplicate {
                registry: <Self as kardia_core::Service>::NAME,
                identity: tool.to_string(),
            }),
            None => Ok(()),
        }
    }
}

fn bounded_subject(subject: &str) -> Option<String> {
    let mut lines = subject
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let first = lines.next()?;
    let mut bounded: String = first.chars().take(MAX_TOOL_SUBJECT_CHARS).collect();
    if bounded.len() < first.len() || lines.next().is_some() {
        bounded.push('…');
    }
    Some(bounded)
}
