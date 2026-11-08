//! A hook for tests of the code that runs MCP tool calls: a call the
//! client panics in, which no server can cause.

use std::collections::BTreeSet;

use parking_lot::Mutex;

use crate::{McpServerId, McpToolBinding};

/// The `(server id, remote tool name)` pairs whose calls panic.
static PANICKING_CALLS: Mutex<BTreeSet<(String, String)>> = Mutex::new(BTreeSet::new());

/// Makes every later call of `remote_name` on `server_id` panic in the
/// client before it reaches the server. Tests share the process, so each
/// names a server of its own.
pub fn panic_on_call(server_id: &McpServerId, remote_name: &str) {
    PANICKING_CALLS
        .lock()
        .insert((server_id.as_str().to_string(), remote_name.to_string()));
}

pub(crate) fn panic_if_armed(binding: &McpToolBinding) {
    let armed = PANICKING_CALLS.lock().contains(&(
        binding.server_id.as_str().to_string(),
        binding.remote_name.clone(),
    ));
    if armed {
        panic!("injected MCP client panic");
    }
}
