use super::AdvisorService;
use hanafubuki_agent_contracts::{
    CancellationToken, ContextRenderer, ContextRole, ContextSectionKind, ContextSlot,
    PromptContributor, PromptFragment, PromptPrepareFuture, PromptRequest, PromptStep,
    TurnExtensions,
};
use std::sync::Arc;

pub(super) const RENDERER: ContextRenderer = ContextRenderer {
    kind: ContextSectionKind::fixed("advisor_guidance"),
    slot: ContextSlot::Capabilities,
    order: 140,
    role: ContextRole::Developer,
    preamble: "",
    epilogue: "",
    versioned: false,
};

pub(super) struct Guidance(pub Arc<AdvisorService>);
struct Instructions(String);

impl PromptContributor for Guidance {
    fn id(&self) -> &str {
        "advisor"
    }
    fn rank(&self) -> u32 {
        0
    }

    fn prepare<'a>(
        &'a self,
        step: &'a PromptStep,
        _: CancellationToken,
    ) -> PromptPrepareFuture<'a> {
        Box::pin(async move {
            let mut result = TurnExtensions::default();
            let options = self.0.options.read();
            if step.tools.contains("ask_advisor")
                && !options.simple
                && !options.review_stages.is_empty()
            {
                let stages = options.review_stages.iter().map(|s| match s.as_str() {
                    "plan" => "Before acting on a consequential plan, ask for a second opinion on your proposed changes and validation.",
                    "failure" => "After repeated failures, ask for an independent diagnosis using the available evidence.",
                    "completion" => "Before reporting substantial work complete, ask for a review of the changes and their verification.",
                    custom => custom,
                }).collect::<Vec<_>>().join("\n");
                result.insert(Arc::new(Instructions(format!(
                    "Advisor review preferences:\n{stages}"
                ))));
            }
            Ok(result)
        })
    }

    fn contribute(&self, request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        request
            .prepared
            .and_then(|p| p.get::<Instructions>())
            .map(|i| PromptFragment {
                kind: RENDERER.kind,
                version: None,
                content: i.0.clone(),
            })
            .into_iter()
            .collect()
    }
}
