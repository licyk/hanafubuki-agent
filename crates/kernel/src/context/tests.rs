//! A turn's world state comes from the registered contributors, ordered by
//! kind rank and checked against what each contributor declared.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    ContextContributor, ContextFuture, ContextSection, ContextSectionKind, Register, TurnExtensions,
};
use kardia_core::{Context, FiberState, Inject, PluginDef, Service};

use super::*;

struct Fixed {
    id: &'static str,
    kinds: Vec<ContextSectionKind>,
    produces: Result<Vec<ContextSection>, String>,
}

impl ContextContributor for Fixed {
    fn id(&self) -> &str {
        self.id
    }

    fn kinds(&self) -> &[ContextSectionKind] {
        &self.kinds
    }

    fn contribute<'a>(
        &'a self,
        _request: &'a ContextRequest,
        _cancellation: CancellationToken,
    ) -> ContextFuture<'a> {
        Box::pin(async move { self.produces.clone() })
    }
}

fn producing(id: &'static str, kinds: &[ContextSectionKind]) -> Arc<dyn ContextContributor> {
    Arc::new(Fixed {
        id,
        kinds: kinds.to_vec(),
        produces: Ok(kinds
            .iter()
            .map(|kind| ContextSection::new(kind.clone(), format!("{kind} text")))
            .collect()),
    })
}

fn request() -> ContextRequest {
    ContextRequest {
        session_id: crate::AgentSessionId::new(1),
        mode: crate::AgentSessionMode::Default,
        permission_mode: crate::AgentPermissionMode::Manual,
        user_message: None,
        turn: TurnExtensions::default(),
    }
}

/// A kernel root with `contributors` registered by one plugin, in order.
async fn registry_with(
    contributors: Vec<Arc<dyn ContextContributor>>,
) -> (
    Context,
    Arc<ContextRegistry>,
    Arc<hanafubuki_agent_contracts::PluginFaults>,
) {
    let registries = crate::AgentKernelRegistries::default();
    let faults = registries.faults.clone();
    let context = registries.context.clone();
    let root = Context::new();
    root.plugin_def(&crate::kernel_plugin(registries), serde_json::Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    let plugin = PluginDef::new(move |ctx: Context, _| {
        let contributors = contributors.clone();
        async move {
            let registry = ctx.get::<ContextRegistry>()?;
            for contributor in contributors {
                registry.register(contributor)?;
            }
            Ok(())
        }
    })
    .name("contributors")
    .inject(Inject::new().with(ContextRegistry::NAME));
    let fiber = root.plugin_def(&plugin, serde_json::Value::Null).unwrap();
    fiber.settled().await.unwrap();
    assert_eq!(fiber.state(), FiberState::Active);
    (root, context, faults)
}

fn unknown(name: &str) -> ContextSectionKind {
    ContextSectionKind::parse(name).unwrap()
}

#[tokio::test]
async fn sections_are_collected_in_kind_name_order_not_registration_order() {
    let (_root, registry, _faults) = registry_with(vec![
        producing("late", &[unknown("zeta"), ContextSectionKind::ENVIRONMENT]),
        producing("unknown", &[unknown("alpha")]),
        producing(
            "early",
            &[
                ContextSectionKind::PERMISSION_MODE,
                ContextSectionKind::WORKSPACE_INSTRUCTIONS,
            ],
        ),
        producing("host", &[unknown("host_location")]),
    ])
    .await;

    let snapshot = collect_world_state(
        &registry,
        &request(),
        &CancellationToken::default(),
        crate::FIRST_BATCH_HARD_LIMITS.plugin_call_deadline,
    )
    .await
    .unwrap();

    let kinds = snapshot
        .sections
        .iter()
        .map(|section| section.kind.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            "alpha",
            "environment",
            "host_location",
            "permission_mode",
            "workspace_instructions",
            "zeta",
        ]
    );
    // The version is the digest the kernel has always written.
    let environment = &snapshot.sections[1];
    assert_eq!(
        environment,
        &AgentWorldStateSection::new(
            ContextSectionKind::ENVIRONMENT,
            "environment text".to_string()
        )
    );
}

