//! MCP for the Agent: binds `hanafubuki-agent-mcp` to the extension points as the MCP tool
//! source (the connected servers' tools, planned per sampling step) and the
//! resource tools `list_mcp_resources`, `list_mcp_resource_templates` and
//! `read_mcp_resource`, which read through the step's binding.

mod plugin;
mod resources;
mod source;
mod tool;

pub use plugin::mcp_plugin;
