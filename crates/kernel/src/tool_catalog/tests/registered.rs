//! Registered tools are planned like builtins, with the availability and
//! effect their handler declares.

use hanafubuki_agent_approval::{ApprovalAction, AuthorizedAction, PermissionRequirement};
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryContextEffect, AgentToolError, AgentToolHandler, Register,
    RegistrationError, RegistryService, ToolAvailability, ToolDescriptor, ToolEffect, ToolFuture,
    ToolInvocation, ToolPlanContext, ToolRegistry, ToolResult,
};
use hanafubuki_agent_tool_contracts::{FreeformToolFormat, FreeformToolSpec, ToolExecutionClass};
use kardia_core::{Context, Inject, PluginDef, Service};

use super::*;

/// `freeform` when the provider supports it, absent without image input.
struct Fixture {
    descriptor: ToolDescriptor,
    needs_images: bool,
}

impl AgentToolHandler for Fixture {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn availability(&self, plan: &ToolPlanContext) -> ToolAvailability {
        if self.needs_images && !plan.image_input {
            return ToolAvailability::Omitted;
        }
        if plan.freeform_tools {
            return ToolAvailability::Freeform(FreeformToolSpec {
                name: self.descriptor.name.clone(),
                description: "freeform".to_string(),
                format: FreeformToolFormat {
                    format_type: "grammar".to_string(),
                    syntax: "lark".to_string(),
                    definition: "start: \"x\"".to_string(),
                },
            });
        }
        ToolAvailability::Function
    }

    fn permission<'a>(
        &'a self,
        _invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        Box::pin(async { Ok(PermissionRequirement::NotRequired) })
    }

    fn invoke(
        &self,
        _invocation: ToolInvocation,
        _approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async { Ok(ToolResult::success(json!({}))) })
    }
}

fn fixture(name: &str, effect: ToolEffect, needs_images: bool) -> Arc<dyn AgentToolHandler> {
    exposed_fixture(name, effect, needs_images, ToolExposure::Direct)
}

fn exposed_fixture(
    name: &str,
    effect: ToolEffect,
    needs_images: bool,
    exposure: ToolExposure,
) -> Arc<dyn AgentToolHandler> {
    let spec = spec(name);
    Arc::new(Fixture {
        descriptor: ToolDescriptor {
            name: ToolName::parse(spec.name).unwrap(),
            description: spec.description,
            input_schema: spec.input_schema,
            exposure,
            execution_class: ToolExecutionClass::Exclusive,
            effect,
            action: AgentActivityAction::USE_TOOL,
            memory_effect: AgentMemoryContextEffect::None,
            timeout: None,
        },
        needs_images,
    })
}

