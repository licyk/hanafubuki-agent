use hanafubuki_agent::{AgentModelProtocol, AgentProviderRegistrySnapshot};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::form::{self, BASE_URL, NAME};
use crate::agent_bridge::setup_needed;
use crate::bottom_pane::FormView;

fn key(form: &mut FormView, code: KeyCode) {
    form.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn type_text(form: &mut FormView, text: &str) {
    for character in text.chars() {
        key(form, KeyCode::Char(character));
    }
}

#[test]
fn the_setup_is_needed_without_a_default_provider_or_a_model() {
    let mut registry = AgentProviderRegistrySnapshot::default();
    assert!(setup_needed(&registry, false));
    assert!(setup_needed(&registry, true));
    registry.default_provider_id = Some("provider-1".into());
    assert!(setup_needed(&registry, false));
    // `-m` gives the sessions their model.
    assert!(!setup_needed(&registry, true));
    registry.model_policy.model = "gpt-5.1".into();
    assert!(!setup_needed(&registry, false));
}

#[test]
fn a_draft_needs_a_name_and_a_base_url() {
    let mut form = form::provider_form(None, true);
    assert_eq!(form::draft(&mut form), None);
    assert_eq!(form.error(NAME), Some("Enter a name."));
    assert!(form.error(BASE_URL).is_some());

    type_text(&mut form, " Gateway ");
    for _ in 0..2 {
        key(&mut form, KeyCode::Tab);
    }
    type_text(&mut form, "https://gateway.test/v1");
    // Enabled, then the headers' add stop: one header.
    for _ in 0..3 {
        key(&mut form, KeyCode::Tab);
    }
    key(&mut form, KeyCode::Enter);
    type_text(&mut form, "X-Team");
    key(&mut form, KeyCode::Tab);
    type_text(&mut form, "blue");
    let draft = form::draft(&mut form).unwrap();
    assert_eq!(form.error(NAME), None);
    assert_eq!(draft.display_name, "Gateway");
    assert_eq!(draft.base_url, "https://gateway.test/v1");
    assert_eq!(draft.protocol, AgentModelProtocol::Responses);
    assert!(draft.enabled);
    // No token typed is none.
    assert_eq!(draft.api_token, None);
    assert_eq!(draft.request_headers.len(), 1);
    assert_eq!(draft.request_headers[0].name, "X-Team");
    assert_eq!(draft.request_headers[0].value, "blue");
}
