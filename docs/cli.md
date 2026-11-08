# The command line

`hanafubuki-agent` is the standalone Agent's binary (the crate
`hanafubuki-agent-cli`). It runs one turn headless (`exec`) and manages the
configuration, the model Providers, the MCP servers, the sessions, the
Skills and the memory; the terminal interface (TUI) comes in a later
release. Its configuration is described in [config.md](config.md), its
design in [architecture.md §3.6](architecture.md#36-standalone).

```sh
cargo install --path crates/cli      # or: cargo run -p hanafubuki-agent-cli -- …
hanafubuki-agent providers add OpenAI --base-url https://api.openai.com/v1 --api-key-env OPENAI_API_KEY
hanafubuki-agent providers default provider-1 --model gpt-5.1
hanafubuki-agent exec "Explain what this repository does"
```

## Global flags

They go before or after the command.

| Flag | Meaning |
| --- | --- |
| `-c`, `--config KEY=VALUE` | Overrides a key of `config.toml` for this run; repeatable, later wins ([config.md](config.md#layers)). |
| `-p`, `--profile NAME` | Reads `[profiles.NAME]` of `config.toml` over the user's settings; a profile the file does not have is an error. |
| `-m`, `--model MODEL` | The model `exec`'s session runs, in place of the model policy's, on the default Provider. |
| `-C`, `--cd DIR` | The working directory: the first workspace root, and with its Git root the project whose configuration may be read. |
| `--add-dir DIR` | Another workspace root; repeatable. |
| `--permission-mode manual\|auto-review\|full-access` | The permission mode for this run (`-c permissionMode=…`, after the other `-c`). |
| `--log-level off\|error\|warn\|info\|debug\|trace` | The log file's level for this run (`-c logLevel=…`, after the other `-c`). Only `trace` records model and tool content. |
| `--data-dir DIR` | The data directory (history, memory, Skills, logs, the lock), in place of the one the platform or `HANAFUBUKI_AGENT_HOME` names. |

`HANAFUBUKI_AGENT_HOME` names one directory for both the configuration and
the data ([config.md](config.md#locations)).

Every command reads the configuration, then installs the Agent's panic hook
and its log file (`<data>/logs/hanafubuki-agent.log`) before it starts the
Agent or changes anything. The
commands that only read or write the configuration (`config`,
`providers list`, `mcp list`, `doctor` without `--online`) do not start the
Agent and work while another Agent process runs; the others start it and
need the data directory to themselves.

## Commands

| Command | Does |
| --- | --- |
| `hanafubuki-agent [PROMPT]`, `resume [--last\|ID]` | The terminal interface, not available yet: prints how to use `exec` and exits with 2. |
| `exec [PROMPT\|-]` | One turn (below). |
| `config path` | Prints where `config.toml`, `providers.toml`, `mcp.toml`, the data and the log are. |
| `config get [KEY]` | A key's value with every layer applied (a string as it is, anything else as TOML); without a key, every setting as a TOML document. The credentials of a proxy URL print as `***`. |
| `config set KEY VALUE` | Writes one key into the user's `config.toml` ([config.md](config.md#reading-and-writing-keys)): any setting, `profiles.NAME.KEY`, `projects."PATH".trustLevel`. |
| `config edit` | Opens the user's `config.toml` in `$VISUAL` or `$EDITOR`. |
| `providers list` | The model policy's model and each Provider: id, name, default, protocol, endpoint, and whether a token and which headers are set; never a token's or a header's value. |
| `providers add NAME --base-url URL [--protocol responses\|chat-completions\|anthropic-messages] [--api-key-env VAR] [--disabled]` | Adds a Provider (the token read from the variable `VAR`, never from the command line) and prints its id. The first one becomes the default. |
| `providers remove ID` | Removes a Provider; the default moves to another one when it was the default. |
| `providers default ID [--model MODEL]` | Makes a Provider the default, and sets the model the policy runs. |
| `mcp list` | Each server: id, enabled, transport and target, the names (never the values) of its environment variables or headers; then the credentials that are set. |
| `mcp add ID (--url URL \| [--env KEY=VALUE]… -- PROGRAM [ARG]…) [--disabled]` | Adds a Streamable HTTP or a stdio server with the defaults of a new server, connects it when it is enabled and prints how that went. Credentials (`credentialEnvironment`, `credentialHeaders`, OAuth) are edited in `mcp.toml`. |
| `mcp remove ID` | Disconnects and removes a server. |
| `mcp connect ID` | Connects a server and prints its phase and tool count; fails when it does not reach `ready`. |
| `sessions list [--archived] [--json]` | The sessions, the latest first: id, last update, model, title (`--json`: one session summary per line). |
| `sessions show ID [--json]` | The session's latest 200 transcript items: messages, tool calls and their results (`--json`: the transcript page). |
| `sessions archive ID`, `sessions delete ID` | Archives a session; deletes one and its history. |
| `skills list`, `skills install DIR [--overwrite]`, `skills remove ID` | The Skill catalog; installs the Skill in `DIR`; removes an installed Skill. |
| `memory status`, `memory reset` | The memory's state; forgets everything it holds. |
| `doctor [--online]` | Checks whether the Agent can run (below). |

## `exec`

`exec` creates a session, runs one turn in it and exits when the turn ends.
The prompt is the argument; `-`, or no argument with standard input not a
terminal, reads it from standard input. `-o FILE`
(`--output-last-message`) writes the turn's final message to `FILE` (empty
when the turn had none).

### Output

Without `--json`, standard output is the turn for a person:

```
→ exec_command
· approval denied: exec_command: Run Posix command in /home/me/project: rm -rf build
  exec_command failed
· The build directory is not removed; …
The final answer.
```

Each assistant message is printed when it completes, commentary prefixed
with `·`, the final answer as it is; a proposed plan follows `plan:`; a
tool call is `→ NAME[: SUBJECT]` and its end `  NAME done` or `  NAME
failed`; `exec`'s own notes (approvals it answered) are prefixed with `·`.
Standard error has the session's id, model retries, and how a turn that
did not complete ended.

With `--json`, standard output has one JSON object per line: every event of
the turn as the facade serializes it (`AgentEvent`), in order. `exec`'s own
notes go to standard error. An event is:

```json
{"sessionId":1,"turnId":1,"generation":1,"sequence":4,"activity":null,
 "kind":{"type":"assistant_text_delta","messageId":2,"delta":"Hel"}}
```

| Field | |
| --- | --- |
| `sessionId`, `turnId`, `generation` | The session, the turn (`null` for an event of the session), and the session's generation. |
| `sequence` | The event's number in the session, increasing. |
| `activity` | The activity the event changed (what the turn is doing, for a status line), or `null`. |
| `kind.type` | What happened; the other fields of `kind` (camelCase) depend on it. |

The kinds, with their fields:

| `kind.type` | Fields |
| --- | --- |
| `turn_started` | |
| `sampling_started` | `step` |
| `response_started` | `responseId` |
| `reasoning_updated`, `reasoning_headline_updated` | `reasoning`; `headline` |
| `assistant_message_started` | `messageId`, `phase` (`commentary`, `final_answer` or `null`), `samplingStep` |
| `assistant_text_delta` | `messageId`, `delta` |
| `assistant_message_completed` | `messageId`, `phase` |
| `proposed_plan_started`, `proposed_plan_delta`, `proposed_plan_completed` | `messageId`, and `samplingStep` or `delta` |
| `tool_started` | `callId`, `name`, `presentation` (`action`, `subject`) |
| `tool_execution_finished`, `tool_completed` | `callId`, `isError` |
| `execution_progress` | `update` (a process's start, output, status, end) |
| `approval_requested` | `request` (`requestId`, `operationId` = the tool call's `callId`, `action` with `kind`, `risk`, `summary`, …) |
| `approval_resolved` | `requestId`, `resolution` (`decision`: `allow_once`, `allow_for_session`, `deny`, `aborted`, `timed_out`; `source`: `user` for `exec`'s answers, `full_access`, `auto_reviewer`, `session_allowance`, `system`) |
| `permission_review_started`, `permission_review_completed` | the automatic review of `auto_review` mode |
| `user_input_requested`, `user_input_resolved` | `request` (`questions` with `options`); `resolution` |
| `task_plan_updated` | `plan` |
| `history_committed` | `startIndex`, `itemCount`, `usage` |
| `continuation_hook_started`, `continuation_hook_completed` | `providerId`, `hookId`, `outcome` |
| `steering_accepted`, `steering_consumed` | the steering sequence numbers |
| `model_retry_scheduled` | `stage`, `currentRetry`, `maximumRetries`, `delayMillis`, `reason` |
| `compaction_updated` | `compaction` |
| `session_title_updated` | `title` |
| `turn_completed` | `finalAnswerMessageId` |
| `turn_cancelled` | `reason` |
| `turn_failed` | `reason`, `message` |

The last line of a turn that ran is `turn_completed`, `turn_cancelled` or
`turn_failed`. The schema is the facade's event type, so it changes with
it: a new kind or field may appear in any release, and a reader should skip
what it does not know; a field is removed or changes meaning only with the
minor version (before 1.0).

### Approvals and questions

`--permission-mode` (or `permissionMode`) decides which tool calls need an
approval; `--approve` answers them:

| `--approve` | |
| --- | --- |
| `deny` (the default) | Every approval is denied; the model is told and the turn goes on. |
| `ask` | The person at the terminal answers (`y`, `s` for the rest of the session when offered, anything else denies). Without a terminal on both standard input and standard error, denied. |
| `allow:TOOL[,TOOL…]` | The named tools are allowed once each time they ask (`allow:exec_command,apply_patch`); the others are denied. |

Each answer is printed (`· approval denied: TOOL: SUMMARY`). A question the
model asks (`request_user_input`) is put to the person at the terminal,
who picks an option by number; without a terminal, or without an answer,
`exec` cancels the turn and exits with 3.

Ctrl-C cancels the turn and `exec` exits once it ended, after the Agent
shut down; a second Ctrl-C exits at once. Ctrl-C during another command
ends it at once: the operating system releases the data directory's lock,
and the next start reaps the processes the Agent had started.

### Exit codes

| Code | |
| --- | --- |
| 0 | The turn completed (or a management command did what it was asked). |
| 1 | The turn failed (`turn_failed`, any reason), the Agent cancelled it (`turn_cancelled` for another reason than an interruption), the Agent stopped during it, or the command could not run (no Provider, the data directory in use, a key that cannot be set). |
| 2 | The command line is not one the binary accepts: an unknown flag, no prompt, an `--approve` that is not a policy, the terminal interface or `resume`. |
| 3 | The model asked a question no one could answer, so the turn was cancelled. |
| 130 | The turn was interrupted (Ctrl-C). |

## `doctor`

Each check prints one line, `ok`, `FAIL` or `skip`, and any `FAIL` exits
with 1:

- **config**: each layer read: the user's `config.toml` (or that there is
  none), the profile, the project's file (and whether it is trusted, hence
  read), the `-c` overrides. A profile that does not exist fails.
- **providers**: how many, and the default with its endpoint and model;
  no default or no model fails.
- **mcp**: how many servers, how many enabled.
- **keyring**: whether the operating system's keyring answers a read
  (MCP OAuth tokens are kept there); a machine without one (a Linux
  without the Secret Service) fails.
- **data**: whether the data directory is free; held by another Agent
  process fails (which process is not known: the lock holds no name).
- **online** (`--online` only, and only when the data directory is free):
  starts the Agent, lists the default Provider's models (`provider`), and
  connects each enabled MCP server (`mcp`), each failing when it does not
  answer or connect.
