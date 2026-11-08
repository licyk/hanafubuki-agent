//! A sampling step's memory policy comes from what a prompt contributor
//! prepared for the step (the `memory` plugin's). It shapes the step's
//! answers and decides whether external content pollutes the session's
//! memory contribution; a step without one changes nothing.

use hanafubuki_agent_approval::PermissionRequirement;
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryCitationSnapshot, AgentMemoryContextEffect,
    AgentMemoryContributionCause, AgentMemoryStep, AgentMemoryStepPolicy, AgentNotification,
    AgentObserver, AgentProcessedMemoryAnswer, AgentToolHandler, ObserverRegistry,
    PromptPrepareFuture, PromptStep, ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation,
    ToolRegistry, ToolResult,
};
use hanafubuki_agent_kernel::{AgentKernelRegistries, AgentMemoryContributionMode};
use hanafubuki_agent_tool_contracts::{ToolExecutionClass, ToolExposure, ToolName};

use super::*;

const CITATION_MARKER: &str = " [cited]";

fn citation() -> AgentMemoryCitationSnapshot {
    AgentMemoryCitationSnapshot {
        generation: 4,
        resource_id: "MEMORY.md".to_string(),
        start_line: None,
        end_line: None,
        note: "fixture".to_string(),
        source_session_ids: vec!["9".to_string()],
    }
}

/// Withholds and strips a trailing marker, reporting it as one citation.
struct MarkerPolicy {
    excludes_external_context: bool,
}

impl AgentMemoryStepPolicy for MarkerPolicy {
    fn excludes_external_context(&self) -> Result<bool, String> {
        Ok(self.excludes_external_context)
    }

    fn visible_stream_prefix_len(&self, text: &str) -> Result<usize, String> {
        Ok(text.find(CITATION_MARKER).unwrap_or(text.len()))
    }

    fn process_final_answer(&self, text: &str) -> Result<AgentProcessedMemoryAnswer, String> {
        Ok(match text.strip_suffix(CITATION_MARKER) {
            Some(visible) => AgentProcessedMemoryAnswer {
                visible_text: visible.to_string(),
                citations: vec![citation()],
            },
            None => AgentProcessedMemoryAnswer {
                visible_text: text.to_string(),
                citations: Vec::new(),
            },
        })
    }
}

/// Prepares a [`MarkerPolicy`] for every step and renders nothing.
struct PolicyPrompt {
    excludes_external_context: bool,
}

impl PromptContributor for PolicyPrompt {
    fn id(&self) -> &str {
        "memory-policy"
    }

    fn rank(&self) -> u32 {
        1
    }

    fn prepare<'a>(
        &'a self,
        _step: &'a PromptStep,
        _cancellation: CancellationToken,
    ) -> PromptPrepareFuture<'a> {
        let policy = MarkerPolicy {
            excludes_external_context: self.excludes_external_context,
        };
        Box::pin(async move {
            Ok(TurnExtensions::default().with(Arc::new(AgentMemoryStep {
                policy: Arc::new(policy),
            })))
        })
    }

    fn contribute(&self, _request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        Vec::new()
    }
}

/// A read-only tool whose output brings external content into the session.
struct ExternalFetch {
    descriptor: ToolDescriptor,
}

impl AgentToolHandler for ExternalFetch {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
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
        Box::pin(async { Ok(ToolResult::success(json!({ "page": "external" }))) })
    }
}

fn external_fetch() -> Arc<dyn AgentToolHandler> {
    Arc::new(ExternalFetch {
        descriptor: ToolDescriptor {
            name: ToolName::parse("external_fetch").unwrap(),
            description: "external_fetch fixture".to_string(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            exposure: ToolExposure::Direct,
            execution_class: ToolExecutionClass::ConcurrentRead,
            effect: ToolEffect::ReadOnly,
            action: AgentActivityAction::USE_TOOL,
            memory_effect: AgentMemoryContextEffect::PollutesContribution,
            timeout: None,
        },
    })
}

fn external_fetch_call() -> Vec<ModelResult<ModelEvent>> {
    let call = ModelToolCall {
        item_id: "item-fetch".to_string(),
        call_id: "call-fetch".to_string(),
        namespace: None,
        name: "external_fetch".to_string(),
        arguments: json!({}),
        argument_error: None,
    };
    vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "response-fetch".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall { call: call.clone() },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall { call },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "response-fetch".to_string(),
            usage: None,
            end_turn: Some(false),
        }),
    ]
}

