//! Plugins contribute per-step tool sources through the source registry; the
//! planner keeps each source's trust, plans its tools as handlers bound to the
//! source's entry and collects the leases they return.

use hanafubuki_agent_contracts::{
    Register, RegistrationError, RegistryService, ToolSourceRegistry,
};
use kardia_core::{Context, FiberState, Inject, PluginDef, Service};

use super::*;

/// A source that plans `tools` with `lease`, or fails as `trust` allows.
struct LeasingSource {
    id: &'static str,
    trust: ToolSourceTrust,
    tools: Vec<SourcedTool>,
    lease: Option<&'static str>,
    fails: bool,
}

impl AgentToolSource for LeasingSource {
    fn id(&self) -> &str {
        self.id
    }

    fn trust(&self) -> ToolSourceTrust {
        self.trust
    }

    fn contribute<'a>(
        &'a self,
        _step: &'a ToolSourceStep,
    ) -> ToolFuture<'a, Result<ToolSourceContribution, AgentToolError>> {
        Box::pin(async move {
            if self.fails {
                return Err(AgentToolError::Failed("offline".into()));
            }
            let contribution = ToolSourceContribution::from_tools(self.tools.clone());
            Ok(match self.lease {
                Some(lease) => contribution.with_lease(Arc::new(lease.to_string())),
                None => contribution,
            })
        })
    }
}

fn source(id: &'static str, tool: &str, lease: &'static str) -> Arc<dyn AgentToolSource> {
    Arc::new(LeasingSource {
        id,
        trust: ToolSourceTrust::Required,
        tools: vec![dynamic_tool(tool, ToolExposure::Direct)],
        lease: Some(lease),
        fails: false,
    })
}