#[tokio::test]
async fn a_failing_contributor_or_section_is_left_out_and_its_plugin_shows_the_issue() {
    let (_root, undeclared, faults) = registry_with(vec![Arc::new(Fixed {
        id: "liar",
        kinds: vec![ContextSectionKind::ENVIRONMENT],
        produces: Ok(vec![
            ContextSection::new(ContextSectionKind::WORKSPACE_ROOTS, "roots".to_string()),
            ContextSection::new(ContextSectionKind::ENVIRONMENT, "env".to_string()),
        ]),
    })])
    .await;
    let snapshot = collect_world_state(
        &undeclared,
        &request(),
        &CancellationToken::default(),
        crate::FIRST_BATCH_HARD_LIMITS.plugin_call_deadline,
    )
    .await
    .unwrap();
    assert_eq!(
        snapshot
            .sections
            .iter()
            .map(|section| section.kind.clone())
            .collect::<Vec<_>>(),
        [ContextSectionKind::ENVIRONMENT]
    );
    assert_eq!(
        faults.issues("contributors"),
        ["context contributor liar: it produced the undeclared kind workspace_roots"]
    );

    let (_root, repeated, faults) = registry_with(vec![Arc::new(Fixed {
        id: "twice",
        kinds: vec![ContextSectionKind::ENVIRONMENT],
        produces: Ok(vec![
            ContextSection::new(ContextSectionKind::ENVIRONMENT, "a".to_string()),
            ContextSection::new(ContextSectionKind::ENVIRONMENT, "b".to_string()),
        ]),
    })])
    .await;
    let snapshot = collect_world_state(
        &repeated,
        &request(),
        &CancellationToken::default(),
        crate::FIRST_BATCH_HARD_LIMITS.plugin_call_deadline,
    )
    .await
    .unwrap();
    assert_eq!(snapshot.sections.len(), 1);
    assert_eq!(snapshot.sections[0].content, "a");
    assert!(faults.issues("contributors")[0].contains("environment was produced twice"));

    let (_root, failing, faults) = registry_with(vec![
        Arc::new(Fixed {
            id: "failing",
            kinds: vec![ContextSectionKind::ENVIRONMENT],
            produces: Err("source offline".to_string()),
        }),
        producing("roots", &[ContextSectionKind::WORKSPACE_ROOTS]),
    ])
    .await;
    let snapshot = collect_world_state(
        &failing,
        &request(),
        &CancellationToken::default(),
        crate::FIRST_BATCH_HARD_LIMITS.plugin_call_deadline,
    )
    .await
    .unwrap();
    assert_eq!(
        snapshot
            .sections
            .iter()
            .map(|section| section.kind.clone())
            .collect::<Vec<_>>(),
        [ContextSectionKind::WORKSPACE_ROOTS]
    );
    assert_eq!(
        faults.issues("contributors"),
        ["context contributor failing: source offline"]
    );
}

#[tokio::test]
async fn a_stopped_plugin_contributes_nothing() {
    let (root, registry, _faults) = registry_with(vec![producing(
        "environment",
        &[ContextSectionKind::ENVIRONMENT],
    )])
    .await;
    root.fiber().dispose().await.unwrap();

    let snapshot = collect_world_state(
        &registry,
        &request(),
        &CancellationToken::default(),
        crate::FIRST_BATCH_HARD_LIMITS.plugin_call_deadline,
    )
    .await
    .unwrap();

    assert!(snapshot.sections.is_empty());
}

#[tokio::test]
async fn the_built_in_kinds_never_use_up_the_plugin_kind_bound() {
    let plugin_kinds = (0..8)
        .map(|index| unknown(&format!("plugin_kind_{index}")))
        .collect::<Vec<_>>();
    let mut contributors = vec![producing(
        "built_in",
        &[
            ContextSectionKind::WORKSPACE_INSTRUCTIONS,
            ContextSectionKind::WORKSPACE_ROOTS,
            ContextSectionKind::PERMISSION_MODE,
            ContextSectionKind::ENVIRONMENT,
        ],
    )];
    contributors.push(producing("plugin", &plugin_kinds));
    let (_root, registry, faults) = registry_with(contributors).await;

    let snapshot = collect_world_state(
        &registry,
        &request(),
        &CancellationToken::default(),
        crate::FIRST_BATCH_HARD_LIMITS.plugin_call_deadline,
    )
    .await
    .unwrap();

    // Four built-in sections and seven plugin ones: the eighth plugin kind is
    // left out and reported, the turn is not.
    assert_eq!(snapshot.sections.len(), 11);
    assert!(!snapshot
        .sections
        .iter()
        .any(|section| section.kind == unknown("plugin_kind_7")));
    assert!(faults.issues("contributors")[0].contains("past the bound of 7 plugin kinds"));
}
