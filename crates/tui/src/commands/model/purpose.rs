//! Why `/model`'s list is open: it sets the session's model, or it adds a
//! Provider's model to the record collection (`Model overrides`), where it
//! is given its own settings. Only the second offers a pair typed in.

use serde_json::Map;

use crate::app::{SettingsPageAction, WorkspaceEvent};
use crate::app_event::{AgentCommand, AppEvent, AppEventSender, Op};

pub(crate) enum Purpose {
    /// The session's model (`/model`).
    Session,
    /// A record of `collection`, named `PROVIDER/MODEL`.
    AddRecord { collection: String },
}

impl Purpose {
    pub(super) fn title(&self) -> &'static str {
        match self {
            Self::Session => "Select Model",
            Self::AddRecord { .. } => "Add model overrides",
        }
    }

    pub(super) fn subtitle(&self) -> &'static str {
        match self {
            Self::Session => "Switch the model of this session.",
            Self::AddRecord { .. } => "The model chosen gets its own settings.",
        }
    }

    /// What Enter does, and what Esc does, in the footer.
    pub(super) fn enter(&self) -> &'static str {
        match self {
            Self::Session => "choose",
            Self::AddRecord { .. } => "add",
        }
    }

    pub(super) fn esc(&self) -> &'static str {
        match self {
            Self::Session => "esc close",
            Self::AddRecord { .. } => "esc back",
        }
    }

    /// Whether a Provider and model can be typed in: one that is not
    /// listed is added as it is; the session's model has to be listed.
    pub(super) fn names_a_pair(&self) -> bool {
        matches!(self, Self::AddRecord { .. })
    }

    /// `provider_id`'s `model` taken: the session's, or the collection's
    /// new record.
    pub(super) fn choose(&self, tx: &AppEventSender, provider_id: &str, model: &str) {
        match self {
            Self::Session => tx.send(AppEvent::Op(Op::Command(AgentCommand::SetModel {
                provider_id: provider_id.to_string(),
                model: model.to_string(),
            }))),
            Self::AddRecord { collection } => tx.send(AppEvent::Workspace(
                WorkspaceEvent::SettingsAction(SettingsPageAction {
                    collection: collection.clone(),
                    record: Some(format!("{provider_id}/{model}")),
                    action: "add".to_string(),
                    values: Map::new(),
                }),
            )),
        }
    }
}
