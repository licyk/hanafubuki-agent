//! A panic in plugin code stays in that plugin: the registries that combine
//! entries themselves (observers, approval rules, prompt fragments) isolate
//! each entry, report the panic as a fault of its owner, and give the entry's
//! own error outcome. An approval rule that panics fails closed.

use std::sync::{Arc, Mutex};

use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalOrigin, ApprovalRiskClass,
};
use hanafubuki_agent_contracts::{
    AgentGeneration, AgentNotification, AgentObserver, AgentSessionId, AgentTurnId, ApprovalRule,
    ApprovalRuleRegistry, ApprovalRuleVerdict, ContextSectionKind, ExtensionPoint,
    ObserverRegistry, PluginFault, PluginFaults, PromptContributor, PromptContributorRegistry,
    PromptFragment, PromptRequest, Register, RegistryService, TurnExtensions,
};
use kardia_core::{Context, FiberState, Inject, PluginDef};
use serde_json::Value;

/// Faults reported so far.
fn recorded(faults: &PluginFaults) -> Arc<Mutex<Vec<PluginFault>>> {
    let reported = Arc::new(Mutex::new(Vec::new()));
    let sink = reported.clone();
    faults.set_handler(move |fault| sink.lock().unwrap().push(fault));
    reported
}

/// A root providing `registry`, and one plugin named `name` per entry that
/// registers `register(name)` in it.
async fn mounted<S: RegistryService>(
    registry: Arc<S>,
    entries: Vec<(&'static str, S::Item)>,
) -> Context
where
    S::Item: Clone,
{
    let root = Context::new();
    let provided = registry.clone();
    root.plugin_def(
        &PluginDef::new(move |ctx: Context, _| {
            let registry = provided.clone();
            async move {
                ctx.provide_arc(registry)?;
                Ok(())
            }
        })
        .provides([S::NAME]),
        Value::Null,
    )
    .unwrap()
    .settled()
    .await
    .unwrap();
    for (name, item) in entries {
        let plugin = PluginDef::new(move |ctx: Context, _| {
            let item = item.clone();
            async move {
                ctx.get::<S>()?.register(item)?;
                Ok(())
            }
        })
        .name(name)
        .inject(Inject::new().with(S::NAME));
        let fiber = root.plugin_def(&plugin, Value::Null).unwrap();
        fiber.settled().await.unwrap();
        assert_eq!(fiber.state(), FiberState::Active);
    }
    root
}

struct Observer {
    id: &'static str,
    panics: bool,
    seen: Mutex<usize>,
}

impl AgentObserver for Observer {
    fn id(&self) -> &str {
        self.id
    }

    fn observe(&self, _notification: &AgentNotification) {
        if self.panics {
            panic!("observer {} broke", self.id);
        }
        *self.seen.lock().unwrap() += 1;
    }
}

fn observer(id: &'static str, panics: bool) -> Arc<Observer> {
    Arc::new(Observer {
        id,
        panics,
        seen: Mutex::new(0),
    })
}

#[tokio::test]
async fn a_panicking_observer_neither_stops_the_others_nor_reaches_the_committer() {
    let faults = Arc::new(PluginFaults::default());
    let reported = recorded(&faults);
    let registry = Arc::new(ObserverRegistry::with_faults(faults));
    let (first, broken, last) = (
        observer("a.first", false),
        observer("b.broken", true),
        observer("c.last", false),
    );
    let _root = mounted(
        registry.clone(),
        vec![
            ("first", first.clone() as Arc<dyn AgentObserver>),
            ("broken", broken as Arc<dyn AgentObserver>),
            ("last", last.clone() as Arc<dyn AgentObserver>),
        ],
    )
    .await;

    registry.notify(&AgentNotification::TurnFinished {
        session_id: AgentSessionId::new(1),
        turn_id: AgentTurnId::new(1),
        generation: AgentGeneration::new(1),
    });

    assert_eq!(*first.seen.lock().unwrap(), 1);
    assert_eq!(*last.seen.lock().unwrap(), 1);
    assert_eq!(
        *reported.lock().unwrap(),
        vec![PluginFault {
            plugin: "broken".to_string(),
            point: ExtensionPoint::Observer.into(),
            message: "observer b.broken broke".to_string(),
        }]
    );
}

struct Rule {
    id: &'static str,
    verdict: Option<ApprovalRuleVerdict>,
    panics: bool,
}

impl ApprovalRule for Rule {
    fn id(&self) -> &str {
        self.id
    }

    fn evaluate(
        &self,
        _origin: Option<hanafubuki_agent_contracts::AgentSessionOrigin>,
        _action: &ApprovalAction,
    ) -> Option<ApprovalRuleVerdict> {
        if self.panics {
            panic!("rule {} broke", self.id);
        }
        self.verdict.clone()
    }
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

#[tokio::test]
async fn a_panicking_approval_rule_denies_the_action() {
    let faults = Arc::new(PluginFaults::default());
    let reported = recorded(&faults);
    let registry = Arc::new(ApprovalRuleRegistry::with_faults(faults));
    let rule = |id, verdict, panics| {
        Arc::new(Rule {
            id,
            verdict,
            panics,
        }) as Arc<dyn ApprovalRule>
    };
    let _root = mounted(
        registry.clone(),
        vec![
            ("quiet", rule("quiet", None, false)),
            (
                "strict",
                rule("strict", Some(ApprovalRuleVerdict::RequireUser), false),
            ),
            ("broken", rule("broken", None, true)),
        ],
    )
    .await;

    assert_eq!(
        registry.snapshot().verdict(None, &action()),
        Some(ApprovalRuleVerdict::Deny {
            reason: "an approval rule of the broken plugin failed with an internal error"
                .to_string(),
        })
    );
    assert_eq!(reported.lock().unwrap().len(), 1);
    assert_eq!(
        reported.lock().unwrap()[0].point,
        ExtensionPoint::ApprovalRule.into()
    );
}

struct Contributor {
    id: &'static str,
    panics: bool,
}

impl PromptContributor for Contributor {
    fn id(&self) -> &str {
        self.id
    }

    fn rank(&self) -> u32 {
        0
    }

    fn contribute(&self, _request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        if self.panics {
            panic!("contributor {} broke", self.id);
        }
        vec![PromptFragment {
            kind: ContextSectionKind::fixed("skill_catalog"),
            version: None,
            content: self.id.to_string(),
        }]
    }
}

#[tokio::test]
async fn a_panicking_prompt_contributor_contributes_nothing() {
    let faults = Arc::new(PluginFaults::default());
    let reported = recorded(&faults);
    let registry = Arc::new(PromptContributorRegistry::with_faults(faults));
    let contributor =
        |id, panics| Arc::new(Contributor { id, panics }) as Arc<dyn PromptContributor>;
    let _root = mounted(
        registry.clone(),
        vec![
            ("broken", contributor("a.broken", true)),
            ("healthy", contributor("b.healthy", false)),
        ],
    )
    .await;

    let fragments = registry.snapshot().fragments(&PromptRequest {
        turn: &TurnExtensions::default(),
        prepared: None,
        context_window_tokens: None,
    });

    assert_eq!(
        fragments
            .iter()
            .map(|fragment| fragment.content.as_str())
            .collect::<Vec<_>>(),
        ["b.healthy"]
    );
    assert_eq!(reported.lock().unwrap()[0].plugin, "broken");
    assert_eq!(
        reported.lock().unwrap()[0].point,
        ExtensionPoint::PromptContribute.into()
    );
}

/// Each change to a plugin's issues is announced once; recording what is
/// already recorded announces nothing, so a contributor failing the same way
/// every turn, or succeeding every turn, raises no stream of notices.
#[test]
fn an_issue_change_is_announced_once_and_a_repeat_not_at_all() {
    let faults = PluginFaults::default();
    let announced = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = announced.clone();
    faults.set_issues_listener(move || {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    let count = || announced.load(std::sync::atomic::Ordering::SeqCst);

    faults.set_issue("memory", "context contributor memory", None);
    assert_eq!(count(), 0, "clearing an absent issue changes nothing");

    faults.set_issue(
        "memory",
        "context contributor memory",
        Some("offline".into()),
    );
    assert_eq!(count(), 1);
    faults.set_issue(
        "memory",
        "context contributor memory",
        Some("offline".into()),
    );
    assert_eq!(count(), 1, "the same issue again is not a change");
    faults.set_issue(
        "memory",
        "context contributor memory",
        Some("timed out".into()),
    );
    assert_eq!(count(), 2, "a different issue is");

    faults.set_issue("memory", "context contributor memory", None);
    assert_eq!(count(), 3);
    faults.set_issue("memory", "context contributor memory", None);
    assert_eq!(count(), 3);

    faults.set_issue(
        "memory",
        "context contributor memory",
        Some("offline".into()),
    );
    assert_eq!(count(), 4);
    faults.clear_issues("memory");
    assert_eq!(count(), 5);
    assert!(faults.issues("memory").is_empty());
    faults.clear_issues("memory");
    assert_eq!(count(), 5, "a plugin with no issues has nothing to clear");

    faults.clear_handler();
    faults.set_issue(
        "memory",
        "context contributor memory",
        Some("offline".into()),
    );
    assert_eq!(count(), 5, "a cleared listener hears nothing");
}