fn lease_names(plan: &AgentToolPlan) -> Vec<String> {
    let mut names = plan
        .leases()
        .iter()
        .filter_map(|lease| lease.as_ref().as_any().downcast_ref::<String>().cloned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

async fn kernel_root() -> (Context, AgentKernelRegistriesFixture) {
    let registries = crate::AgentKernelRegistries::default();
    let fixture = AgentKernelRegistriesFixture {
        sources: registries.tool_sources.clone(),
    };
    let root = Context::new();
    root.plugin_def(&crate::kernel_plugin(registries), serde_json::Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    (root, fixture)
}

struct AgentKernelRegistriesFixture {
    sources: Arc<ToolSourceRegistry>,
}

fn sources_plugin(name: &'static str, sources: Vec<Arc<dyn AgentToolSource>>) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let sources = sources.clone();
        async move {
            let registry = ctx.get::<ToolSourceRegistry>()?;
            for source in sources {
                registry.register(source)?;
            }
            Ok(())
        }
    })
    .name(name)
    .inject(Inject::new().with(ToolSourceRegistry::NAME))
}

async fn plan_with_sources(sources: &ToolSourceRegistry) -> AgentToolPlan {
    AgentToolPlanner::default()
        .with_sources(sources.registry().snapshot())
        .build(
            plan_context(AgentSessionMode::Default, true),
            CancellationToken::default(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn registered_sources_are_planned_with_their_leases_until_their_plugin_stops() {
    let (root, kernel) = kernel_root().await;
    let fiber = root
        .plugin_def(
            &sources_plugin(
                "two-sources",
                vec![
                    source("first", "first_tool", "first-lease"),
                    source("second", "second_tool", "second-lease"),
                ],
            ),
            serde_json::Value::Null,
        )
        .unwrap();
    fiber.settled().await.unwrap();
    assert_eq!(fiber.state(), FiberState::Active);

    let plan = plan_with_sources(&kernel.sources).await;
    assert!(plan.resolve(None, "first_tool").is_ok());
    assert!(plan.resolve(None, "second_tool").is_ok());
    assert_eq!(lease_names(&plan), ["first-lease", "second-lease"]);

    fiber.dispose().await.unwrap();
    assert!(kernel.sources.registry().snapshot().is_empty());
    let plan = plan_with_sources(&kernel.sources).await;
    assert!(plan.resolve(None, "first_tool").is_err());
    assert!(plan.leases().is_empty());
    root.fiber().dispose().await.unwrap();
}

#[tokio::test]
async fn a_duplicate_source_id_fails_the_registering_plugin() {
    assert!(matches!(
        ToolSourceRegistry::admit(
            &[&source("same", "one", "one")],
            &source("same", "two", "two")
        ),
        Err(RegistrationError::Duplicate { identity, .. }) if identity == "same"
    ));

    let (root, kernel) = kernel_root().await;
    let fiber = root
        .plugin_def(
            &sources_plugin(
                "duplicate-sources",
                vec![source("same", "one", "one"), source("same", "two", "two")],
            ),
            serde_json::Value::Null,
        )
        .unwrap();
    let _ = fiber.settled().await;
    assert_ne!(fiber.state(), FiberState::Active);
    assert!(kernel.sources.registry().snapshot().is_empty());
    root.fiber().dispose().await.unwrap();
}

#[tokio::test]
async fn a_failed_optional_source_is_omitted_with_its_lease_and_a_required_one_fails_the_plan() {
    let failing = |trust| -> Arc<dyn AgentToolSource> {
        Arc::new(LeasingSource {
            id: "flaky",
            trust,
            tools: Vec::new(),
            lease: Some("flaky-lease"),
            fails: true,
        })
    };
    let planner = |sources: Vec<Arc<dyn AgentToolSource>>| {
        let contributors = sources
            .into_iter()
            .map(|source| {
                Arc::new(SourcedAgentToolContributor::new(detached_entry(
                    "sources", source,
                ))) as Arc<dyn AgentToolContributor>
            })
            .collect();
        AgentToolPlanner::new(contributors)
    };
    let optional = planner(vec![
        source("steady", "steady_tool", "steady-lease"),
        failing(ToolSourceTrust::Optional),
    ])
    .build(
        plan_context(AgentSessionMode::Default, true),
        CancellationToken::default(),
    )
    .await
    .unwrap();
    assert!(optional.resolve(None, "steady_tool").is_ok());
    assert_eq!(lease_names(&optional), ["steady-lease"]);
    assert_eq!(optional.omissions()[0].contributor, "flaky");

    let error = planner(vec![failing(ToolSourceTrust::Required)])
        .build(
            plan_context(AgentSessionMode::Default, true),
            CancellationToken::default(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, AgentToolCatalogError::ProviderFailed(message) if message.starts_with("flaky: "))
    );
}

/// Waits until its call is cancelled, as a source does when its plugin stops
/// in the middle of a slow contribution (an MCP catalog refresh).
struct StallingSource {
    entered: Arc<tokio::sync::Notify>,
}

impl AgentToolSource for StallingSource {
    fn id(&self) -> &str {
        "stalling"
    }

    fn contribute<'a>(
        &'a self,
        step: &'a ToolSourceStep,
    ) -> ToolFuture<'a, Result<ToolSourceContribution, AgentToolError>> {
        Box::pin(async move {
            self.entered.notify_one();
            while !step.cancellation.is_cancelled() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            Err(AgentToolError::Cancelled)
        })
    }
}

#[tokio::test]
async fn a_required_source_stopped_while_it_is_asked_is_omitted_not_fatal() {
    let (root, kernel) = kernel_root().await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let fiber = root
        .plugin_def(
            &sources_plugin(
                "stalling-sources",
                vec![
                    Arc::new(StallingSource {
                        entered: entered.clone(),
                    }),
                    source("steady", "steady_tool", "steady-lease"),
                ],
            ),
            serde_json::Value::Null,
        )
        .unwrap();
    fiber.settled().await.unwrap();
    let steady = root
        .plugin_def(
            &sources_plugin(
                "steady-sources",
                vec![source("kept", "kept_tool", "kept-lease")],
            ),
            serde_json::Value::Null,
        )
        .unwrap();
    steady.settled().await.unwrap();

    let snapshot = kernel.sources.registry().snapshot();
    let planning = tokio::spawn(async move {
        AgentToolPlanner::default()
            .with_sources(snapshot)
            .build(
                plan_context(AgentSessionMode::Default, true),
                CancellationToken::default(),
            )
            .await
    });
    entered.notified().await;
    // The user turns the plugin off while its source is being asked.
    fiber.dispose().await.unwrap();

    let plan = planning.await.unwrap().unwrap();
    assert!(plan.resolve(None, "kept_tool").is_ok());
    assert!(plan
        .omissions()
        .iter()
        .any(|omission| omission.contributor == "stalling"));
    root.fiber().dispose().await.unwrap();
}
