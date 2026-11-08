use super::*;

#[test]
fn ring_buffer_is_bounded_and_redacts_secret_values_and_url_queries() {
    let store = McpDiagnosticStore::default();
    let server = McpServerId::parse("fixture").unwrap();
    for index in 0..MCP_DIAGNOSTIC_CAPACITY + 7 {
        store.record(
            server.clone(),
            McpDiagnosticStage::ToolCall,
            McpDiagnosticOutcome::Error,
            "tool_failed",
            format!("token=secret-{index} https://example.test/callback?code=private"),
            "stdio",
            [1; 32],
            None,
            Some("read".to_string()),
            true,
            "inspect",
        );
    }
    store.set_stderr_tail(
        server.clone(),
        &format!(
            "password=hunter2\n{}",
            "line\n".repeat(MCP_STDERR_TAIL_LINES + 8)
        ),
    );
    let snapshot = store.snapshot(&server);
    assert_eq!(snapshot.entries.len(), MCP_DIAGNOSTIC_CAPACITY);
    assert_eq!(snapshot.entries[0].sequence, 8);
    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(!serialized.contains("secret-"));
    assert!(!serialized.contains("hunter2"));
    assert!(!serialized.contains("code=private"));
    assert!(snapshot.stderr_tail.len() <= MCP_STDERR_TAIL_BYTES);
}

#[test]
fn diagnostic_redaction_covers_bearer_and_json_credential_shapes() {
    let sanitized = sanitize_mcp_diagnostic(
        "Authorization:Bearer top-secret Authorization: Bearer second-secret \"access_token\":\"third-secret\" refresh_token=fourth-secret",
        MCP_DIAGNOSTIC_MESSAGE_BYTES,
    );
    for secret in [
        "top-secret",
        "second-secret",
        "third-secret",
        "fourth-secret",
    ] {
        assert!(!sanitized.contains(secret), "leaked {secret}: {sanitized}");
    }
}

#[test]
fn the_summary_counts_current_entries_by_severity_and_heads_with_the_latest_worst() {
    let store = McpDiagnosticStore::default();
    let server = McpServerId::parse("fixture").unwrap();
    let record = |outcome, code: &str| {
        store.record(
            server.clone(),
            McpDiagnosticStage::DiscoverTools,
            outcome,
            code,
            code,
            "stdio",
            [1; 32],
            None,
            None,
            false,
            "none",
        )
    };
    record(McpDiagnosticOutcome::Error, "earlier_attempt_failed");
    store.begin_attempt(server.clone());
    record(McpDiagnosticOutcome::Success, "initialize_completed");
    record(McpDiagnosticOutcome::Error, "first_error");
    record(McpDiagnosticOutcome::Warning, "later_warning");
    record(McpDiagnosticOutcome::OutcomeUnknown, "cleanup_unconfirmed");
    record(McpDiagnosticOutcome::Cancelled, "operation_cancelled");
    record(McpDiagnosticOutcome::Warning, "latest_warning");

    let summary = store.summary(&server);
    assert_eq!(summary.server_id, server);
    assert_eq!((summary.errors, summary.warnings), (2, 2));
    assert_eq!(
        summary.headline.map(|entry| entry.code),
        Some("cleanup_unconfirmed".to_string())
    );
    // The earlier attempt is still in the full list, but not current.
    assert_eq!(summary.entries.len(), 6);
    assert_eq!(summary.entries[0].code, "initialize_completed");
    assert_eq!(store.snapshot(&server).entries.len(), 7);

    store.begin_attempt(server.clone());
    record(McpDiagnosticOutcome::Success, "initialize_completed");
    let summary = store.summary(&server);
    assert_eq!((summary.errors, summary.warnings), (0, 0));
    assert_eq!(summary.headline, None);
    assert_eq!(summary.entries.len(), 1);

    let unknown = store.summary(&McpServerId::parse("other").unwrap());
    assert!(unknown.entries.is_empty() && unknown.headline.is_none());
}
