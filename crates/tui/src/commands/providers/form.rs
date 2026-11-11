// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The Provider form (Codex's `app/provider/editor/form.rs`): one Provider
//! in full, its token shown as dots and kept as it is unless edited, and
//! its request headers as rows. Only what the write needs is checked here;
//! the Agent checks the rest and says why it refused.

use hanafubuki_agent::{
    AgentModelProtocol, AgentProviderDraft, AgentProviderHeaderSettings, AgentProviderSettings,
};

use crate::bottom_pane::{FieldId, FormItem, FormView, RowValue};

pub(super) const NAME: FieldId = "name";
pub(super) const PROTOCOL: FieldId = "protocol";
pub(super) const BASE_URL: FieldId = "base_url";
pub(super) const TOKEN: FieldId = "token";
pub(super) const ENABLED: FieldId = "enabled";
pub(super) const HEADERS: FieldId = "headers";
pub(super) const TEST: FieldId = "test";
pub(super) const SAVE: FieldId = "save";

const PROTOCOLS: [AgentModelProtocol; 3] = [
    AgentModelProtocol::Responses,
    AgentModelProtocol::ChatCompletions,
    AgentModelProtocol::AnthropicMessages,
];

/// The form over `provider`, or an empty one to add a Provider.
pub(super) fn provider_form(provider: Option<&AgentProviderSettings>, setup: bool) -> FormView {
    let protocol = provider.map_or(AgentModelProtocol::Responses, |p| p.protocol);
    let headers = provider
        .map(|provider| {
            provider
                .request_headers
                .iter()
                .map(|header| RowValue {
                    name: header.name.clone(),
                    value: header.value.clone(),
                    secret: false,
                })
                .collect()
        })
        .unwrap_or_default();
    let items = vec![
        FormItem::text(NAME, "Name", provider.map_or("", |p| &p.display_name))
            .placeholder("My gateway"),
        FormItem::choice(
            PROTOCOL,
            "Protocol",
            &PROTOCOLS.map(protocol_name),
            PROTOCOLS.iter().position(|p| *p == protocol).unwrap_or(0),
        ),
        FormItem::text(BASE_URL, "Base URL", provider.map_or("", |p| &p.base_url))
            .placeholder("https://api.example.com/v1"),
        FormItem::text(
            TOKEN,
            "API token",
            provider.and_then(|p| p.api_token.as_deref()).unwrap_or(""),
        )
        .secret()
        .placeholder("none"),
        FormItem::toggle(ENABLED, "Enabled", provider.is_none_or(|p| p.enabled)),
        FormItem::rows(HEADERS, "Request headers", headers, "Add a header"),
        FormItem::action(TEST, "Test: list the models"),
        FormItem::action(SAVE, "Save"),
    ];
    let title = match provider {
        Some(provider) => format!("Edit {}", provider.display_name),
        None if setup => "Set up a model Provider".to_string(),
        None => "Add a Provider".to_string(),
    };
    let form = FormView::new(title, items);
    if setup && provider.is_none() {
        form.with_subtitle(
            "The Agent needs a model Provider before the first message: its endpoint and token. Test it, then save. providers.toml is not layered; saving changes only this owner's file.",
        )
    } else {
        form.with_subtitle("providers.toml is not layered; saving changes only this owner's file.")
    }
}

/// The Provider the form describes; `None`, with the form saying why,
/// when a field the write needs is missing.
pub(super) fn draft(form: &mut FormView) -> Option<AgentProviderDraft> {
    form.clear_errors();
    let display_name = form.text(NAME).trim().to_string();
    let base_url = form.text(BASE_URL).trim().to_string();
    if base_url.is_empty() {
        form.set_error(
            BASE_URL,
            "Enter the base URL, such as https://api.openai.com/v1.",
        );
    }
    if display_name.is_empty() {
        form.set_error(NAME, "Enter a name.");
    }
    if display_name.is_empty() || base_url.is_empty() {
        return None;
    }
    let token = form.text(TOKEN).trim().to_string();
    Some(AgentProviderDraft {
        display_name,
        enabled: form.toggle(ENABLED),
        protocol: PROTOCOLS
            .get(form.choice(PROTOCOL))
            .copied()
            .unwrap_or(AgentModelProtocol::Responses),
        base_url,
        api_token: (!token.is_empty()).then_some(token),
        request_headers: form
            .rows(HEADERS)
            .into_iter()
            .map(|row| AgentProviderHeaderSettings {
                name: row.name,
                value: row.value,
            })
            .collect(),
    })
}

/// A protocol as the configuration spells it.
pub(super) fn protocol_name(protocol: AgentModelProtocol) -> &'static str {
    match protocol {
        AgentModelProtocol::Responses => "responses",
        AgentModelProtocol::ChatCompletions => "chat_completions",
        AgentModelProtocol::AnthropicMessages => "anthropic_messages",
    }
}
