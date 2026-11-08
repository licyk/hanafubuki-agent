//! Context section kinds keep their recorded wire names; the context registry
//! admits one contributor per id and per kind, the renderer registry one
//! renderer per kind.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    ContextContributor, ContextFuture, ContextRegistry, ContextRenderer, ContextRendererRegistry,
    ContextRequest, ContextRole, ContextSection, ContextSectionKind, ContextSlot,
    RegistrationError, RegistryService,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use serde_json::json;

#[test]
fn kinds_travel_as_their_bare_name_and_reject_non_identifiers() {
    assert_eq!(
        serde_json::to_value(ContextSectionKind::WORKSPACE_ROOTS).unwrap(),
        json!("workspace_roots")
    );
    let known: ContextSectionKind = serde_json::from_value(json!("workspace_roots")).unwrap();
    assert_eq!(known, ContextSectionKind::WORKSPACE_ROOTS);
    let unknown: ContextSectionKind = serde_json::from_value(json!("plugin_notes")).unwrap();
    assert_eq!(unknown.as_str(), "plugin_notes");
    assert!(serde_json::from_value::<ContextSectionKind>(json!("Not A Kind")).is_err());
    assert!(ContextSectionKind::parse(&"a".repeat(ContextSectionKind::MAX_BYTES + 1)).is_none());
}

#[test]
fn json_sections_render_with_sorted_keys() {
    let section = ContextSection::from_json(
        ContextSectionKind::ENVIRONMENT,
        &json!({ "platform": "linux", "application": "Hanafubuki" }),
    );
    assert_eq!(
        section.content,
        "{\"application\":\"Hanafubuki\",\"platform\":\"linux\"}"
    );
}

struct Declares(&'static str, Vec<ContextSectionKind>);

impl ContextContributor for Declares {
    fn id(&self) -> &str {
        self.0
    }

    fn kinds(&self) -> &[ContextSectionKind] {
        &self.1
    }

    fn contribute<'a>(
        &'a self,
        _request: &'a ContextRequest,
        _cancellation: CancellationToken,
    ) -> ContextFuture<'a> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn contributor(id: &'static str, kinds: &[ContextSectionKind]) -> Arc<dyn ContextContributor> {
    Arc::new(Declares(id, kinds.to_vec()))
}

#[test]
fn the_registry_admits_one_contributor_per_id_and_per_kind() {
    let environment = contributor("environment", &[ContextSectionKind::ENVIRONMENT]);
    let existing = [&environment];

    assert!(ContextRegistry::admit(
        &existing,
        &contributor("roots", &[ContextSectionKind::WORKSPACE_ROOTS])
    )
    .is_ok());
    assert!(matches!(
        ContextRegistry::admit(
            &existing,
            &contributor("environment", &[ContextSectionKind::WORKSPACE_ROOTS])
        ),
        Err(RegistrationError::Duplicate { identity, .. }) if identity == "environment"
    ));
    assert!(matches!(
        ContextRegistry::admit(
            &existing,
            &contributor("other", &[ContextSectionKind::ENVIRONMENT])
        ),
        Err(RegistrationError::Duplicate { .. })
    ));
    assert!(matches!(
        ContextRegistry::admit(
            &[],
            &contributor("omissions", &[ContextSectionKind::TOOL_CATALOG_OMISSIONS])
        ),
        Err(RegistrationError::Rejected { .. })
    ));
    assert!(matches!(
        ContextRegistry::admit(
            &[],
            &contributor("invalid", &[ContextSectionKind::fixed("Not-Valid")])
        ),
        Err(RegistrationError::Rejected { .. })
    ));
}

fn renderer(kind: ContextSectionKind) -> ContextRenderer {
    ContextRenderer {
        kind,
        slot: ContextSlot::WorldState,
        role: ContextRole::Developer,
        preamble: "",
        epilogue: "",
        versioned: true,
    }
}

#[test]
fn the_renderer_registry_admits_one_renderer_per_kind() {
    let environment = renderer(ContextSectionKind::ENVIRONMENT);
    let existing = [&environment];

    assert!(ContextRendererRegistry::admit(
        &existing,
        &renderer(ContextSectionKind::WORKSPACE_ROOTS)
    )
    .is_ok());
    assert!(matches!(
        ContextRendererRegistry::admit(&existing, &renderer(ContextSectionKind::ENVIRONMENT)),
        Err(RegistrationError::Duplicate { .. })
    ));
    assert!(matches!(
        ContextRendererRegistry::admit(&[], &renderer(ContextSectionKind::fixed("Not-Valid"))),
        Err(RegistrationError::Rejected { .. })
    ));
}

#[test]
fn a_snapshot_finds_a_renderer_by_kind_with_its_registration_order() {
    let registry = ContextRendererRegistry::default();
    for kind in [
        ContextSectionKind::WORKSPACE_ROOTS,
        ContextSectionKind::ENVIRONMENT,
    ] {
        registry
            .registry()
            .register_permanent("test", renderer(kind), ContextRendererRegistry::admit)
            .unwrap();
    }
    let snapshot = registry.snapshot();
    let (position, found) = snapshot.find(&ContextSectionKind::ENVIRONMENT).unwrap();
    assert_eq!(position, 1);
    assert_eq!(found.kind, ContextSectionKind::ENVIRONMENT);
    assert!(snapshot
        .find(&ContextSectionKind::PERMISSION_MODE)
        .is_none());
}