async fn registered_tools() -> (Context, Arc<ToolRegistry>) {
    let tools = Arc::new(ToolRegistry::default());
    let provided = tools.clone();
    let root = Context::new();
    let registry = PluginDef::new(move |ctx: Context, _| {
        let tools = provided.clone();
        async move {
            ctx.provide_arc(tools)?;
            Ok(())
        }
    })
    .provides([ToolRegistry::NAME]);
    root.plugin_def(&registry, serde_json::Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    let plugin = PluginDef::new(|ctx: Context, _| async move {
        let tools = ctx.get::<ToolRegistry>()?;
        tools.register(fixture("patch_like", ToolEffect::Mutating, false))?;
        tools.register(fixture("image_like", ToolEffect::ReadOnly, true))?;
        tools.register(exposed_fixture(
            "deferred_mutation",
            ToolEffect::Mutating,
            false,
            ToolExposure::Deferred,
        ))?;
        Ok(())
    })
    .inject(Inject::new().with(ToolRegistry::NAME));
    root.plugin_def(&plugin, serde_json::Value::Null)
        .unwrap()
        .settled()
        .await
        .unwrap();
    (root, tools)
}

async fn plan(tools: &ToolRegistry, context: AgentToolCatalogContext) -> AgentToolPlan {
    AgentToolPlanner::new(vec![Arc::new(RegisteredAgentToolContributor::new(
        tools.registry().snapshot(),
    ))])
    .build(context, CancellationToken::default())
    .await
    .unwrap()
}

#[tokio::test]
async fn availability_selects_the_spec_and_omits_unsupported_tools() {
    let (root, tools) = registered_tools().await;
    let mut context = plan_context(AgentSessionMode::Default, false);

    let function = plan(&tools, context.clone()).await;
    assert!(matches!(
        function.resolve(None, "patch_like").unwrap().provider_spec,
        ToolSpec::Function(_)
    ));
    assert!(function.resolve(None, "image_like").is_err());

    context.provider_capabilities.freeform_tools = true;
    context.input_capabilities.image_input = true;
    let freeform = plan(&tools, context).await;
    let patch_like = freeform.resolve(None, "patch_like").unwrap();
    assert!(matches!(patch_like.provider_spec, ToolSpec::Freeform(_)));
    assert_ne!(
        patch_like.generation,
        function.resolve(None, "patch_like").unwrap().generation,
        "the generation covers the provider spec"
    );
    assert!(freeform.resolve(None, "image_like").is_ok());
    root.fiber().dispose().await.unwrap();
}

#[tokio::test]
async fn plan_mode_keeps_only_read_only_registered_tools() {
    let (root, tools) = registered_tools().await;
    let mut context = plan_context(AgentSessionMode::Plan, false);
    context.input_capabilities.image_input = true;
    let planned = plan(&tools, context).await;
    assert!(planned.resolve(None, "patch_like").is_err());
    assert!(planned.resolve(None, "image_like").is_ok());
    root.fiber().dispose().await.unwrap();
}

#[tokio::test]
async fn a_deferred_mutation_is_searchable_by_default_and_hidden_in_plan_mode() {
    let (root, tools) = registered_tools().await;
    let default_plan = plan(&tools, plan_context(AgentSessionMode::Default, false)).await;
    assert!(default_plan
        .deferred_registrations()
        .any(|registration| registration.model_spec.name == "deferred_mutation"));
    assert!(default_plan
        .search_deferred("deferred mutation", 10)
        .iter()
        .any(|matched| matched.model_name == "deferred_mutation"));

    let plan_mode = plan(&tools, plan_context(AgentSessionMode::Plan, false)).await;
    assert!(plan_mode
        .search_deferred("deferred mutation", 10)
        .is_empty());
    assert!(plan_mode.resolve(None, "deferred_mutation").is_err());
    root.fiber().dispose().await.unwrap();
}

fn named_fixture(name: ToolName) -> Arc<dyn AgentToolHandler> {
    let fixture = fixture("unnamed", ToolEffect::ReadOnly, false);
    Arc::new(Fixture {
        descriptor: ToolDescriptor {
            name,
            ..fixture.descriptor().clone()
        },
        needs_images: false,
    })
}

/// The registry refuses what the planner would have to rename: a kernel
/// tool's name or `tool_search`, a name another tool's flattens to, and a
/// name too long to be a tool id.
#[test]
fn registration_refuses_reserved_taken_and_oversized_names() {
    for reserved in crate::AgentCoreTool::ALL
        .iter()
        .map(|tool| tool.name())
        .chain([hanafubuki_agent_model_api::FALLBACK_TOOL_SEARCH_NAME])
    {
        assert!(hanafubuki_agent_contracts::RESERVED_TOOL_NAMES.contains(&reserved));
        assert!(matches!(
            ToolRegistry::admit(&[], &named_fixture(ToolName::parse(reserved).unwrap())),
            Err(RegistrationError::Rejected { reason, .. }) if reason.contains("reserved")
        ));
    }
    assert_eq!(
        hanafubuki_agent_contracts::RESERVED_TOOL_NAMES.len(),
        crate::AgentCoreTool::ALL.len() + 1
    );
    let flat = named_fixture(ToolName::parse("calendar__create").unwrap());
    let namespaced =
        named_fixture(ToolName::namespaced(vec!["calendar".to_string()], "create").unwrap());
    assert!(ToolRegistry::admit(&[], &namespaced).is_ok());
    assert!(matches!(
        ToolRegistry::admit(&[&flat], &namespaced),
        Err(RegistrationError::Duplicate { identity, .. }) if identity == "calendar__create"
    ));
    let oversized =
        named_fixture(ToolName::namespaced(vec!["n".repeat(100)], "t".repeat(100)).unwrap());
    assert!(matches!(
        ToolRegistry::admit(&[], &oversized),
        Err(RegistrationError::Rejected { .. })
    ));
}
