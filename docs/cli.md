# The command line

`hanafubuki-agent` is the standalone Agent's binary (the crate
`hanafubuki-agent-cli`). It runs one turn headless (`exec`) and manages the
configuration, the model Providers, the MCP servers, the sessions, the
Skills and the memory; with no command it runs the terminal interface
(TUI). Its configuration is described in [config.md](config.md), its
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
| `hanafubuki-agent [PROMPT]` | The terminal interface (below), in a new session; `PROMPT` is its first message. Without a terminal on standard input and output it prints how to use `exec` and exits with 2. |
| `resume [ID] [PROMPT]`, `resume --last [PROMPT]` | The terminal interface on an earlier session, its transcript replayed: the session `ID` (from `sessions list`), the latest one a turn ran in (`--last`; a new session when there is none), or, with neither, the one picked from the resume picker (Esc there starts a new session). `PROMPT` is the first message. Without a terminal it exits with 2, as the interface does. |
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
| `sessions show ID [--json]` | The session's latest 200 transcript items: messages, how long the model thought (`thought for 12s`), tool calls and their results (`--json`: the transcript page, the thinking's text included). |
| `sessions archive ID`, `sessions delete ID` | Archives a session; deletes one and its history. |
| `skills list`, `skills install DIR [--overwrite]`, `skills remove ID` | The Skill catalog; installs the Skill in `DIR`; removes an installed Skill. |
| `memory status`, `memory reset` | The memory's state; forgets everything it holds. |
| `doctor [--online]` | Checks whether the Agent can run (below). |

## The terminal interface

`hanafubuki-agent [PROMPT]` opens the interface at the bottom of the
terminal, and the conversation goes into the terminal's scrollback above
it. `-m`, `-C`, `--add-dir` and the other global flags apply as for `exec`;
the permission mode decides which tool calls ask, and each one that asks
opens an approval: `y` once, `a` for the rest of the session (when the
call offers it), `n` or Esc to deny. The model's questions open one at a
time, an option to pick for each.

| Key | Does |
| --- | --- |
| Enter | Sends the message: a new turn, or, while one runs, a message that steers it; in a popup, picks its entry |
| Shift+Enter, Ctrl+J | A new line |
| Up, Down | The session's earlier messages; in a popup, its entries |
| Tab | In the command popup, completes the command; in the file popup, inserts the file |
| Esc, Ctrl-C | Interrupts the running turn; Esc first closes a popup; Ctrl-C first clears a composer that has text, and in an approval denies it and those waiting |
| Ctrl-T | The transcript in a pager (below) |
| Ctrl-O | Shows or hides the model's thinking (below), for the live thinking and the rest of the session |
| `?` on an empty composer | The shortcuts |
| Ctrl-C twice, Ctrl-D twice on an empty composer, `/quit` | Quits |

Quitting restores the terminal and shuts the Agent down; the exit code is
0, or 1 when the Agent could not start or the terminal could not be used.

### Thinking

While the model thinks, the viewport shows `• Thinking 5s`, counting up;
the status line stays `Working`. Ctrl-O expands it to the thinking's text
as it streams (the provider's summary of its reasoning, or its raw
reasoning, as Markdown) and collapses it again; the choice holds for every
later thinking in the session, and the footer offers it while thinking is
shown. When the thinking ends, it goes into the scrollback as it was
shown: `• Thought for 12s` (with `(truncated)` when its text was cut at
the Agent's bound), or that line and the text. The scrollback cannot be
changed afterwards, so the transcript pager (Ctrl-T) always shows the
text. A turn that ends while the model thinks ends its thinking with the
time so far; a provider that sends no text leaves only the line. A resumed
session shows its thinking the same way, collapsed unless Ctrl-O expanded
thinking.

### Commands

A message that starts with `/` names a command; typing `/` opens a popup
of them, narrowed as the name is typed. A `/name` that is no command is
not sent (a path such as `/usr/bin/env`, or a leading space, is). The
commands marked *not during a turn* say so instead of running while a turn
runs.

| Command | Does |
| --- | --- |
| `/model` | A list of the models the enabled Providers report (their model catalog), the session's own marked; picking one sets it on the session. A Provider whose catalog cannot be read is listed with why. *Not during a turn.* |
| `/mode` | Switches the session between `default` and `plan` mode (read-only exploration that ends with a proposed plan). *Not during a turn.* |
| `/skills` | The Skill catalog: each Skill, whether it is enabled, and the catalog's issues. |
| `/new` | A new session; the previous one stays in the history. *Not during a turn.* |
| `/resume` | The resume picker (below). *Not during a turn.* |
| `/compact` | Compacts the session's context now, as automatic compaction does. *Not during a turn.* |
| `/diff` | `git diff` of the first workspace root, untracked files included, in a pager. |
| `/mention` | Inserts `@`, which opens the file popup. |
| `/status` | The session's model and Provider, the workspace roots, the permission mode, the session's mode and id, and its token use. |
| `/mcp` | The MCP servers: enabled, connection state, command or URL, tools. |
| `/quit`, `/exit` | Quits. |

The permission mode is the configuration's (`permissionMode`,
`--permission-mode`); `/status` shows it, no command changes it.

### File mentions

`@` followed by a few characters opens a popup of the workspace roots'
files that match them fuzzily (`.gitignore` and the other ignore files are
honoured); Enter or Tab replaces the `@token` with the file's path:
relative to the first root, or whole for a file in another root, quoted
when it has spaces. Only the path is sent; the model reads the file with
its tools.

### The pager and the resume picker

Ctrl-T shows the session's transcript (each thinking with its text), and
`/diff` its diff, on the terminal's alternate screen: Up, Down (`k`, `j`),
Page Up, Page Down (Space), Home and End scroll; `q` (and for the
transcript Esc or Ctrl-T) closes it. What the turn writes meanwhile
appears in the scrollback when it closes.

The resume picker lists the sessions a turn ran in, the latest first, by
title (a session's first message until it is named) and when it was last
updated; typing searches the titles. Enter reopens the session: the
interface shows its header and its latest 200 transcript items (messages,
thinking, plans, tool calls with their output), and the next message
continues it.
Esc goes back (at `resume`'s start, to a new session); Ctrl-C quits.

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
Standard error has the session's id, how long each thinking took (`·
thought for 3s`; its text is not printed), model retries, and how a turn
that did not complete ended.

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
| `reasoning_updated` | `reasoning` (the turn's thinking time: `phase`, `activeSince`, …) |
| `reasoning_started`, `reasoning_delta`, `reasoning_completed` | `reasoningId`, and `samplingStep`, `delta` (Markdown) or `durationMillis` |
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
| 2 | The command line is not one the binary accepts: an unknown flag, no prompt, an `--approve` that is not a policy, a session id that is not one, the terminal interface or `resume` without a terminal. |
| 3 | The model asked a question no one could answer, so the turn was cancelled. |
| 130 | The turn was interrupted (Ctrl-C). |

## `doctor`

Each check prints one line, `ok`, `FAIL` or `skip`, and any `FAIL` exits
with 1:

- **config**: each layer read: the user's `config.toml` (or that there is
  none), the profile, the project's file (and whether it is trusted, hence
  read), the `-c` overrides. A profile that does not exist fails. Then
  each file a read had to repair, with what it repaired; one whose
  original could not be kept fails, since it refuses changes until the
  Agent restarts.
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
