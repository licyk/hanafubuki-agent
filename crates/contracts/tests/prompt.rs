//! Prompt contributors and turn admission contributors are unique by id and
//! ordered by `(rank, id)`, never by registration order.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    AgentSessionId, AgentSessionMode, ContextSectionKind, PromptContributor,
    PromptContributorRegistry, PromptFragment, PromptRequest, Register, TurnAdmissionContributor,
    TurnAdmissionFuture, TurnAdmissionRegistry, TurnAdmissionRequest, TurnExtensions,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use kardia_core::{Context, FiberState, Inject, PluginDef, Service};
use serde_json::Value;

struct Named {
    id: &'static str,
    rank: u32,
}

impl PromptContributor for Named {
    fn id(&self) -> &str {
        self.id
    }

    fn rank(&self) -> u32 {
        self.rank
    }

    fn contribute(&self, request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        vec![PromptFragment {
            kind: ContextSectionKind::fixed("skill_catalog"),
            version: None,
            content: format!("{} {:?}", self.id, request.context_window_tokens),
        }]
    }
}

impl TurnAdmissionContributor for Named {
    fn id(&self) -> &str {
        self.id
    }

    fn rank(&self) -> u32 {
        self.rank
    }

    fn contribute<'a>(
        &'a self,
        _request: &'a TurnAdmissionRequest,
        _cancellation: CancellationToken,
    ) -> TurnAdmissionFuture<'a> {
        Box::pin(async { Ok(TurnExtensions::default()) })
    }
}

/// A root providing both registries, and one plugin per entry of
/// `contributors` registering it in both, in order. Returns the registries
/// and each plugin's final state.
async fn registered(
    contributors: &[(&'static str, u32)],
) -> (
    Context,
    Arc<PromptContributorRegistry>,
    Arc<TurnAdmissionRegistry>,
    Vec<FiberState>,
) {
    let root = Context::new();
    root.plugin_def(
        &PluginDef::new(|ctx: Context, _| async move {
            ctx.provide(PromptContributorRegistry::default())?;
            ctx.provide(TurnAdmissionRegistry::default())?;
            Ok(())
        })
        .provides([PromptContributorRegistry::NAME, TurnAdmissionRegistry::NAME]),
        Value::Null,
    )
    .unwrap()
    .settled()
    .await
    .unwrap();
    let mut states = Vec::new();
    for (index, (id, rank)) in contributors.iter().copied().enumerate() {
        let plugin = PluginDef::new(move |ctx: Context, _| async move {
            let named = Arc::new(Named { id, rank });
            ctx.get::<PromptContributorRegistry>()?
                .register(named.clone() as Arc<dyn PromptContributor>)?;
            ctx.get::<TurnAdmissionRegistry>()?
                .register(named as Arc<dyn TurnAdmissionContributor>)?;
            Ok(())
        })
        .name(format!("contributor-{index}"))
        .inject(
            Inject::new()
                .with(PromptContributorRegistry::NAME)
                .with(TurnAdmissionRegistry::NAME),
        );
        let fiber = root.plugin_def(&plugin, Value::Null).unwrap();
        // A refused registration fails the plugin, which `settled` reports.
        let _ = fiber.settled().await;
        states.push(fiber.state());
    }
    let prompt = root
        .get::<PromptContributorRegistry>()
        .unwrap()
        .arc()
        .clone();
    let admission = root.get::<TurnAdmissionRegistry>().unwrap().arc().clone();
    (root, prompt, admission, states)
}

#[tokio::test]
async fn contributors_follow_rank_then_id_and_ids_are_unique() {
    let (_root, prompt, admission, states) =
        registered(&[("zeta", 1), ("beta", 2), ("alpha", 2), ("beta", 0)]).await;
    assert_eq!(
        states,
        [
            FiberState::Active,
            FiberState::Active,
            FiberState::Active,
            FiberState::Failed
        ]
    );

    let turn = TurnExtensions::default();
    let fragments = prompt.snapshot().fragments(&PromptRequest {
        turn: &turn,
        prepared: None,
        context_window_tokens: Some(8),
    });
    let contents = fragments
        .iter()
        .map(|fragment| fragment.content.as_str())
        .collect::<Vec<_>>();
    assert_eq!(contents, ["zeta Some(8)", "alpha Some(8)", "beta Some(8)"]);

    let ids = admission
        .ordered()
        .iter()
        .map(|entry| entry.item().id().to_string())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["zeta", "alpha", "beta"]);
    let request = TurnAdmissionRequest {
        session_id: AgentSessionId::new(1),
        mode: AgentSessionMode::Default,
    };
    let contributed = admission.ordered()[0]
        .item()
        .contribute(&request, CancellationToken::default())
        .await
        .unwrap();
    assert!(contributed.get::<u8>().is_none());
}
