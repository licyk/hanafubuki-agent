//! Approval rules only tighten: the registry combines their verdicts
//! monotonically (deny > require user > no opinion), and the combined
//! verdict does not depend on the order the rules were registered in.

use std::sync::Arc;

use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalOrigin, ApprovalRiskClass,
};
use hanafubuki_agent_contracts::{
    ApprovalRule, ApprovalRuleRegistry, ApprovalRuleVerdict, Register, RegistrationError,
    RegistryService,
};
use kardia_core::{Context, FiberState, Inject, PluginDef, Service};
use serde_json::Value;

struct Fixed(&'static str, Option<ApprovalRuleVerdict>);

impl ApprovalRule for Fixed {
    fn id(&self) -> &str {
        self.0
    }

    fn evaluate(
        &self,
        _origin: Option<hanafubuki_agent_contracts::AgentSessionOrigin>,
        _action: &ApprovalAction,
    ) -> Option<ApprovalRuleVerdict> {
        self.1.clone()
    }
}

fn deny(reason: &str) -> Option<ApprovalRuleVerdict> {
    Some(ApprovalRuleVerdict::Deny {
        reason: reason.to_string(),
    })
}

fn action() -> ApprovalAction {
    ApprovalAction::from_canonical_bytes(
        ApprovalActionKind::DomainMutation,
        ApprovalRiskClass::High,
        ApprovalOrigin::Model,
        "start instance".to_string(),
        b"start",
    )
    .unwrap()
}

/// A registry whose rules were registered by one plugin each, in `rules`
/// order.
async fn registered(
    rules: &[(&'static str, Option<ApprovalRuleVerdict>)],
) -> (Context, Arc<ApprovalRuleRegistry>) {
    let root = Context::new();
    root.plugin_def(
        &PluginDef::new(|ctx: Context, _| async move {
            ctx.provide(ApprovalRuleRegistry::default())?;
            Ok(())
        })
        .provides([ApprovalRuleRegistry::NAME]),
        Value::Null,
    )
    .unwrap()
    .settled()
    .await
    .unwrap();
    for (id, verdict) in rules {
        let (id, verdict) = (*id, verdict.clone());
        let plugin = PluginDef::new(move |ctx: Context, _| {
            let verdict = verdict.clone();
            async move {
                let rules = ctx.get::<ApprovalRuleRegistry>()?;
                rules.register(Arc::new(Fixed(id, verdict)) as Arc<dyn ApprovalRule>)?;
                Ok(())
            }
        })
        .name(id)
        .inject(Inject::new().with(ApprovalRuleRegistry::NAME));
        let fiber = root.plugin_def(&plugin, Value::Null).unwrap();
        fiber.settled().await.unwrap();
        assert_eq!(fiber.state(), FiberState::Active);
    }
    let registry = root.get::<ApprovalRuleRegistry>().unwrap().arc().clone();
    (root, registry)
}

async fn verdict(
    rules: &[(&'static str, Option<ApprovalRuleVerdict>)],
) -> Option<ApprovalRuleVerdict> {
    let (root, registry) = registered(rules).await;
    let verdict = registry.snapshot().verdict(None, &action());
    root.fiber().dispose().await.unwrap();
    verdict
}

#[tokio::test]
async fn verdicts_combine_monotonically() {
    assert_eq!(verdict(&[]).await, None);
    assert_eq!(verdict(&[("silent", None)]).await, None);
    assert_eq!(
        verdict(&[
            ("silent", None),
            ("person", Some(ApprovalRuleVerdict::RequireUser))
        ])
        .await,
        Some(ApprovalRuleVerdict::RequireUser)
    );
    assert_eq!(
        verdict(&[
            ("person", Some(ApprovalRuleVerdict::RequireUser)),
            ("deny", deny("frozen")),
            ("silent", None),
        ])
        .await,
        deny("frozen")
    );
}

#[tokio::test]
async fn the_combined_verdict_does_not_depend_on_registration_order() {
    let rules = [
        ("person", Some(ApprovalRuleVerdict::RequireUser)),
        ("deny-b", deny("second reason")),
        ("silent", None),
        ("deny-a", deny("first reason")),
        ("deny-c", deny("first reason")),
    ];
    let expected = deny("first reason; second reason");
    assert_eq!(verdict(&rules).await, expected);
    let mut reversed = rules.to_vec();
    reversed.reverse();
    assert_eq!(verdict(&reversed).await, expected);
    let mut rotated = rules.to_vec();
    rotated.rotate_left(2);
    assert_eq!(verdict(&rotated).await, expected);
}

#[tokio::test]
async fn a_rule_leaves_with_its_plugin() {
    let (root, registry) = registered(&[("person", Some(ApprovalRuleVerdict::RequireUser))]).await;
    assert_eq!(
        registry.snapshot().verdict(None, &action()),
        Some(ApprovalRuleVerdict::RequireUser)
    );
    root.fiber().dispose().await.unwrap();
    assert!(registry.registry().snapshot().is_empty());
    assert_eq!(registry.snapshot().verdict(None, &action()), None);
}

#[test]
fn the_registry_admits_one_rule_per_id() {
    let existing: Arc<dyn ApprovalRule> = Arc::new(Fixed("rule", None));
    let other: Arc<dyn ApprovalRule> = Arc::new(Fixed("other", None));
    let same: Arc<dyn ApprovalRule> = Arc::new(Fixed("rule", None));
    assert!(ApprovalRuleRegistry::admit(&[&existing], &other).is_ok());
    assert!(matches!(
        ApprovalRuleRegistry::admit(&[&existing], &same),
        Err(RegistrationError::Duplicate { identity, .. }) if identity == "rule"
    ));
}
