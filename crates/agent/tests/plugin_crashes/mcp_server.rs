//! The test MCP server: this binary, started with [`MCP_ARG`] and a state
//! directory, serves MCP over stdio, one JSON-RPC message per line. It
//! counts its starts in `<state>/starts` and offers two tools: `echo`
//! answers, and `die` ends the process mid-call, as a crashing server does.

use std::io::{BufRead, Write};
use std::path::Path;

use serde_json::{json, Value};

/// The first argument that makes this binary the test MCP server.
pub const MCP_ARG: &str = "--hanafubuki-crash-mcp-server";

/// Serves MCP with the state directory `args[0]` until stdin closes.
pub fn serve(args: &[String]) {
    let state = Path::new(args.first().map(String::as_str).unwrap_or_default());
    std::fs::write(state.join("starts"), (starts(state) + 1).to_string()).unwrap();
    let schema = json!({ "type": "object", "properties": {}, "additionalProperties": false });
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let Ok(request) = serde_json::from_str::<Value>(&line.unwrap_or_default()) else {
            continue;
        };
        // Notifications are not answered.
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let result = match request["method"].as_str().unwrap_or_default() {
            "initialize" => json!({
                "protocolVersion": request["params"]["protocolVersion"],
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "crashing", "version": "1" },
            }),
            "tools/list" => json!({ "tools": [
                { "name": "echo", "description": "Echoes the records.", "inputSchema": schema },
                { "name": "die", "description": "Ends the server.", "inputSchema": schema },
            ]}),
            "tools/call" if request["params"]["name"] == "die" => std::process::exit(3),
            "tools/call" => json!({
                "content": [{ "type": "text", "text": "the crashing server echoed" }],
                "isError": false,
            }),
            _ => json!({}),
        };
        let answer = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        writeln!(stdout, "{answer}").unwrap();
        stdout.flush().unwrap();
    }
}

/// How many times the server started with the state directory `state`.
pub fn starts(state: &Path) -> u32 {
    std::fs::read_to_string(state.join("starts"))
        .ok()
        .and_then(|count| count.trim().parse().ok())
        .unwrap_or(0)
}
