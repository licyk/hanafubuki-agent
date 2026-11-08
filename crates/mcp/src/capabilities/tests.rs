use rmcp::model::{Implementation, InitializeResult, ServerCapabilities, ServerPeerInfo};

use super::*;

#[test]
fn capability_snapshot_is_generation_bound_bounded_and_marks_unsupported_features() {
    let mut info = InitializeResult::new(
        ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build(),
    );
    info.server_info = Implementation::new("fixture", "1.0.0");
    info.instructions = Some("external instructions".repeat(8));
    let generation = McpConnectionGeneration([7; 32]);
    let info = ServerPeerInfo::from(info);
    let snapshot = McpCapabilitySnapshot::from_peer(generation, &info);
    assert_eq!(snapshot.connection_generation, generation);
    assert!(snapshot.tools);
    assert!(snapshot.resources);
    assert!(!snapshot.tasks_wired);
    assert!(snapshot.elicitation_wired);
    assert!(snapshot.instructions_summary.unwrap().len() <= MAX_MCP_INSTRUCTIONS_SUMMARY_BYTES);
    let mut oversized_info = info;
    oversized_info.instructions = Some("x".repeat(MAX_MCP_INSTRUCTIONS_BYTES + 1));
    let oversized = McpCapabilitySnapshot::from_peer(generation, &oversized_info);
    assert_eq!(oversized.instructions_bytes, MAX_MCP_INSTRUCTIONS_BYTES + 1);
    assert!(oversized.instructions_summary.unwrap().len() <= MAX_MCP_INSTRUCTIONS_SUMMARY_BYTES);
}
