//! A turn's extensions merge every contributor's values; a type returned
//! twice or a contributor error leaves that contributor out.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    Register, TurnAdmissionContributor, TurnAdmissionFuture, TurnAdmissionRequest,
};
use kardia_core::{Context, FiberState, Inject, PluginDef, Service};

use super::*;

#[derive(Debug, PartialEq)]
struct Catalog(&'static str);

#[derive(Debug, PartialEq)]
struct Names(Vec<&'static str>);

enum Produces {
    Catalog(&'static str),
    Names,
    Failure,
}

struct Fixed {
    id: &'static str,
    produces: Produces,
}

impl TurnAdmissionContributor for Fixed {
    fn id(&self) -> &str {
        self.id
    }

    fn rank(&self) -> u32 {
        0
    }

    fn contribute<'a>(
        &'a self,
        _request: &'a TurnAdmissionRequest,
        _cancellation: CancellationToken,
    ) -> TurnAdmissionFuture<'a> {
        Box::pin(async move {
            match self.produces {
                Produces::Catalog(name) => {
                    Ok(TurnExtensions::default().with(Arc::new(Catalog(name))))
                }
                Produces::Names => Ok(TurnExtensions::default().with(Arc::new(Names(vec!["a"])))),
                Produces::Failure => Err("catalog offline".to_string()),
            }
        })
    }
}

fn request() -> TurnAdmissionRequest {
    TurnAdmissionRequest {
        session_id: crate::AgentSessionId::new(1),
        mode: crate::AgentSessionMode::Default,
    }
}

/// A kernel root with `contributors` registered by one plugin, in order.
async fn registry_with(
    contributors: Vec<Fixed>,
) -> (
    Context,
    Arc<TurnAdmissionRegistry>,
    Arc<hanafubuki_agent_contracts::PluginFaults>,
) {
    let registries = crate::AgentKernelRegistries::default();
    let faults = registries.faults.clone();
    let admission = registries.turn_admission.clone();
    let root = Context::new();
    root.plugin_def(&crate::kernel_plugin(registries), serde_json::Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    let contributors = Arc::new(std::sync::Mutex::new(Some(contributors)));
    let plugin = PluginDef::new(move |ctx: Context, _| {
        let contributors = contributors.lock().unwrap().take().unwrap_or_default();
        async move {
            let registry = ctx.get::<TurnAdmissionRegistry>()?;
            for contributor in contributors {
                registry.register(Arc::new(contributor) as Arc<dyn TurnAdmissionContributor>)?;
            }
            Ok(())
        }
    })
    .name("contributors")
    .inject(Inject::new().with(TurnAdmissionRegistry::NAME));
    let fiber = root.plugin_def(&plugin, serde_json::Value::Null).unwrap();
    fiber.settled().await.unwrap();
    assert_eq!(fiber.state(), FiberState::Active);
    (root, admission, faults)
}

#[tokio::test]
async fn every_contributor_adds_its_values_to_the_turn() {
    let (_root, registry, _faults) = registry_with(vec![
        Fixed {
            id: "names",
            produces: Produces::Names,
        },
        Fixed {
            id: "catalog",
            produces: Produces::Catalog("frozen"),
        },
    ])
    .await;

    let turn = collect_turn_extensions(&registry, &request())
        .await
        .unwrap();

    assert_eq!(turn.get::<Catalog>(), Some(&Catalog("frozen")));
    assert_eq!(turn.get::<Names>(), Some(&Names(vec!["a"])));
}

#[tokio::test]
async fn a_type_produced_twice_or_a_contributor_error_is_left_out_and_shown() {
    let (_root, twice, faults) = registry_with(vec![
        Fixed {
            id: "first",
            produces: Produces::Catalog("first"),
        },
        Fixed {
            id: "second",
            produces: Produces::Catalog("second"),
        },
    ])
    .await;
    let turn = collect_turn_extensions(&twice, &request()).await.unwrap();
    assert_eq!(turn.get::<Catalog>(), Some(&Catalog("first")));
    assert_eq!(
        faults.issues("contributors"),
        ["turn admission contributor second: it produced a value another contributor produced"]
    );

    let (_root, failing, faults) = registry_with(vec![
        Fixed {
            id: "failing",
            produces: Produces::Failure,
        },
        Fixed {
            id: "names",
            produces: Produces::Names,
        },
    ])
    .await;
    let turn = collect_turn_extensions(&failing, &request()).await.unwrap();
    assert_eq!(turn.get::<Catalog>(), None);
    assert_eq!(turn.get::<Names>(), Some(&Names(vec!["a"])));
    assert_eq!(
        faults.issues("contributors"),
        ["turn admission contributor failing: catalog offline"]
    );
}

#[tokio::test]
async fn a_stopped_plugin_contributes_nothing() {
    let (root, registry, _faults) = registry_with(vec![Fixed {
        id: "catalog",
        produces: Produces::Catalog("frozen"),
    }])
    .await;
    root.fiber().dispose().await.unwrap();

    let turn = collect_turn_extensions(&registry, &request())
        .await
        .unwrap();

    assert_eq!(turn.get::<Catalog>(), None);
}
