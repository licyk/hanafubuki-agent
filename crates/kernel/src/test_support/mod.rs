//! Deterministic fake tools for Agent contract tests.

mod fake_tools;

pub use fake_tools::{
    fake_mutation_action, standard_fake_tool_specs, FakeRegisteredExchange, FakeTools,
};

/// The view-latch digest that makes a session's compression state panic in
/// every projection and compression tool call, to test that the panic is
/// contained. The algorithm never writes it.
pub const COMPRESSION_PANIC_MARKER: &str = "test-support: panic";

/// A kernel over `model` whose caught kernel panics reach `kernel_faults`.
pub fn service_reporting_kernel_faults(
    model: std::sync::Arc<dyn hanafubuki_agent_model_api::ModelPort>,
    registries: crate::AgentKernelRegistries,
    kernel_faults: std::sync::Arc<dyn crate::AgentKernelFaultSink>,
) -> crate::AgentService {
    crate::AgentService::with_optional_model(
        Some(model),
        registries,
        std::sync::Arc::new(crate::EphemeralAgentHistoryStore::default()),
        hanafubuki_agent_approval::ApprovalService::manual(),
        crate::FIRST_BATCH_HARD_LIMITS,
        kernel_faults,
    )
}

pub fn bound_tool_output_for_test(
    output: hanafubuki_agent_tool_contracts::ToolOutput,
    maximum_bytes: usize,
) -> (
    hanafubuki_agent_tool_contracts::ToolOutput,
    Option<crate::AgentToolOutputTruncation>,
) {
    crate::tool_output::bound_tool_output(output, maximum_bytes)
        .expect("test tool output must satisfy the typed contract and canonical bound")
}

pub fn canonicalize_tool_output_for_test(
    output: hanafubuki_agent_tool_contracts::ToolOutput,
    maximum_bytes: usize,
) -> (
    hanafubuki_agent_tool_contracts::ToolOutput,
    Option<crate::AgentToolOutputTruncation>,
) {
    crate::tool_output::canonicalize_tool_output(output, maximum_bytes)
}
