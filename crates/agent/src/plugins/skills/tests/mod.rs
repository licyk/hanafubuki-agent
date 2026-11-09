//! The Skill tools' contracts: arguments, approval classification, the
//! model-visible specs and the plugin's registrations.
//!
//! `kernel_reference.json` was recorded from the kernel's closed tool set
//! before these tools moved here. The specs, exposure, Plan-mode
//! availability and approval actions (digests included) must stay
//! byte-identical to it; the two descriptions changed on purpose since
//! (Skill directories and the single delivery of `SKILL.md`).

use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalOrigin, ApprovalRiskClass,
};
use hanafubuki_agent_contracts::{
    AgentSessionMode, ApprovalRuleVerdict, RegistryService, ToolCallContext,
    ToolCallValidationError, ToolEffect,
};
use hanafubuki_agent_tool_contracts::ToolExecutionClass;
use serde_json::{json, Value};

use super::arguments::{parse, SkillCall};
use super::manage::capability_management_error;
use super::permission::skill_approval_action;
use super::specs::spec;
use super::{skills_plugin, SkillToolKind};
use crate::plugins::tests::fixtures::disconnected_mcp_application;
use crate::skills::AgentSkillApplication;
use crate::test_support::mount;
use crate::SkillRoots;

mod arguments;

const KERNEL_REFERENCE: &str = include_str!("kernel_reference.json");

fn kernel_reference() -> Value {
    serde_json::from_str(KERNEL_REFERENCE).unwrap()
}

fn kind(name: &str) -> SkillToolKind {
    SkillToolKind::ALL
        .into_iter()
        .find(|kind| kind.name() == name)
        .unwrap_or_else(|| panic!("{name} is a Skill tool"))
}

fn parse_call(name: &str, arguments: Value) -> Result<SkillCall, ToolCallValidationError> {
    parse(kind(name), &arguments)
}

fn reference_context(reference: &Value) -> ToolCallContext {
    let context = &reference["context"];
    ToolCallContext {
        session_id: serde_json::from_value(context["sessionId"].clone()).unwrap(),
        turn_id: serde_json::from_value(json!(1)).unwrap(),
        generation: serde_json::from_value(context["generation"].clone()).unwrap(),
        mode: AgentSessionMode::Default,
        session_revision: serde_json::from_value(json!(1)).unwrap(),
        call_id: context["callId"].as_str().unwrap().to_string(),
        image_input: false,
    }
}

fn empty_skills(name: &str) -> (AgentSkillApplication, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "hanafubuki-skills-plugin-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let skills = AgentSkillApplication::open(
        SkillRoots {
            system: root.join("system"),
            user: root.join("user"),
            install_bundled_system_skills: false,
            state_file: root.join("skills-state.json"),
        },
        None,
    );
    (skills, root)
}

#[test]
fn specs_match_the_kernel_reference() {
    let reference = kernel_reference();
    let tools = reference["tools"].as_object().unwrap();
    assert_eq!(tools.len(), SkillToolKind::ALL.len());
    for kind in SkillToolKind::ALL {
        let (description, input_schema) = spec(kind);
        let expected = &tools[kind.name()];
        assert_eq!(description, expected["description"], "{}", kind.name());
        assert_eq!(input_schema, expected["inputSchema"], "{}", kind.name());
    }
}

#[test]
fn approval_actions_match_the_kernel_reference() {
    let reference = kernel_reference();
    let context = reference_context(&reference);
    let cases = reference["permissions"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let name = case["tool"].as_str().unwrap();
        let call = parse_call(name, case["arguments"].clone()).unwrap();
        let actual = match skill_approval_action(&context, &call).unwrap() {
            None => json!("not_required"),
            Some(action) => serde_json::to_value(action).unwrap(),
        };
        assert_eq!(actual, case["permission"], "{name} {}", case["arguments"]);
    }
}

#[test]
fn a_skill_change_is_bound_to_its_call() {
    let context = reference_context(&kernel_reference());
    let action = |context: &ToolCallContext, name: &str| -> ApprovalAction {
        let call = parse_call(name, json!({ "skill": "user:pdf-tools" })).unwrap();
        skill_approval_action(context, &call).unwrap().unwrap()
    };
    let enable = action(&context, "enable_skill");
    assert_ne!(enable.digest, action(&context, "disable_skill").digest);
    let mut other_call = context.clone();
    other_call.call_id = "other-call".to_string();
    assert_ne!(enable.digest, action(&other_call, "enable_skill").digest);
}