/// The citations of each committed answer and the cause of each committed
/// contribution change.
#[derive(Default)]
struct CommittedMemoryFacts {
    citations: Mutex<Vec<Vec<AgentMemoryCitationSnapshot>>>,
    contribution_causes: Mutex<Vec<AgentMemoryContributionCause>>,
}

impl AgentObserver for CommittedMemoryFacts {
    fn id(&self) -> &str {
        "committed-citations"
    }

    fn observe(&self, notification: &AgentNotification) {
        match notification {
            AgentNotification::SamplingCommitted {
                memory_citations, ..
            } => self
                .citations
                .lock()
                .unwrap()
                .push(memory_citations.clone()),
            AgentNotification::MemoryContributionChanged { cause, .. } => {
                self.contribution_causes.lock().unwrap().push(*cause);
            }
            _ => {}
        }
    }
}

/// The kernel registries with the external fetch tool, the memory facts
/// observer and, when `policy` is set, a memory policy that excludes
/// external context as it says.
async fn registries(policy: Option<bool>) -> (FakeTools, Arc<CommittedMemoryFacts>) {
    let tools = FakeTools::new([]).await;
    tools
        .mount(&registering::<ToolRegistry>(
            "external-fetch",
            vec![external_fetch()],
        ))
        .await;
    let citations = Arc::new(CommittedMemoryFacts::default());
    tools
        .mount(&registering::<ObserverRegistry>(
            "committed-citations",
            vec![citations.clone() as Arc<dyn AgentObserver>],
        ))
        .await;
    if let Some(excludes_external_context) = policy {
        tools
            .mount(&registering::<PromptContributorRegistry>(
                "memory-policy",
                vec![Arc::new(PolicyPrompt {
                    excludes_external_context,
                }) as Arc<dyn PromptContributor>],
            ))
            .await;
    }
    (tools, citations)
}

fn service_over(model: &ScriptedEventModel, registries: AgentKernelRegistries) -> AgentService {
    AgentService::new(Arc::new(model.clone()), registries)
}

#[tokio::test]
async fn a_steps_memory_policy_strips_the_answer_and_reports_its_citations() {
    let (tools, committed) = registries(Some(false)).await;
    let model = ScriptedEventModel::new([deterministic_assistant_events("done [cited]")]);
    let service = service_over(&model, tools.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "answer from memory".to_string())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "done");
    assert_eq!(*committed.citations.lock().unwrap(), [vec![citation()]]);
    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    assert!(history.iter().any(|item| matches!(
        item,
        ConversationItem::AssistantMessage { text, memory_citations, .. }
            if text == "done" && *memory_citations == [citation()]
    )));
}

#[tokio::test]
async fn without_a_memory_policy_the_answer_is_shown_as_it_is() {
    let (tools, committed) = registries(None).await;
    let model = ScriptedEventModel::new([deterministic_assistant_events("done [cited]")]);
    let service = service_over(&model, tools.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "answer from memory".to_string())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "done [cited]");
    assert_eq!(*committed.citations.lock().unwrap(), [Vec::new()]);
}

/// Runs one turn that calls the external fetch tool in a session that
/// contributes to memory, and returns the contribution mode afterwards and
/// the causes of the contribution changes observers were told.
async fn contribution_after_external_content(
    policy: Option<bool>,
) -> (
    AgentMemoryContributionMode,
    Vec<AgentMemoryContributionCause>,
) {
    let (tools, committed) = registries(policy).await;
    let model = ScriptedEventModel::new([
        external_fetch_call(),
        deterministic_assistant_events("fetched"),
    ]);
    let service = service_over(&model, tools.registries());
    let session = service.create_session(config()).unwrap();
    service
        .set_session_memory_contribution(session.id, session.memory_contribution.revision, true)
        .unwrap();

    let outcome = service
        .start_turn(session.id, "fetch it".to_string())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "fetched");
    let mode = service
        .snapshot(session.id)
        .unwrap()
        .memory_contribution
        .mode;
    let causes = committed.contribution_causes.lock().unwrap().clone();
    (mode, causes)
}

#[tokio::test]
async fn external_content_pollutes_the_contribution_only_under_a_policy_that_excludes_it() {
    use AgentMemoryContributionCause::{Polluted, User};
    assert_eq!(
        contribution_after_external_content(Some(true)).await,
        (AgentMemoryContributionMode::Polluted, vec![User, Polluted])
    );
    assert_eq!(
        contribution_after_external_content(Some(false)).await,
        (AgentMemoryContributionMode::Enabled, vec![User])
    );
    // The memory plugin is off: no step has a policy, nothing is marked.
    assert_eq!(
        contribution_after_external_content(None).await,
        (AgentMemoryContributionMode::Enabled, vec![User])
    );
}
