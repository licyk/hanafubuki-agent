//! Model-visible descriptions and schemas, unchanged from the fixed tools
//! they replace (the Agent's model-request goldens pin them).

use serde_json::{json, Value};

use crate::tool::ExecToolKind;

pub(crate) fn spec(kind: ExecToolKind) -> (&'static str, Value) {
    match kind {
        ExecToolKind::ExecCommand => (
            "Run a shell command after approval. If it completes before yield-time, return output and exit code; otherwise return a live session ID for write_stdin. tty=false uses non-interactive pipes; tty=true allocates a writable, resizable PTY.",
            json!({
                "type": "object",
                "properties": {
                    "cmd": { "type": "string", "minLength": 1, "maxLength": 49152 },
                    "shell": { "type": ["string", "null"], "enum": ["posix", "power_shell", "cmd", null] },
                    "login": { "type": ["boolean", "null"] },
                    "tty": { "type": ["boolean", "null"] },
                    "yield_time_ms": { "type": ["integer", "null"], "minimum": 1, "maximum": 30000 },
                    "max_output_bytes": { "type": ["integer", "null"], "minimum": 1, "maximum": 262144 },
                    "workdir": { "type": ["string", "null"], "maxLength": 4096 }
                },
                "required": ["cmd", "shell", "login", "tty", "yield_time_ms", "max_output_bytes", "workdir"],
                "additionalProperties": false
            }),
        ),
        ExecToolKind::WriteStdin => (
            "Interact with a live Unified Exec session, then wait for bounded output. Empty chars polls only. PTY sessions accept arbitrary input; pipe sessions reject ordinary input but accept exactly Ctrl+C (ETX, U+0003) as an interrupt request.",
            json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "integer", "minimum": 1 },
                    "chars": { "type": ["string", "null"], "maxLength": 16384 },
                    "yield_time_ms": { "type": ["integer", "null"], "minimum": 1, "maximum": 30000 },
                    "max_output_bytes": { "type": ["integer", "null"], "minimum": 1, "maximum": 262144 }
                },
                "required": ["session_id", "chars", "yield_time_ms", "max_output_bytes"],
                "additionalProperties": false
            }),
        ),
        ExecToolKind::ResizeExecution => (
            "Resize a live Unified Exec PTY session.",
            json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "integer", "minimum": 1 },
                    "cols": { "type": "integer", "minimum": 1, "maximum": 500 },
                    "rows": { "type": "integer", "minimum": 1, "maximum": 500 }
                },
                "required": ["session_id", "cols", "rows"],
                "additionalProperties": false
            }),
        ),
    }
}