#[test]
fn refused_skill_changes_keep_a_typed_outcome() {
    let status = |reason: &str| {
        capability_management_error(reason.to_owned())
            .output
            .structured_content
            .expect("typed management error must expose structured content")["status"]
            .as_str()
            .expect("typed management error must expose status")
            .to_owned()
    };
    assert_eq!(status("stale settings revision"), "stale");
    assert_eq!(status("server generation changed"), "stale");
    assert_eq!(status("cleanup remains unconfirmed"), "unknown_outcome");
    assert_eq!(status("server is still referenced"), "blocked");
    assert_eq!(status("transport request failed"), "failed");
}

/// A Skill package is authored, installed, and removed with the ordinary
/// filesystem and execution tools, exactly as Codex does it, so the plugin
/// registers the four catalog tools and nothing Skill-shaped beside them.
#[tokio::test]
async fn registers_the_skill_tools_with_their_kernel_descriptors_until_stopped() {
    let (skills, root) = empty_skills("registration");
    let mounted = mount(skills_plugin(
        skills,
        disconnected_mcp_application(root.join("mcp-settings.json")),
    ))
    .await;

    let reference = kernel_reference();
    let expected = reference["tools"].as_object().unwrap();
    let snapshot = mounted.tools.registry().snapshot();
    let registered = snapshot.items().collect::<Vec<_>>();
    assert_eq!(registered.len(), expected.len());
    for tool in registered {
        let descriptor = tool.descriptor();
        let name = descriptor.name.qualified();
        let reference = &expected[&name];
        assert_eq!(
            serde_json::to_value(descriptor.exposure).unwrap(),
            reference["exposure"],
            "{name}"
        );
        // Plan mode offers exactly the read-only tools.
        assert_eq!(
            descriptor.effect == ToolEffect::ReadOnly,
            reference["allowedInPlanMode"] == json!(true),
            "{name}"
        );
        // Reading the frozen catalog runs concurrently; Skill management
        // serializes with the session's other exclusive tools.
        assert_eq!(
            descriptor.execution_class == ToolExecutionClass::ConcurrentRead,
            descriptor.effect == ToolEffect::ReadOnly,
            "{name}"
        );
        assert_eq!(descriptor.action.as_str(), kind(&name).action());
    }

    // The turn's catalog and its listing come from the same plugin.
    assert_eq!(mounted.admission.registry().snapshot().len(), 1);
    assert_eq!(mounted.prompt.registry().snapshot().len(), 1);
    assert_eq!(mounted.renderers.registry().snapshot().len(), 1);

    mounted.fiber.dispose().await.unwrap();
    assert!(mounted.tools.registry().snapshot().is_empty());
    assert!(mounted.admission.registry().snapshot().is_empty());
    assert!(mounted.prompt.registry().snapshot().is_empty());
    assert!(mounted.renderers.registry().snapshot().is_empty());
    mounted.root.fiber().dispose().await.unwrap();
    let _ = std::fs::remove_dir_all(root);
}

/// Turning a Skill on or off changes the model's instructions, so the
/// plugin's rule puts every Skill management action to a person, under any
/// permission mode, for as long as the plugin runs.
#[tokio::test]
async fn skill_management_always_requires_a_person_while_the_plugin_runs() {
    let (skills, root) = empty_skills("approval-rule");
    let mounted = mount(skills_plugin(
        skills,
        disconnected_mcp_application(root.join("mcp-settings.json")),
    ))
    .await;
    let context = reference_context(&kernel_reference());
    for name in ["enable_skill", "disable_skill"] {
        let call = parse_call(name, json!({ "skill": "user:pdf-tools" })).unwrap();
        let action = skill_approval_action(&context, &call).unwrap().unwrap();
        assert_eq!(
            mounted.rules.snapshot().verdict(None, &action),
            Some(ApprovalRuleVerdict::RequireUser),
            "{name}"
        );
    }
    let other = ApprovalAction::from_canonical_bytes(
        ApprovalActionKind::DomainMutation,
        ApprovalRiskClass::High,
        ApprovalOrigin::Model,
        "start instance".to_string(),
        b"instance",
    )
    .unwrap();
    assert_eq!(mounted.rules.snapshot().verdict(None, &other), None);

    mounted.fiber.dispose().await.unwrap();
    assert!(mounted.rules.registry().snapshot().is_empty());
    mounted.root.fiber().dispose().await.unwrap();
    let _ = std::fs::remove_dir_all(root);
}
