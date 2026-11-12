use hanafubuki_agent::AgentProviderRegistrySnapshot;

use crate::agent_bridge::setup_needed;

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
