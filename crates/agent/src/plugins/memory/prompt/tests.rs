use super::*;

fn render(prepared: Option<&TurnExtensions>) -> Option<PromptFragment> {
    let turn = TurnExtensions::default();
    memory_fragment(&PromptRequest {
        turn: &turn,
        prepared,
        context_window_tokens: Some(128_000),
    })
}

fn served(generation: u64, summary: &str) -> TurnExtensions {
    TurnExtensions::default().with(Arc::new(ServedMemory(AgentMemoryContext {
        generation,
        summary: summary.to_string(),
    })))
}

/// The summary a step was served becomes one developer fragment carrying
/// its generation as provenance; a request outside a sampling step, or a
/// step served no summary, gets none.
#[test]
fn the_served_summary_is_the_steps_memory_fragment() {
    assert_eq!(render(None), None);
    assert_eq!(render(Some(&TurnExtensions::default())), None);
    assert_eq!(render(Some(&served(3, " \n"))), None);
    assert_eq!(
        render(Some(&served(
            3,
            "v1\n\n- Validation preferences: MEMORY.md"
        ))),
        Some(PromptFragment {
            kind: AGENT_MEMORY,
            version: Some("3".to_string()),
            content: "v1\n\n- Validation preferences: MEMORY.md".to_string(),
        })
    );
}
