//! A guest's tool presenters: `contributes.toolPresenters` pairs a tool with
//! the fields of its structured output, which the interface shows labelled
//! and in that order instead of the fallback's alphabetical keys. An output
//! that is not an object, or a field it does not have, is left to the
//! fallback; nothing here calls the guest.

use std::sync::Arc;

use hanafubuki_agent_contracts::{AgentToolDisplay, AgentToolPresenter, ToolPresenterRegistry};
use hanafubuki_agent_tool_contracts::ToolOutput;
use kardia_core::Context;
use serde_json::Value;

use super::register_in;
use crate::package::GuestPackage;

pub(super) fn register(ctx: &Context, package: &GuestPackage, issues: &mut Vec<String>) {
    if package.contributes.tool_presenters.is_empty() {
        return;
    }
    let namespace = super::tools::namespace(&package.id.plugin_name);
    let mut presenters = Vec::new();
    for declared in &package.contributes.tool_presenters {
        if !package
            .contributes
            .tools
            .iter()
            .any(|tool| tool.name == declared.tool)
        {
            issues.push(format!(
                "the presenter of `{}` was left out: the manifest declares no such tool",
                declared.tool
            ));
            continue;
        }
        // The registry keys a presenter by the names it lists, and a guest's
        // tool name is built from its manifest: the one name it presents is
        // registered once, with the guest's fiber.
        let name: &'static str =
            Box::leak(format!("{}__{}", namespace, declared.tool).into_boxed_str());
        presenters.push(Arc::new(FieldsPresenter {
            name,
            fields: declared.fields.clone(),
        }) as Arc<dyn AgentToolPresenter>);
    }
    if presenters.is_empty() {
        return;
    }
    for presenter in presenters {
        register_in::<ToolPresenterRegistry>(ctx, presenter, "its tool presenter", issues);
    }
}

/// One tool's output as its declared fields.
struct FieldsPresenter {
    /// The tool's name as the model knows it, which the registry keys on.
    name: &'static str,
    fields: Vec<hanafubuki_agent_packages::ContributedField>,
}

impl AgentToolPresenter for FieldsPresenter {
    fn tools(&self) -> &[&'static str] {
        std::slice::from_ref(&self.name)
    }

    fn display(
        &self,
        _tool: &str,
        _arguments: &Value,
        output: &ToolOutput,
    ) -> Option<AgentToolDisplay> {
        let structured = output.structured_content.as_ref()?;
        let object = structured.as_object()?;
        let rows = self
            .fields
            .iter()
            .filter_map(|field| {
                let value = object.get(&field.key)?;
                Some((field.label.clone(), text_of(value)))
            })
            .collect::<Vec<_>>();
        (!rows.is_empty()).then_some(AgentToolDisplay::Fields { rows })
    }
}

/// A field's value as one line: text as it is, anything else as compact
/// JSON.
fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}
