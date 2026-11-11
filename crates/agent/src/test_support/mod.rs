//! Fixtures for tests of the Agent and of a host's integration with it: a
//! scripted model provider and MCP servers, plugins mounted on their own
//! root, and an Agent built over temporary paths with a host's ports.

#[cfg(unix)]
mod mcp_server;
mod model_server;
mod plugins;
mod service;

#[cfg(unix)]
pub use mcp_server::{annotated_tool, scripted_tool, McpCallHold, ScriptedMcpServers};
pub use model_server::{
    anthropic_assistant_events, anthropic_tool_events, assistant_events, namespace_tool_events,
    reasoning_answer_events, reasoning_tool_events, tool_calls_events, tool_events,
    tool_search_events, with_input_tokens, CapturedRequest, FakeListing, FakeServer,
};
pub use plugins::{mount, mount_tool_plugin, test_tool_context, MountedPlugin, MountedToolPlugin};
pub use service::{
    configure_test_provider, enable_memory, enable_two_skills, service_with_host, test_host,
    test_paths, FixedProxy, FixedWorkspaceRoots, NoHostContext, TestBlockingOperationPort,
    TestPaths, TEST_SHUTDOWN_DEADLINE, TEST_WAIT,
};

/// A secure store in memory.
pub use hanafubuki_agent_foundation::secrets::MemorySecretStore;
/// A turn's world state from a registry's contributors, as admission
/// collects it.
pub use hanafubuki_agent_kernel::collect_world_state;
/// What a host's tests configure MCP servers with.
pub use hanafubuki_agent_mcp::test_support::panic_on_call;
pub use hanafubuki_agent_mcp::{McpApprovalPolicy, McpExposurePolicy};
