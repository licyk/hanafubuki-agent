#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum McpError {
    #[error("MCP configuration is invalid: {0}")]
    InvalidConfig(String),
    #[error("MCP capability is unsupported: {0}")]
    Unsupported(String),
    #[error("MCP server is unavailable: {0}")]
    Unavailable(String),
    #[error("MCP stdio program resolution failed: {0}")]
    ResolveProgram(String),
    #[error("MCP connection generation is stale")]
    StaleGeneration,
    /// The step's tool was bound to a connection that has since closed.
    #[error("the MCP server's tools changed; search for the tool again or retry the call")]
    CatalogChanged,
    #[error("MCP operation timed out")]
    Timeout,
    #[error("MCP operation was cancelled")]
    Cancelled,
    #[error("MCP protocol failed: {0}")]
    Protocol(String),
    #[error("MCP process cleanup is unconfirmed: {0}")]
    CleanupUnconfirmed(String),
    #[error("MCP result exceeds local limits")]
    LimitExceeded,
}
