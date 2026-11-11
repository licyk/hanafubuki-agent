// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The screen's last step: the models of the Provider just saved (or of
//! the default one, when only the model is missing), one of which, or an
//! id typed in, becomes the model new sessions run on it.

use hanafubuki_agent::ModelCatalogEntry;
use ratatui::crossterm::event::KeyEvent;
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::{ProvidersScreen, Stage};
use crate::agent_bridge::ProviderCall;
use crate::app_event::AppEvent;
use crate::bottom_pane::{
    BottomPaneView, FormEvent, FormItem, FormView, ListSelectionView, SelectionItem,
    SelectionViewParams,
};
use crate::history_cell;
use crate::model_facts::entry_badges;

const MODEL: &str = "model";
const USE_MODEL: &str = "use";

impl ProvidersScreen {
    /// The models of `provider_id` are listed; the picker shows them.
    pub(super) fn open_models(&mut self, provider_id: String) {
        let list = self.models_list(&provider_id, Err("Listing the models…".to_string()));
        self.send(ProviderCall::ListModels {
            provider_id: provider_id.clone(),
        });
        self.stage = Stage::Models {
            list,
            provider_id,
            models: Vec::new(),
            busy: false,
        };
    }

    fn models_list(
        &self,
        provider_id: &str,
        result: Result<&[ModelCatalogEntry], String>,
    ) -> ListSelectionView {
        let current = self
            .registry
            .as_ref()
            .filter(|registry| registry.default_provider_id.as_deref() == Some(provider_id))
            .map(|registry| registry.model_policy.model.clone());
        let mut items = Vec::new();
        let note = match result {
            Ok(models) => {
                items.extend(models.iter().map(|model| SelectionItem {
                    name: model.id.clone(),
                    description: describe(model),
                    is_current: current.as_deref() == Some(model.id.as_str()),
                    search_value: Some(model.id.clone()),
                    ..Default::default()
                }));
                None
            }
            Err(note) => Some(Line::from(note.dim())),
        };
        let selected = items.iter().position(|item| item.is_current);
        items.push(SelectionItem {
            name: "Enter a model id…".to_string(),
            description: Some("one the Provider does not list".to_string()),
            ..Default::default()
        });
        let name = self
            .provider(provider_id)
            .map_or(provider_id, |provider| provider.display_name.as_str());
        ListSelectionView::new(
            SelectionViewParams {
                title: Some("Choose the model".to_string()),
                subtitle: Some(format!(
                    "{name} becomes the default; new sessions run this model."
                )),
                footer_note: note,
                footer_hint: Some(Line::from("enter choose · esc back")),
                items,
                is_searchable: true,
                search_placeholder: Some("Type to search models".to_string()),
                initial_selected_idx: selected,
                ..Default::default()
            },
            self.tx.clone(),
        )
    }

    pub(super) fn handle_models_key(&mut self, key: KeyEvent) {
        let expected_revision = self.revision();
        let Stage::Models {
            list,
            provider_id,
            models,
            busy,
        } = &mut self.stage
        else {
            return;
        };
        if *busy {
            return;
        }
        list.handle_key_event(key);
        let provider_id = provider_id.clone();
        match list.take_last_selected_index() {
            Some(index) => match models.get(index).cloned() {
                Some(model) => {
                    *busy = true;
                    self.send(ProviderCall::SetDefault {
                        expected_revision,
                        provider_id,
                        model: Some(model),
                    });
                }
                None => {
                    let form = FormView::new(
                        "Enter a model id",
                        vec![
                            FormItem::text(MODEL, "Model", "").placeholder("gpt-5.1"),
                            FormItem::action(USE_MODEL, "Use this model"),
                        ],
                    );
                    self.stage = Stage::ModelEntry { form, provider_id };
                }
            },
            None if list.is_complete() => self.show_list(None, None),
            None => {}
        }
    }

    pub(super) fn handle_model_entry_key(&mut self, key: KeyEvent) {
        let expected_revision = self.revision();
        let Stage::ModelEntry { form, provider_id } = &mut self.stage else {
            return;
        };
        match form.handle_key(key) {
            FormEvent::Cancel => {
                let provider_id = provider_id.clone();
                self.open_models(provider_id);
            }
            FormEvent::Action(_) => {
                let model = form.text(MODEL).trim().to_string();
                if model.is_empty() {
                    form.set_error(MODEL, "Enter the model's id.");
                    return;
                }
                form.set_status(vec!["Saving…".dim().into()]);
                let provider_id = provider_id.clone();
                self.send(ProviderCall::SetDefault {
                    expected_revision,
                    provider_id,
                    model: Some(model),
                });
            }
            FormEvent::None => {}
        }
    }

    pub(super) fn models_listed(
        &mut self,
        listed: String,
        result: Result<Vec<ModelCatalogEntry>, String>,
    ) {
        let Stage::Models { provider_id, .. } = &self.stage else {
            return;
        };
        if *provider_id != listed {
            return;
        }
        let models = result
            .as_ref()
            .map(|models| models.iter().map(|model| model.id.clone()).collect())
            .unwrap_or_default();
        let list = self.models_list(
            &listed,
            result
                .as_deref()
                .map_err(|error| format!("The models were not listed: {error}")),
        );
        self.stage = Stage::Models {
            list,
            provider_id: listed,
            models,
            busy: false,
        };
    }

    /// The model policy was written: the setup is done.
    pub(super) fn model_chosen(&mut self, provider_id: &str) {
        let model = self
            .registry
            .as_ref()
            .map(|registry| registry.model_policy.model.clone())
            .unwrap_or_default();
        let name = self
            .provider(provider_id)
            .map_or(provider_id.to_string(), |p| p.display_name.clone());
        let next = if self.setup {
            " Your next message starts a session."
        } else {
            ""
        };
        self.tx.send(AppEvent::InsertHistoryCell(Box::new(
            history_cell::new_info_event(
                format!("New sessions run {model} on {name}.{next}"),
                None,
            ),
        )));
        if self.setup {
            self.complete = true;
        } else {
            self.show_list(None, None);
        }
    }
}

/// An entry's name, if it has one, and what it reports of itself.
fn describe(model: &ModelCatalogEntry) -> Option<String> {
    let parts: Vec<String> = model
        .display_name
        .iter()
        .cloned()
        .chain(entry_badges(model))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}
