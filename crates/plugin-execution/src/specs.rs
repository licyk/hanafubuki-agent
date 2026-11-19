//! Model-visible descriptions and schemas (the Agent's model-request
//! goldens pin them).

use serde_json::{json, Value};

use crate::tool::ExecToolKind;

pub(crate) fn spec(kind: ExecToolKind) -> (&'static str, Value) {
    match kind {
        ExecToolKind::ExecCommand => (
            "Run a shell command after approval. If it completes before yield-time, return output and exit code; otherwise return a live session ID for write_stdin. tty=false uses non-interactive pipes; tty=true allocates a writable, resizable PTY. shell \"posix\" and \"power_shell\" are the platform's own choice and fall back (posix: bash then sh; power_shell: pwsh then powershell); \"bash\", \"sh\", \"pwsh\" and \"powershell\" run exactly the shell they name, never the other of the pair, and \"cmd\" runs cmd.exe. On Windows, the POSIX values run the bash or sh of Git for Windows or MSYS2 when one is installed; the default there stays power_shell. The command runs in the sandbox the `sandbox` world state describes. When a command important to the task needs to run outside it (to write outside the writable roots, or use the network when it is off), or failed because of it (a permission error, or a DNS, connection or download failure), run it again with sandbox_permissions \"require_escalated\" and a short justification question; it then needs its own approval, which the full-access permission mode never gives.",
            json!({
                "type": "object",
                "properties": {
                    "cmd": { "type": "string", "minLength": 1, "maxLength": 49152 },
                    "shell": {
                        "type": ["string", "null"],
                        "enum": ["posix", "bash", "sh", "power_shell", "pwsh", "powershell", "cmd", null],
                        "description": "Shell to run the command in. Omit for the platform's default: `power_shell` on Windows, `posix` elsewhere."
                    },
                    "login": {
                        "type": ["boolean", "null"],
                        "description": "Load shell startup files: POSIX login mode or PowerShell profiles. true enables, false disables; null or omitted defaults to true for posix, bash and sh, and the shellUseProfile setting for the three PowerShell values. Ignored by cmd."
                    },
                    "tty": {
                        "type": ["boolean", "null"],
                        "description": "True allocates a PTY for the command; false or omitted uses plain pipes."
                    },
                    "yield_time_ms": {
                        "type": ["integer", "null"],
                        "minimum": 1,
                        "maximum": 30000,
                        "description": "Wait before returning a session ID for a still-running command. Defaults to 10000 ms."
                    },
                    "max_output_bytes": {
                        "type": ["integer", "null"],
                        "minimum": 1,
                        "maximum": 262144,
                        "description": "Bytes of unread output this wait returns, keeping the head and the tail. Defaults to 65536."
                    },
                    "workdir": {
                        "type": ["string", "null"],
                        "maxLength": 4096,
                        "description": "Working directory for the command. Defaults to the session's working directory."
                    },
                    "sandbox_permissions": {
                        "type": ["string", "null"],
                        "enum": ["use_default", "require_escalated", null],
                        "description": "Per-command sandbox override. Defaults to `use_default`; use `require_escalated` for unsandboxed execution."
                    },
                    "justification": {
                        "type": ["string", "null"],
                        "maxLength": 1024,
                        "description": "User-facing approval question for `require_escalated`, for example \"Do you want to download the project's dependencies?\"; null otherwise."
                    }
                },
                "required": ["cmd"],
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
