# Plan: Codex's shortcuts, image paste and rendering; titles, `/effort` and `/context`; three fixes

Repositories, read from source on 2026-11-13:

- The Agent: `/root/code_workspace/hanafubuki-agent`, main `e2981f4`.
- Codex: `/root/code_workspace/codex`, `57a808a392` (2026-11-07), the
  revision our files' "Modified from" headers name. Paths below are under
  `codex-rs/tui/src/` unless they say otherwise.
- billion-context: `/root/code_workspace/billion-context`, `4be21b08e`, the
  revision `crates/compression` was derived from.
- Hoshimi: `/root/code_workspace/Hoshimi`, `83f7813`, the revision
  `plan-model-metadata.md` read.

The request has nine parts. Numbers in brackets refer to them throughout:

- [1] Complete the migration of Codex's keyboard shortcuts, each with its
  function, and make `?` Codex's categorised, multi-column overlay:
  - **Compose**: `/` Commands, `@` Mention files, `!` Shell command, Ctrl+J
    New line, Ctrl+V Paste image, Ctrl+G External editor, Ctrl+R Search
    history, Esc Esc Edit last message;
  - **Session**: Tab Send message, Shift+Tab Change mode, Alt+, Less
    reasoning, Alt+. More reasoning, ← Agents (empty prompt), Alt+←/Alt+→
    Subagents (empty prompt), F4 Inspect activity, Ctrl+C Quit;
  - **Transcript (open first)**: Ctrl+T Open transcript, F3 Find text,
    PgUp/PgDn Scroll, Ctrl+Space Start selection, Ctrl+Home/Ctrl+End and
    Shift+Alt+, / Shift+Alt+. Top/latest;
  - the footer `/keymap customize`. F8 Voice is **not** migrated.
- [2] Paste images into the terminal interface, as Codex does.
- [3] Generate a conversation's title from its first user message, and
  keep the first message as the title when generation fails.
- [4] Codex's rendering of Markdown and of the other output formats.
- [5] `/effort`: the current model's reasoning effort. The levels come from
  the model metadata the API reported; with none, only Custom and Default.
  The same options in the Settings Center's reasoning-effort row; a custom
  value can be typed.
- [6] A command that shows the sections of the current context, their
  composition and size (after billion-context), with cached tokens,
  uncached tokens and the cache hit rate.
- [7] Bug: a thinking entry's timer keeps counting after the
  `Worked for … · thought for …` separator appears and the answer streams.
- [8] Bug: `/status` shows `Context window: unknown` though the model
  metadata has it; compaction must use the fetched window too; unknown
  only when the metadata has none.
- [9] Bug: `/recap` fails with `model remote error status=Some(400)
  code=Some("invalid_input"): Structured output (`text.format`) is not
  supported by this stateless gateway.`

The `todo.md` section "Plan: Codex's shortcuts, image paste and rendering;
titles, /effort and /context; three fixes" tracks the milestones.

A path this plan names that does not exist yet is a file its milestone
creates. A module that gains production children moves to `foo/mod.rs`
(AGENTS.md, "Rust source and test layout"): `clipboard.rs`, `footer.rs`,
`markdown_render.rs`, `streaming/controller.rs` and `wrapping.rs` of
`crates/tui/src/` among them.

## 0. Terms

- **Fullscreen** and **Scrollback** are the two presentations
  (`transcript_mode.rs`, `TranscriptMode::{Owned, Terminal}`); Scrollback's
  recent entries live in the repainted tail (`plan-settings-and-timeline.md`
  §4.11).
- **Rung**: one way of asking a model for a JSON object. Native output
  (`text.format` / `response_format` with a JSON schema), a forced tool call
  (`tool_choice` naming one tool), or plain text parsed afterwards.
- **Route**: protocol, endpoint and model id (`ModelRouteKey`), the key of
  every model fact.
- **Prompt total**: every input token a response's request was billed for,
  cached or not: uncached input + cache read + cache write.
- **Codex format**: one rendering behaviour of Codex's transcript (a table,
  a link, a diff), listed in §2.12.

## 1. The request against the code

The surveys this plan started from were checked against the code. Where the
code says otherwise, the design follows the code:

| Premise | What the code says | Consequence |
| --- | --- | --- |
| [9] Tool-name normalisation breaks `/recap` | It does not. `request_structured` (`crates/agent/src/plugins/structured_request.rs`) sends a native request with **no tools** wherever `structured_output_capability()` says `NativeJsonSchema`. That answer is per protocol (`crates/model-api/src/protocol.rs:79`: Responses and Chat Completions are native, Anthropic is not), not per route. The forced-tool names (`submit_agent_conversation_recap`, `…_title`) are valid. | Learn the rung per route, and fall back native → forced tool → text (§2.1). No tool-name change. |
| [3] Titles are missing | The `titles` plugin exists (`crates/agent/src/plugins/titles/`). The first message, cut to 36 characters, is the provisional title (`crates/agent/src/titles.rs`). The generated title replaces it through the same `request_structured`, so on this gateway generation fails silently and the first message stays. A title over 36 characters is refused, not trimmed, and generation is tried once. | [3] is B1's fix plus a tolerant title reader and a retry (§2.1). |
| [9] Three users of the native rung | Correct, but the approval reviewer (`crates/kernel/src/permission.rs:299`) has **two** rungs: native, else plain text, with no forced tool. Memory extraction (`crates/memory/src/generation.rs:107`) has native and forced tool. | The chain is per caller: the reviewer keeps two rungs and its strict reading (approval stays strict). |
| [8] Hoshimi reads more window fields | Only some are fields of a listing entry: Gemini `inputTokenLimit`, Mistral `max_context_length`, Alibaba `model_info.context_window`, Venice `model_spec.availableContextTokens`. Ollama's `model_info.*.context_length` comes from `/api/show`, and LM Studio's `loaded_context_length` from `/api/v0/models`: other requests. Hoshimi avoids `max_input_tokens` only in Qoder's signed private catalogue (`hoshimi/core/qoder/catalog.py`), not in generic listings. | B2 adds the four entry fields. The other two requests are out of scope (§5, D8). `max_input_tokens` stays first. |
| [8] The window is unknown | `session_model_metadata` (`crates/kernel/src/service/model_facts.rs:42-63`) resolves the route's facts, then overwrites the window with the session's stored copy. That copy is written only at creation, configuration and turn admission (`refresh_context_window`, `crates/kernel/src/service/context_window.rs`). Discovery lists a Provider when a session is bound to it (`crates/agent/src/model_discovery.rs`), in the background, so the listing usually lands after creation and often during the first turn. | Refresh at the listing, and at each response commit (§2.2). |
| [7] The TUI does not stop the clock | Neither layer does. The kernel completes a reasoning item only on its own `output_item.done` (`crates/kernel/src/service/sampling/stream/events.rs`), and the Responses adapter sends that done separately, after text may have started. The Chat Completions and Anthropic adapters close reasoning before text. `output_item_added` returns early for a tool call (no pending item), so "a tool call starts" is seen only at its completion. | Close the reasoning interval in the kernel, when any non-reasoning item starts or completes (§2.3). |
| [6] The cache numbers exist | `ModelUsage { input_tokens, output_tokens }` (`crates/model-api/src/model/output.rs:134`) drops every cache field in all three decoders. `cache_report` hard-codes `cached: 0` (`crates/kernel/src/service/compression/tools/mod.rs:305`). billion-context's panel is already ported (`crates/compression/src/report/panel.rs`: `cache_hit_stats`, the `█`/`░` bar, `format_hit_rate`), but nothing records per-request cache samples. | C1 decodes and records the cache fields; C2 reuses the ported panel helpers. |
| [5] The settings row needs a new kind | Correct: `reasoningEffort` is `SettingKind::Text { nullable: true }` (`crates/agent/src/service/settings/model_policy.rs:134`). The resolved reasoning fact is **one** winner with its source (`crates/kernel/src/model_metadata/resolve.rs`): a configured `reasoningEfforts` replaces the reported levels, never adds to them. | The options are the resolved fact's efforts when its source is Configured or Reported (§2.6). No new metadata accessor. |
| [1] `?` becomes rebindable | The keymap refuses a key that types text beside the composer (`crates/tui/src/keymap/resolve.rs:154`), so a default `?` cannot be bound as things are. Alt+← and Alt+→ are `move_word_left` and `move_word_right` (`keymap/actions.rs`). Two actions may share a key only when listed in `SHARED_KEYS`. | An "empty composer only" flag that lets text keys through, and `SHARED_KEYS` pairs (§2.7). |
| [1] Codex's overlay is two files | `bottom_pane/shortcut_overlay.rs` and `shortcut_help.rs` (at `tui/src/`, not `bottom_pane/`). On WSL Codex shows Ctrl+Alt+V for pasting an image. | Port both. |
| [2] A clipboard file outside the workspace is refused | It is not. `read_selected_binary` (`crates/workspace-fs/src/service/binary.rs`) resolves through `resolve_existing`, which refuses only protected roots. The standalone workspace has none (`crates/agent/src/standalone/workspace.rs`). | No bytes variant: the TUI writes the clipboard image to a temporary file and passes its path, as Codex does (§2.11). |
| [4] Scrollback cannot re-render written rows | Codex does it: on a width change `app/resize_reflow.rs` clears the terminal's scrollback and visible screen (`clear_scrollback_and_visible_screen_ansi`) and writes the transcript again from source-backed cells, under a row cap. | Port the reflow in both presentations (D19). |
| [4] Colour can be added in the TUI | It cannot: the execution service renders command output to plain text (`render_plain_text`, `crates/execution/src/service/mod.rs`; the `vt100` screen of `crates/execution/src/terminal_screen.rs` for a PTY) before the model, history or a presenter see it. Displays are computed from that committed plain output (`crates/contracts/src/tools/display.rs`). | The colour must come from the execution service as a display-only copy, live and persisted (§2.12, M7, D21). |
| [1] Ctrl+R searches persistent history | Ours keeps no prompt history across runs (`bottom_pane/chat_composer_history.rs`); Codex reads `history.jsonl`. | Search this run's messages and the user messages in the history store (D16). |
| [1] `!` reuses a path that exists | No facade or kernel path runs a command for the person. Codex runs it as a task outside the sandbox, without an approval, and records it as a `<user_shell_command>` fragment in the model's history (`core/src/tasks/user_shell.rs`, `core/src/user_shell_command.rs`). | A new turn kind and history item (§2.8, D14). |

## 2. Design

### 2.1 Structured output on any gateway [9][3]

**Rungs, per route.**

- `ModelStructuredOutputCapability` becomes three values, best first:
  `NativeJsonSchema`, `RequiredTool`, `Text`. Its per-protocol answer stays
  the starting point: Responses and Chat Completions native, Anthropic
  `RequiredTool`.
- A new learned fact, `ModelMetadata.structured_output:
  Option<Sourced<ModelStructuredOutputCapability>>`, decoded tolerantly like
  the others. It is the best rung the route still takes.
  - Set only by a refusal (`Learned`); it expires with the other learned
    facts (`LEARNED_TTL_MS`, 7 days, `crates/kernel/src/model_metadata/state.rs`).
  - `forget_learned_model_facts` forgets it too.
  - A learned rung only ever lowers the protocol's answer, the same safe
    direction as the other learned facts.
- `ModelSession` gains two methods with no-op defaults:
  `structured_output_capability_for(model)`, the rung for that route, and
  `learn_structured_output(model, rung)`.
  - `AgentService::create_private_model_session` becomes a method (`&self`)
    and wraps the session in one that answers them from the registry. So
    does the model a session is created with (`create_model_session`,
    `crates/kernel/src/service/session/create.rs`), whose sessions the
    reviewer samples (`interaction/approval/auto_review.rs`).
  - So memory (which depends on model-api only) and the agent crate's
    plugins learn through the trait, without reaching the kernel's
    registry.

**The refusal.** `classify_remote_model_error` (`crates/model-api/src/image_input.rs`)
gains `ModelError::UnsupportedStructuredOutput`, recognised narrowly in a new
`structured_output_refusal.rs` beside `reasoning_refusal.rs`:

- the status is 400 or 422;
- the message names `text.format`, `response_format`, `json_schema`,
  `structured output` or `structured_output`;
- and it carries a refusal wording ("not supported", "unsupported", "does
  not support", "unknown", "unrecognized", "not allowed", …) or the code
  `unsupported_parameter`/`unsupported_value`.

The forced tool's refusal is recognised the same way, naming `tool_choice`
(or `tools` with a refusal wording), as `UnsupportedToolChoice`. The recap
message matches (400, `text.format`, "not supported").

Both checks run before `is_unsupported_image_input`, which takes any 400
saying "only supports text"; that near miss is a classifier test. The new
variants reach every match on `ModelError` and `AgentError`:
`crates/kernel/src/model/error.rs`, `service/sampling/mod.rs`,
`service/sampling/stream/events.rs` and `service/turn/mod.rs`. In a turn's
own sampling they are ordinary model errors: only the structured helper
retries on them.

**The chain.** One shared helper in model-api, `request_json_object`
(`crates/model-api/src/structured_request.rs`, new), takes the request
(instructions, one user text, schema, names, timeout) and the rungs its
caller allows. It walks down from the route's rung:

- **Native** refused → learn `RequiredTool` (or `Text` when the caller
  skips the tool rung) and retry at once.
- **RequiredTool** refused → learn `Text` and retry.
- **RequiredTool** answered with text and no call (a gateway that ignores
  `tool_choice`) → parse the text, and learn nothing.
- **Text**: the instructions gain one sentence asking for only the JSON
  object. The answer is parsed tolerantly: a fenced block's content, else
  the first balanced `{…}` in it, else the whole text.

A success at a lower rung is what the learned fact records, so later
requests on that route start there and pay no failed request. Each retry is
logged at debug with the rung, never the content.

**Callers.**

- `request_structured` (titles, title suggestions, recap) becomes a thin
  wrapper over it with all three rungs.
- Memory extraction (`crates/memory/src/generation.rs`) uses it with all
  three rungs. Its reading stays as it is: tolerant per field.
- The reviewer keeps its two rungs (native, text) and its strict
  `parse_assessment_text`. It only gains the learned fact and the retry
  after a native refusal: approval stays strict whatever the source
  (AGENTS.md, "Decoding by input source").

**Titles.**

- `normalize_generated_title` becomes Codex's `parse_thread_title`
  (`app/thread_title.rs:417-441`), on the decoded `title`:
  - trim; strip surrounding `"'` and backticks and curly quotes; collapse
    whitespace; strip trailing `.?!`;
  - cut to 36 characters instead of refusing.
  - Codex's "starts with `{`" gate belongs to its JSON reading, which the
    text rung replaces.
- **Retry.** A title is generated at each user turn's admission while the
  session's title is still the provisional one. That is: source
  `Automatic`, and equal to `provisional_agent_title` of the session's first
  user message. No new title source (D6).
  - The `titles` observer then needs the turn's admission even when the
    session already has a title. The notification carries the current
    title, and the plugin decides.
  - The commit stays the compare-and-swap `replace_title`, so a title the
    person gave in the meantime is kept.
  - A generated title equal to the provisional one (a short first message)
    is indistinguishable from a failure. The plugin keeps, for the run, the
    sessions it generated a title for and does not retry those; after a
    restart such a session is tried once more.

### 2.2 The context window the metadata knows [8]

- **At a listing.** After `record_listing` (`crates/kernel/src/service/factory.rs`,
  `list_provider_models`), each session whose route is on the listed
  endpoint takes its window.
  - In its own commit: lock after persistence, `refresh_context_window`,
    revision, persist. This is `learn_context_window`'s pattern without a
    turn identity.
  - A session with a staged commit is skipped; its next commit takes it.
  - The commit raises the session's revision, as `learn_context_window`'s
    does. The TUI reads a fresh snapshot before each mutation
    (`agent_bridge/commands.rs`, `current`), so `/mode` or `/effort` after
    it is not stale; a host that caches a revision sees an ordinary bump.
  - `usage.context_window_tokens` follows, as `refresh_context_window`
    already does (`validate_persisted_usage` refuses a mismatch).
- **At each response.** The commit that records a response's usage
  (`crates/kernel/src/service/sampling/commit.rs`) calls
  `refresh_context_window` first. So a listing that landed mid-turn, while
  a commit was staged, is taken before the next step's compaction check.
- **`session_model_metadata`** keeps the session's window when it has one.
  Otherwise it returns the resolved one: a window known for a session that
  has none yet, for example an unbound session. It no longer replaces a
  known window with `None`.
- **Fields** (`crates/model-api/src/model_catalog/shapes.rs`,
  `CONTEXT_WINDOW_FIELDS`): add `inputTokenLimit`, `max_context_length`,
  `model_info.context_window` and `model_spec.availableContextTokens`, after
  the present ones. Mistral's `max_context_length` gets a documented
  example fixture; the others are cited from Hoshimi (`providers/cloud.py`,
  `catalog_profiles.py`, `station_catalogs.py`).
- Compaction reads `config.context_window_tokens`
  (`crates/kernel/src/service/compaction/admission.rs`), so it follows
  without change.

### 2.3 Thinking stops when the answer starts [7]

In the kernel, so every host and both presentations get it:

- `PendingReasoning` (`crates/kernel/src/service/sampling/stream/reasoning.rs`)
  gains a `closed: Option<u64>` state, set when the interval ends early, with
  its duration frozen.
- **Closing.** These events close every open, unclosed reasoning item of
  the response:
  - `OutputItemAdded` for an assistant message, a tool call or any other
    non-reasoning item;
  - `AssistantTextDelta`;
  - `OutputItemCompleted` of a non-reasoning item (a tool call is first
    seen there).

  Closing emits `ReasoningCompleted { duration_millis }` and calls
  `reasoning_item_completed` once, which ends the turn's thinking interval
  (`crates/kernel/src/model/reasoning.rs`, `item_completed`).
- **The late done.** The provider's later `output_item.done` for a closed
  item:
  - completes it with its summary, content and replay (`complete`) and
    persists them;
  - keeps the frozen duration;
  - emits no second `ReasoningCompleted` and no second `item_completed`.
- **Late deltas.** A reasoning delta for a closed item is kept in the
  item's text, not shown, and logged at debug. The entry is final, and the
  persisted text is what a replay shows.
- `finalize.rs`'s "unfinished reasoning item" error stays for an item that
  was neither closed nor completed. A closed item with no done uses its
  streamed text, as `complete_unfinished_reasoning` does for a stopped
  response.
- No TUI guard. The TUI's cell already ends at `ReasoningCompleted`
  (`crates/tui/src/chatwidget/turn.rs`, `on_reasoning_completed`), and the
  kernel fix covers every adapter.

### 2.4 Cache usage [6]

- **`ModelUsage`** gains `cache_read_tokens: Option<u64>` and
  `cache_write_tokens: Option<u64>` (`None`: not reported), with
  `#[serde(default)]`. `input_tokens` is defined as the prompt total.
- **Per decoder** (`crates/model-api/src/tolerant_decode.rs` `token_usage`
  and the Anthropic stream):

  | Protocol | Reported input | Cache fields | Prompt total |
  | --- | --- | --- | --- |
  | Responses | `input_tokens` includes cached | `input_tokens_details.cached_tokens` (read) | `input_tokens` |
  | Chat Completions | `prompt_tokens` includes cached | `prompt_tokens_details.cached_tokens`; DeepSeek's `prompt_cache_hit_tokens` (read) | `prompt_tokens` |
  | Anthropic | `input_tokens` excludes the cache | `cache_read_input_tokens`, `cache_creation_input_tokens` (write) | `input_tokens + read + write` |

  A cache field that does not decode is `None`, one bad field never failing
  the rest.
- **`AgentModelUsage`** (`crates/kernel/src/model/turn.rs`) gains
  `last_response_cache_read_tokens`, `last_response_cache_write_tokens`,
  their cumulative sums, and `cache_reporting_responses`, the count of
  responses that reported any cache field. All are `#[serde(default)]`, so
  old persisted usage decodes. `next_model_usage`
  (`crates/kernel/src/service/sampling/mod.rs`) fills them;
  `validate_persisted_usage` adds "cumulative ≥ last" for them.
- **Context consumed** stays `last input`, which is now the prompt total
  everywhere.
  - This deliberately raises the figure for Anthropic: today it counts only
    the uncached part, so the percentage shown and the automatic-compaction
    trigger sit too low.
  - A session's compaction may therefore start earlier than it did. This is
    a change of meaning in `history_committed.usage` (json-events
    "Compatibility": a minor version before 1.0).
- **Goals** (`crates/agent/src/goals.rs`) count `cumulative_input_tokens`,
  which for Anthropic now includes cache reads (D10).
- `cache_report` uses the recorded read/write instead of `cached: 0`.

### 2.5 `/context` [6]

**Capture.** When a step's request is built
(`crates/kernel/src/service/sampling/request.rs`, after `prompt.rs` places
the inputs), the kernel keeps a `AgentContextComposition` in the session's
runtime state, not persisted. Each group holds estimated tokens, from the
existing estimator: `estimate_tokens` (`crates/contracts/src/tokens.rs`) and
`estimate_model_request_tokens`'s image rule
(`crates/kernel/src/service/sampling/estimate.rs`). The groups:

- base instructions (`request.instructions`);
- tools, split by `AgentPlannedTool.provider_id`: the kernel's and built-in
  plugins' tools vs MCP and installed plugins' tools, each with its count;
- each context section, by its `ContextFragment.kind`, in prompt order;
- conversation:
  - user messages;
  - assistant messages;
  - reasoning replay;
  - tool calls and outputs;
  - continuation context;
  - compaction summaries;
- images (count and estimated tokens).

**Facade read.** `AgentApplicationService::session_context(session_id)`
returns:

- that composition and when it was captured;
- the session's window (§2.2);
- the provider-reported prompt total of that request when its response has
  come back;
- the cache figures (§2.4);
- with no request yet, an estimate of the next request, built as the
  standalone compaction's check of the next request builds it
  (`render_prompt`, `crates/kernel/src/service/sampling/prompt.rs`, then
  the projected step request). No contributor prepares for it (no sampling
  step), so plugin-prepared values are missing and the estimate says so. No
  tool runs and no model is called. If that cannot be built (no route, no
  tool plan yet), a note replaces the composition.

**Command.** `/context` (`crates/tui/src/commands/context.rs`, new, after
`/status` in `commands/reports.rs`). It works during a task, since it reads a
snapshot.

- The cell (`crates/tui/src/history_cell/context.rs`, new) has billion-context's
  layout (`kernel/src/panel/panel.ts`, already in
  `crates/compression/src/report/panel.rs`):
  - one row per group: label, 20-cell `█`/`░` bar, percent of the window,
    tokens;
  - a "Free" row: the window minus the request;
  - the estimate's label: "estimated: about 4 bytes a token", with the
    provider's total beside it;
  - then `Cached tokens`, `Uncached tokens`, and `Cache hit rate: X% last ·
    Y% session — N of M prompt tokens from cache (K requests)`.
- `bar`, `cache_hit_stats`, `format_hit_rate` and `format_compact_tokens`
  are made public in `hanafubuki-agent-compression` and reused, not copied.
- A provider that reports no cache field shows "not reported", never 0%.

### 2.6 `/effort` and the settings row [5]

**Options**, one list for the command, the row and Alt+, / Alt+.
(`crates/tui/src/model_facts.rs` gains `effort_options`):

- The resolved reasoning fact of the model: `session_model_metadata` for the
  command, `model_metadata` of the configured default model for the row.
  Its efforts count when its source is `Configured` (the person's
  `models."<id>".reasoningEfforts`) or `Reported` (the Provider's listing).
- `Catalog` (models.dev) and `BuiltIn` do not count. Neither does
  `Supported { efforts: [] }` (reasoning reported, levels not).
- `Unsupported` lists no level, with the note "the Provider refused the
  reasoning parameter" when the fact is learned.
- Always: **Default** (unset: the Provider's default) and **Custom…**
  (free text, `ModelReasoningEffort::parse`, the review picker's
  custom-prompt view: `crates/tui/src/commands/review/picker.rs`,
  `commands/prompt.rs`).
- The current value and the model's default (`Supported.default`) are
  marked. A current value outside the list is shown as custom and selected.

**`/effort`** (`crates/tui/src/commands/effort.rs`, new; Codex
`chatwidget/model_popups.rs:496-669` for the list):

- Bare: the picker. `/effort <value>` and `/effort default` set directly.
- It calls `set_session_reasoning_effort` (agent wrapper:
  `crates/agent/src/service/provider/selection.rs`), new in the TUI.
- Not during a task: the kernel refuses a mid-turn change
  (`TurnAlreadyRunning`), so the command is `during_task: false` with
  "Change the effort after this turn, or interrupt it."
- A value the model's list does not have is still sent. The kernel narrows
  it per route already (`crates/kernel/src/service/reasoning.rs`).

**The settings row.**

- A new kind, `SettingKind::ChoiceOrText { values, maximum_chars,
  nullable }` (`crates/foundation/src/settings/mod.rs`): choices, a typed
  value and unset.
  - `read` and `check` accept any text within the bound, like `Text`. The
    value's own rule stays where it is today: `model_policy.rs`'s write
    validates the effort with `ModelReasoningEffort::parse`, and the center
    shows its refusal. No parser hook on the spec.
- `reasoningEffort` (`crates/agent/src/service/settings/model_policy.rs`)
  uses it. Its `values` are filled at catalog time from the configured
  default model's metadata, by the rule above, in `describe_known`, which
  already reads that metadata for the row's summary.
- Every match on `SettingKind` gains the arm:
  - `foundation/src/settings/read.rs`;
  - `packages/src/options/lower.rs` (never produced by a guest schema:
    rejected there);
  - `tui/src/settings_center/{editor,form,render,values,main_page}.rs`;
  - `tui/src/commands/plugins/options.rs`;
  - `config/src/settings/*`;
  - the serialized shape test in `crates/foundation/tests/settings.rs`.
- The configuration reference describes the row; `cargo xtask
  check-config-reference` holds.

### 2.7 The shortcut overlay and the new actions [1]

**Overlay.** Port `bottom_pane/shortcut_overlay.rs` and `shortcut_help.rs`
as `crates/tui/src/bottom_pane/footer/shortcut_overlay.rs` and
`crates/tui/src/shortcut_help.rs`, replacing `shortcut_overlay_lines` and
`build_columns` (`crates/tui/src/bottom_pane/footer.rs:172-264`):

- Three titled groups, Codex's labels and order, minus Voice.
  - Every key is read from the keymap's hints (`keymap::hint`), so a
    remapped action shows its key, and an unbound one is left out.
  - Fixed keys show as fixed: `/`, `@`, `!`, Ctrl+C, PgUp/PgDn.
- Paste image shows Ctrl+Alt+V on WSL, as Codex does: `paste_image` has
  both defaults, and the overlay picks the WSL one there.
- Tab's label is "Queue message" while a task runs, else "Send message";
  Ctrl+C's is "Interrupt" while one runs, else "Quit".
- Three, two or one column by width (Codex's measurement).
- When rows are cut, a "… more: /keymap" note.
- Footer: `/keymap customize`.

**Actions** (`crates/tui/src/keymap/actions.rs`, 66 → 75: K1 adds the
first six, K3, K4 and I1 one each). Each is rebindable under `tui.keymap`:

| Action | Context | Default | Does |
| --- | --- | --- | --- |
| `show_shortcuts` | composer | `?` | Toggles the overlay on an empty composer |
| `cycle_mode` | chat | `shift-tab` | Default ↔ Plan, as `/mode` (`commands/mode.rs`) |
| `reasoning_down` / `reasoning_up` | chat | `alt-,` / `alt-.` | Steps the session's effort through §2.6's list |
| `previous_agent` / `next_agent` | composer | `alt-left` / `alt-right` | On an empty composer: shows the previous/next of the main session and its subagents |
| `external_editor` | composer | `ctrl-g` | K4 |
| `history_search` | composer | `ctrl-r` | K3 |
| `paste_image` | composer | `ctrl-v`, `ctrl-alt-v` | I1; Ctrl+Alt+V for WSL, where the terminal takes Ctrl+V |

- **Empty composer only.** `ActionSpec` gains `empty_composer_only: bool`
  (`show_shortcuts`, `previous_agent`, `next_agent`). Such an action may
  take a text key: the text-key rule of `check_binding` skips it, since it
  is read only when there is no text to type into. Its handler checks
  emptiness as `agents_navigation.rs` does.
- **Shared keys.** `SHARED_KEYS` gains `(previous_agent, move_word_left)`
  and `(next_agent, move_word_right)`. On an empty composer the agent
  action wins; word motion has nothing to move over there.
- **Collisions checked against today's defaults:**
  - `alt-,` and `alt-.` are free; `alt-shift-,` and `alt-shift-.` are
    `transcript_top`/`transcript_bottom`, distinct as keys.
  - The keymap's matching must not fold Shift away for punctuation. A test
    pins `alt-,` vs `alt-<` vs `alt-shift-,` as reported by crossterm with
    and without the enhanced protocol.
  - `ctrl-g`, `ctrl-r` and `ctrl-v` have no default today. `shift-tab` is
    used only inside forms and the command center, which take the keys
    while open.
- **Subagent cycling.**
  - The order is the main session, then its children in `/subagents`'
    order (`crates/tui/src/commands/subagents/`). The step wraps around.
  - Showing one is the existing `SecondKind::Child` path
    (`crates/tui/src/app/workspace/mod.rs`); the main session is shown by
    `focus_pane_left`'s path.
  - With no subagent: "This session has no subagents."
- **Effort stepping.**
  - It anchors an unset effort to the model's default, else the first
    level, as Codex's `chatwidget/reasoning_shortcuts.rs`, and stops at the
    ends ("Reasoning is already at the highest level (High).").
  - With an empty list, it says "No reasoning levels are known for this
    model; use /effort to type one." During a turn it says what `/effort`
    says.
- Docs: `terminal-interface#keys` and `#keymap`, both languages. The
  "stay as they are" sentence loses `?`.

### 2.8 `!` shell commands [1]

- **The composer.** A submitted message starting with `!` (Codex:
  `chatwidget/input_submission.rs:38-70`, `chat_composer.rs` ~3588)
  becomes `Op::UserShellCommand(String)`.
  - `!` alone shows Codex's help cell.
  - The composer shows a shell-mode hint while the draft starts with `!`.
  - The message goes into the in-run history as typed.
- **The facade and the kernel.** `run_user_shell_command(session_id,
  command)` admits a turn of a new kind, `AgentTurnKind::UserShellCommand`,
  that samples no model:
  - The kernel invokes the planned `exec_command` handler through the tool
    registry, like any call (the kernel reaches plugins only through the
    contracts registries).
    - Its call id is a kernel-reserved `user-shell-<n>`, and its origin
      `User`.
    - Its approval is decided at once with `ApprovalDecisionSource::User`:
      the person typed it. The approval rules still evaluate it, so
      `execution.git-clone` still asks.
  - It runs in the session's sandbox and through the Agent's
    `ExecutorManager`: process ownership, stop sweeps and reaping as for
    the model's commands.
  - **The cost of the sandbox.** No model is there to ask for escalation,
    so a command the sandbox blocks (`!npm install` with the network off,
    a push that writes outside the writable roots) fails as the operating
    system reports it. The entry then adds "`!` commands run in the
    session's sandbox (/permissions)". Under a `full-access` sandbox
    nothing is blocked.
  - Output streams as `ExecutionProgress` with its call id, so the TUI's
    command entry (T6) shows it live. Esc interrupts it as a turn.
- **History.** A new item, `ConversationItem::UserShellCommand { command,
  cwd, exit_code, duration_millis, output, truncated }`.
  - The output is the model-visible plain text, bounded like a tool
    output.
  - It is lowered to the model as a user-role context fragment,
    `<user_shell_command>` with the command, exit code, duration and
    output, as Codex does. The next request's goldens change on purpose,
    with a new golden for it.
  - Replay, export, the transcript and `exec --json` (`history_committed`
    items) show it.
  - Every match on `ConversationItem` gains its arm, found by grepping
    `ConversationItem::ToolOutput`: the estimator
    (`sampling/estimate.rs`), compaction and its replacement history,
    hierarchical compression, the reviewer's evidence (`permission.rs`),
    backtrack, export, history validation and `/context`'s groups (as
    conversation: tool calls and outputs).
- **During a running turn** it is refused: "A turn is running; wait for it
  or interrupt it first." Codex's auxiliary mode is left out (D15).

### 2.9 Ctrl+R history search [1]

- Port `bottom_pane/chat_composer/history_search.rs` and
  `history_search_draft.rs` into `crates/tui/src/bottom_pane/chat_composer/history_search/`.
  Codex's vim states are left out.
  - Enter starts idle; typing restarts traversal newest-first.
  - Ctrl+R/Up older, Ctrl+S/Down newer, over unique matches.
  - Enter accepts the preview as an editable draft; Esc or Ctrl+C restores
    the saved draft.
- **Sources** (D16): this run's sent messages (`ChatComposerHistory`), then
  the user messages of the Agent's sessions, newest first. The latter come
  from a new bounded facade read, `recent_user_messages(limit, before)`,
  over the history store (at most 1,000, deduplicated).
  - Read lazily, off the UI thread, on the first Ctrl+R of a run.
  - No new history file.

### 2.10 Ctrl+G external editor [1]

- Port `external_editor.rs` (Codex) as `crates/tui/src/external_editor.rs`.
  - It reads `VISUAL`, then `EDITOR`, split with `shlex` (Windows: `which`
    for `.cmd` shims).
  - It writes the draft to a temporary `.md` file, suspends the TUI
    (leaves raw mode and the alternate screen), waits for the editor, and
    puts the result back as the draft, its placeholders kept (Codex's
    rehydration of pending pastes).
  - Neither variable set: "Set VISUAL or EDITOR to edit the message in an
    editor."
- **The process** is the person's foreground program, which the TUI waits
  for with the terminal handed over, as `/diff` runs `git` today
  (`crates/tui/src/get_git_diff.rs`). It is not an Agent-run process, so it
  is not under `ExecutorManager` (D17).
- `shlex` and `which` are new dependencies: `about.toml` and the notices
  (`cargo xtask check` "third-party notices") must accept them.

### 2.11 Image paste [2]

- **Clipboard.** Port `clipboard_paste.rs` (Codex), including its WSL
  fallback (PowerShell `Get-Clipboard -Format Image` through
  `powershell.exe`), as `crates/tui/src/clipboard/paste.rs`.
  - `arboard` gains its `image-data` feature.
  - The image is encoded to PNG (the `image` crate, already a workspace
    dependency, `png` feature) into a temporary file under the run's temp
    directory.
  - This supersedes `plan-codex-tui-port.md` §8's "no WSL clipboard route"
    for pasting.
- **Ctrl+V** (`paste_image`).
  - A clipboard without an image falls through to the terminal's paste.
  - An empty bracketed paste (what Windows Terminal sends for an image)
    also tries the clipboard image, as Codex's `chatwidget/clipboard.rs`
    does.
- **Pasted paths.** A paste that is one existing image file path (quoted,
  `file://`, or with escaped spaces; Codex's normalisation) attaches the
  file instead of inserting text.
- **Composer.** Port `chat_composer/attachment_state.rs`:
  - `[Image #N]` text elements;
  - deleting one removes its attachment, and the rest renumber;
  - a message may be images only.
- **Submission.** `Op::UserInput` carries `images:
  Vec<DesktopAgentImageInput>` (`path`, `display_name`, `detail: auto`),
  passed to `AgentTurnRequest.images` and `AgentSteeringRequest.images`
  (`crates/tui/src/agent_bridge/mod.rs:653`, `steering.rs:43`).
  - The path is read by `prepare_and_stage_images`
    (`crates/agent/src/attachments.rs`), which accepts a path outside the
    workspace (§1).
  - The temporary file is removed once the submission is accepted (the
    staged copy is durable), or when its placeholder is deleted.
- **Gate.** Before sending, the session's image capability
  (`effective_input_capabilities.image_input`, through
  `session_model_metadata`) is checked. A model known not to read images
  keeps the draft with "This model does not read images (…); remove the
  image or switch with /model." Unknown sends; the kernel's own fallback
  applies.
- **Backtrack** (`crates/tui/src/app/backtrack.rs:338-341`). An edited
  message's `restored_images` (durable attachment refs) come back as
  `[Image #N]` with `attachment` inputs, and the "not restored" note goes.
- **Transcript.** A user message shows `[Image #N: name]` per image (Codex's
  label), in both presentations.
- **Platforms.** Linux X11/Wayland is tested by hand where available. The
  Windows, macOS and WSL clipboards cannot be verified from Linux: their code
  is covered only by unit tests of the non-clipboard logic (path
  normalisation, placeholders and renumbering, PNG encoding, the WSL
  command line), and reported so.

### 2.12 Rendering: every Codex format [4]

The layer today is an older, reduced snapshot:

- `markdown.rs` (14 lines), `markdown_render.rs` (584): only strikethrough
  is enabled; a link prints as ` (url)`.
- `markdown_stream.rs` (111) and `streaming/controller.rs` (189) re-render
  the whole buffer, one line per 50 ms tick.
- `AgentMessageCell` stores lines, not source (`history_cell/mod.rs:284`).
- No OSC 8; `wrapping.rs` (401) is not URL-aware.
- Command output is plain.

Codex's is ~25,000 lines: `markdown_render.rs` 3,329 with 13 submodules,
`markdown_stream.rs` 905, `streaming/` 7 modules, `wrapping.rs` 2,096,
`terminal_hyperlinks.rs` 1,452, `diff_render.rs` 2,745 and the `codex-mermaid`
crate (2,428).

Every format:

| Codex format | Codex source | Decision |
| --- | --- | --- |
| Headings, block quotes, fenced code with highlighting, emphasis, strong, strikethrough, inline code, rules, ordered and unordered lists | `markdown_render.rs` | **Already present**; kept, rewritten with the port |
| Footnotes, images in Markdown, raw HTML, metadata blocks | `markdown_render.rs` (ignored there) | **Ignored, as in Codex**; pulldown 0.13's definition lists, superscript and subscript stay disabled |
| Pipe tables, column fitting, `tables` preference | `markdown_render.rs`, `table_detect.rs` | **Port** (M1) |
| Key/value fallback for tables too wide to scan | `markdown_render/table_key_value.rs` | **Port** (M1) |
| Tables inside ` ```md ` fences unwrapped | `markdown.rs` | **Port** (M1) |
| Literal tables when disabled | `markdown_render/source_tables.rs` | **Port** (M1) |
| Task lists `[ ]`/`[x]`, `lists` preference | `markdown_render/task_lists.rs` | **Port** (M1) |
| Tight/loose list spacing | `markdown_render/list_spacing.rs` | **Port** (M1) |
| Web links shown as their text, URL kept for OSC 8 | `markdown_render/web_links.rs` | **Port** (M1, OSC 8 in M4) |
| Local file links shown as cwd-relative paths with `:line` | `markdown_render/local_links.rs` | **Port** (M1) |
| Rendering preferences (`mermaid`, `math`, `tables`, `lists`) | `markdown_render/preferences.rs`, `config/src/tui_rendering.rs` | **Port** (M1) as `tui.rendering.*` settings with specs, behind a `parking_lot::RwLock` (not Codex's std lock) |
| Source-backed message cells; consolidation at stream end | `history_cell/messages.rs`, `app/agent_message_consolidation.rs` | **Port** (M2) |
| Re-render on resize | `app/resize_reflow.rs` | **Port** (M2), both presentations: Fullscreen lays out again; Scrollback clears and writes again under a row cap (D19) |
| Incremental block rendering, stable and tail regions | `markdown_stream.rs`, `streaming/render.rs`, `streaming/controller.rs` | **Port** (M3) |
| Adaptive catch-up chunking, commit ticks | `streaming/chunking.rs`, `streaming/commit_tick.rs` | **Port** (M3) |
| Table holdback while rows arrive | `streaming/table_holdback.rs` | **Port** (M3) |
| Open-fence highlighting while streaming | `streaming/code_fence.rs`, `render/highlight_streaming.rs` | **Port** (M3) |
| Bounded prose previews of an unterminated line | `streaming/prose_preview.rs` | **Port** (M3), minus follow-up directives |
| OSC 8 hyperlinks carried beside the text | `terminal_hyperlinks.rs`, `insert_history.rs` | **Port** (M4) |
| URL-aware wrapping | `wrapping.rs` | **Port** (M4) |
| Math as Unicode (inline and display) | `markdown_render/math.rs`, `math/render.rs` | **Port** (M5) |
| Mermaid as text diagrams | `markdown_render/mermaid.rs`, `codex-rs/mermaid` | **Port** (M6) as `crates/tui/src/mermaid/`, not a new crate |
| ANSI colour in command output | `codex-rs/ansi-escape` (`ansi-to-tui`), exec cells | **Port** (M7): SGR colour and attributes only |
| JSON tool output pretty-printed | `history_cell` (MCP/function output) | **Port** (M8) |
| MCP image, audio and resource blocks as summaries | `history_cell/mcp_result.rs` | **Port** (M8): a one-line summary each (`image/png · 34 KB`, resource URI), never the bytes |
| Diff: theme-aware tints, palettes per colour depth, style-preserving wrap, hunk-wide highlighting, update line numbers | `diff_render.rs` | **Port** (M8); line numbers of an updated file need each hunk's start line from the patch applier (`AgentPatchPreview`, display-only) |
| Inline images through terminal graphics protocols | `history_cell` image previews | **Not ported**: no graphics protocol in our terminal layer; `plan-codex-tui-port.md` §8 |
| File citations and `:codex-followup[…]` directives | `markdown_render/inline_directives.rs`, `assistant_directives.rs` | **Not ported**: Codex-service syntax our models are not prompted to write |
| Voice, pets, sparkles | `chat_composer/sparkle*.rs`, voice | **Not ported**: not requested; voice excluded by [1] |

**pulldown-cmark 0.13 vs 0.10.** Ours is 0.13 (`crates/tui/Cargo.toml`).
`Tag::BlockQuote` carries a kind; `TagEnd` names differ;
`Event::InlineMath`/`DisplayMath` exist behind `ENABLE_MATH`. The port keeps
Codex's math masker, which works on the source before parsing, and leaves
`ENABLE_MATH` off (D20). Every ported match is adapted and covered by
Codex's tests, ported with it.

**Module size.** Codex's large files are split along their own submodules
and responsibilities, each under 600 lines:

- `markdown_render/` (writer, blocks, inline, tables, lists, links);
- `streaming/` as Codex's seven files, with `controller.rs` split into
  state, commit and preview;
- `wrapping/` (words, URLs, spans);
- `terminal_hyperlinks/` (annotations, emission);
- `diff_render/` (theme, palette, wrap, highlight);
- `mermaid/` as Codex's files.

No exception is planned; any becomes a report.

**Colour (M7)**:

- The execution service keeps, beside the plain text, a display-only
  styled copy: runs of text with SGR state (fg, bg, bold, dim, italic,
  underline, reverse). Every other control sequence is dropped exactly as
  today.
  - From the `vt100` screen's cell attributes for a PTY.
  - From an SGR-tracking variant of `render_plain_text` for pipes
    (`crates/foundation/src/terminal_text.rs`).
- It is bounded like the output and never reaches the model or its
  goldens.
- **Live**: `ExecutionProgress` carries it.
- **Replay**: the committed tool output keeps it as display metadata,
  decoded tolerantly and absent from older records, and the `Command`
  display gains `styled: Option<AgentStyledText>`.
- The TUI renders runs with ratatui styles. The garbled-output work's
  stripping (`strip_terminal_controls`) stays for every other path.

## 3. Milestones

Strictly sequential: each milestone is one commit on main, pushed, with
green CI, before the next starts:

**B1 → B2 → B3 → C1 → C2 → E1 → K1 → K2 → K3 → K4 → I1 → M1 → … → M8**

Each milestone, when done, records in `todo.md`:

- what landed and any deviation from this plan;
- the validation commands that ran and those skipped;
- the CI run's URL (Linux, Windows, macOS from the workflow, never claimed
  from a Linux run);
- whether goldens changed;
- what Hanafubuki must change at its next pin;
- any module-size exception added or raised.

Every milestone runs AGENTS.md's validation matrix (`cargo fmt --all
--check`; `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
--all-features`; `cargo clippy --workspace --all-targets --all-features -- -D
warnings`; `cargo xtask check`; `cargo test --workspace --all-features`) and,
when it touches `site/`, the site's (`bun install --frozen-lockfile`, `bun
run format:check:site`, `bun run site:check`, `bun run site:test`,
`VITE_BASE_PATH=/hanafubuki-agent/ bun run site:build`, every page
prerendered). Each changes the docs of what it changes, in both languages.

### Fixes

- **B1 — structured output on any gateway** (§2.1).
  - Files:
    - `model-api`: `structured_output_refusal.rs` (new),
      `structured_request.rs` (new), `image_input.rs`, `port.rs`,
      `protocol.rs`, `model_metadata.rs`;
    - `kernel`: `model_metadata/{mod,resolve,state}.rs`,
      `service/factory.rs`, `service/model_facts.rs`, `permission.rs`;
    - `memory/src/generation.rs`;
    - `agent`: `plugins/structured_request.rs`, `plugins/titles/*`,
      `plugins/recap/mod.rs`, `conversation_reader.rs`,
      `memory/supervisor.rs`.
  - Tests:
    - The refusal classifier on the recap's message and near misses (a 400
      naming `text` without a refusal, a 500).
    - With the fake server (`crates/agent/src/test_support/model_server.rs`,
      a new refusal predicate):
      - a gateway refusing `text.format` → the recap succeeds through the
        forced tool;
      - one refusing `text.format` and `tool_choice` → it succeeds through
        text;
      - a second request starts at the learned rung (one request
        captured);
      - the fact expires after 7 days and is forgotten by
        `forget_learned_model_facts`;
      - the reviewer on a native refusal falls back to text and stays
        strict on a malformed answer;
      - memory extraction on both refusals.
    - Titles: Codex's normalisation cases; a failed first generation
      retried at the second turn; a user rename stops the retries.
    - Failing first: the recap test reproduces the reported 400 before the
      fix.
  - Docs:
    - `sessions` (titles: generated, retried, first message as fallback);
    - `providers-models#forget` (the learned rung);
    - `internals` (the rungs);
    - `terminal-interface` (`/recap`).
  - Acceptance: `/recap` and titles work against a gateway that refuses
    structured output, at most one failed request per route per week.
  - Hanafubuki: `ModelStructuredOutputCapability` gains `RequiredTool`
    (`TextOnly` → `Text`); `ModelMetadata` gains `structured_output`;
    `create_private_model_session` takes `&self`.
- **B2 — the context window the metadata knows** (§2.2).
  - Files:
    - `kernel/src/service/{factory,context_window,model_facts}.rs`;
    - `kernel/src/service/sampling/commit.rs`;
    - `model-api/src/model_catalog/shapes.rs` and its listing fixtures.
  - Tests:
    - A listing recorded after the session's creation and before any turn:
      `/status`'s metadata and the compaction threshold both use it.
    - The same listing landing during the first turn, with a staged
      commit: taken at the response commit, and `usage.context_window_tokens`
      equal to the config's.
    - A session with no route shows the resolved window.
    - Each new field, from a fixture.
    - Failing first.
  - Docs: `providers-models#compaction` (when a listed window is taken).
  - Acceptance: `/status` shows the listed window as soon as the listing is
    recorded, and unknown only when no source has one.
- **B3 — thinking stops when the answer starts** (§2.3).
  - Files: `kernel/src/service/sampling/stream/{events,reasoning,finalize}.rs`.
  - Tests:
    - Responses-shaped events: reasoning added → text delta → reasoning
      done → completed. `ReasoningCompleted` comes before the first text
      delta with the frozen duration, then nothing more for that item.
    - The persisted item carries the done's summary and replay.
    - The turn's `turn_completed_millis` counts the interval once.
    - The same for a tool call completed without an added event.
    - A late reasoning delta after close is not shown.
    - The TUI: the entry reads `Thought · Ns` while the answer streams
      (snapshot).
    - Failing first.
  - Docs: `terminal-interface#thinking` (when the timer stops), `internals`.
  - Acceptance: no thinking timer runs after the `Worked for …` separator,
    on any protocol.

### Context and cache

- **C1 — cache usage** (§2.4).
  - Files:
    - `model-api`: `model/output.rs`, `tolerant_decode.rs`,
      `anthropic_messages/mod.rs`, `chat_completions/mod.rs`,
      `responses/decode.rs`;
    - `kernel`: `model/turn.rs`, `service/sampling/mod.rs`,
      `service/validation/history.rs`, `service/compression/tools/mod.rs`;
    - `agent/src/goals.rs`; `tui/src/agent_bridge/status.rs`.
  - Tests:
    - Each decoder with and without cache fields, and a malformed one.
    - The prompt total per protocol (no double count).
    - Old persisted usage decodes.
    - Anthropic's context consumed includes cache reads, and its threshold
      test moves accordingly.
    - `cache_report` shows a hit.
  - Docs: `json-events` (`usage`'s new fields, and `contextConsumedTokens`
    now the prompt total for Anthropic), `providers-models#compaction`.
  - Acceptance: the todo entry states the Anthropic change of meaning and
    the earlier compaction.
  - Hanafubuki: the new `ModelUsage` and `AgentModelUsage` fields.
- **C2 — `/context`** (§2.5).
  - Files:
    - `kernel/src/service/sampling/{request,prompt,estimate}.rs`, a
      composition type in `kernel/src/model/`;
    - `agent/src/service/projections.rs` (the facade read);
    - `compression/src/report/panel.rs` (made public);
    - `tui/src/commands/{mod,context}.rs`, `tui/src/history_cell/context.rs`,
      `tui/src/agent_bridge/commands.rs`.
  - Tests:
    - The composition of a scripted request: each group's estimate equals
      the estimator's on that part, and their sum the request's estimate.
    - Before any request, the estimate is labelled as such.
    - Snapshots at 100 and 50 columns, with and without cache reporting.
    - `COMMANDS`' pinned order updated.
  - Docs: `terminal-interface#commands`, `embedding` (the facade read).
  - Acceptance: `/context` lists every group, the free space and the cache
    lines, during a task too.

### Effort

- **E1 — `/effort` and the settings kind** (§2.6).
  - Files:
    - `foundation/src/settings/{mod,read}.rs`;
    - `agent/src/service/settings/model_policy.rs`;
    - every match site of §2.6;
    - `tui/src/commands/{mod,effort}.rs`, `tui/src/model_facts.rs`.
  - Tests:
    - Options from a reported listing, from configured efforts, from
      models.dev only (none), from none (Default and Custom only).
    - A custom value refused by `parse` stays in the field with its error.
    - Mid-turn refusal message.
    - The settings row's values and a typed value round-trip.
    - `config-reference` check.
    - Picker snapshots.
  - Docs: `terminal-interface#commands`, `providers-models#reasoning`,
    `config-reference`.
  - Acceptance: `/effort`, the Settings Center row and Alt+,/. offer the
    same options for the same model; with no reported or configured
    levels, only Default and Custom.
  - Hanafubuki: `SettingKind::ChoiceOrText` in the catalog's JSON.

### Keys

- **K1 — the overlay and the new actions** (§2.7).
  - Files:
    - `tui/src/bottom_pane/footer.rs` and `footer/shortcut_overlay.rs`;
    - `tui/src/shortcut_help.rs`;
    - `tui/src/keymap/{actions,resolve}.rs`;
    - `tui/src/bottom_pane/chat_composer/{mod,agents_navigation}.rs`;
    - `tui/src/chatwidget/` (mode cycle, effort stepping);
    - `tui/src/app/workspace/keys.rs`.
  - Tests:
    - Overlay snapshots at 140, 90 and 50 columns.
    - A remapped `history_search` shows its key; an unbound one is absent.
    - `?` mid-text types `?`.
    - Alt+← on an empty composer cycles and with text moves by word.
    - Shift+Tab toggles the mode.
    - Effort stepping at both ends and with an empty list.
    - The punctuation-modifier test.
    - `ACTIONS` order and the conflict tests.
    - The overlay lists Ctrl+R, Ctrl+G and Ctrl+V only once K3, K4 and I1
      add their actions: an action not yet present is left out, as an
      unbound one is.
  - Docs: `terminal-interface#keys`, `#keymap`.
  - Acceptance: `?` shows the three groups with the keys in force, and
    every listed key present by then does what its label says.
- **K2 — `!` shell commands** (§2.8).
  - Files:
    - `kernel`: `model/conversation_item.rs`, a new turn kind and its
      admission, the lowering, `service/tool_execution/`;
    - the facade (`agent/src/service/turns.rs`);
    - `tui/src/app_event.rs`, `tui/src/chatwidget/`,
      `tui/src/history_cell/tool/command.rs`;
    - export.
  - Tests:
    - A command runs, streams, is committed and lowered (new golden).
    - It is refused during a turn.
    - Esc interrupts it and its process is terminated and forgotten only
      after confirmation.
    - The git-clone rule still asks.
    - Resume replays it.
  - Docs: `terminal-interface#composer`, `json-events`, `sessions#export`,
    `permissions` (a typed command is the person's approval), `internals`.
  - Goldens: regenerated on purpose (the new item's lowering).
  - Acceptance: `!ls` shows its output live, and the model's next request
    carries it.
  - Hanafubuki: the new `ConversationItem` and turn kind.
- **K3 — Ctrl+R** (§2.9).
  - Files: `tui/src/bottom_pane/chat_composer/history_search/`, a facade
    read in `agent/src/service/history.rs`.
  - Tests: Codex's history-search tests ported (traversal, dedupe, accept,
    cancel restoring the draft); the facade read bounded and newest-first.
  - Docs: `terminal-interface#composer`.
  - Acceptance: Ctrl+R finds a message sent in an earlier run.
- **K4 — Ctrl+G** (§2.10).
  - Files: `tui/src/external_editor.rs`, the composer, `Cargo.toml` (`shlex`,
    `which`), `about.toml` if needed.
  - Tests: editor resolution (VISUAL over EDITOR, quoting, empty); round
    trip through a fake editor script on Unix; placeholders kept. Windows'
    `.cmd` resolution is unit-tested only.
  - Docs: `terminal-interface#composer`.
  - Acceptance: Ctrl+G edits the draft in `$VISUAL` and the TUI comes back
    intact.

### Images

- **I1 — image paste** (§2.11).
  - Files:
    - `tui/src/clipboard/paste.rs`;
    - `tui/src/bottom_pane/chat_composer/attachments.rs` (from Codex's
      `attachment_state.rs`);
    - `tui/src/app_event.rs`;
    - `tui/src/agent_bridge/{mod,steering}.rs`;
    - `tui/src/app/backtrack.rs`;
    - the user-message cell;
    - `tui/Cargo.toml` (`arboard` `image-data`).
  - Tests:
    - Placeholders, deletion and renumbering; an image-only message.
    - A steer with an image; backtrack restoring images.
    - A pasted path in each form.
    - The capability gate's message.
    - A submission reaching `AgentTurnRequest.images` and the model request
      (fake server), from a temporary file outside the workspace roots
      (§1's finding, pinned). The temporary file is removed after
      acceptance.
    - Clipboard reading itself is not unit-testable; the platforms are as
      §2.11 says.
  - Docs: `terminal-interface#composer`, `providers-models#images`.
  - Acceptance: an image copied to the clipboard is pasted with Ctrl+V,
    shown as `[Image #1]`, and reaches the model (Linux, by hand; other
    platforms as §2.11).

### Rendering

- **M1 — the Markdown renderer**: tables, key/value fallback, fence unwrap,
  literal tables, task lists, list spacing, web and local link display,
  `tui.rendering.*`.
  - Files: `tui/src/markdown_render/`, `tui/src/markdown.rs`,
    `tui/src/table_detect.rs`, `config/src/tui.rs`.
  - Tests: Codex's renderer tests and snapshots, ported and adapted to
    pulldown 0.13.
  - Docs: `terminal-interface` (a new "Rendering" heading, `#rendering`),
    `config-reference`.
  - Acceptance: a table, a task list and a link from a model's answer
    render as in Codex; a table too wide falls back to key/value rows.
- **M2 — source-backed messages and resize**.
  - Answer cells keep their Markdown source and re-render per width.
  - Consolidation at the stream's end.
  - Reflow on resize in Fullscreen, and in Scrollback by clearing and
    rewriting under Codex's row cap (D19).
  - Files: `tui/src/history_cell/{mod,messages}.rs`,
    `tui/src/app/{resize_reflow,message_consolidation}.rs`,
    `tui/src/insert_history.rs`, `tui/src/transcript_view/`.
  - Tests: resize snapshots in both presentations (the `vt100` test
    terminal); the row cap.
  - Acceptance: after a resize every answer is wrapped to the new width.
- **M3 — streaming**: incremental block rendering, stable/tail regions,
  chunking and commit ticks, table holdback, open-fence highlighting,
  prose previews.
  - Files: `tui/src/markdown_stream.rs`, `tui/src/streaming/`,
    `tui/src/render/highlight_streaming.rs`.
  - Tests: Codex's streaming tests; a frame-by-frame test that committed
    rows never change.
  - Acceptance: a long answer with a table and a code block streams
    without re-rendering committed rows, and catches up when it falls
    behind.
- **M4 — hyperlinks and wrapping**: OSC 8 carried beside the text and
  emitted at the buffer and scrollback writers; URL-aware wrapping.
  - Files: `tui/src/terminal_hyperlinks/`, `tui/src/wrapping.rs`,
    `tui/src/insert_history.rs`, `tui/src/custom_terminal.rs`.
  - Tests: a URL never split; OSC 8 bytes never counted in width; a
    terminal without OSC 8 support shows the text.
  - Docs: `terminal-interface#terminal` (which terminals show links).
  - Acceptance: a link is clickable in a terminal with OSC 8, in both
    presentations.
- **M5 — math**: Codex's masker and Unicode renderer.
  - Files: `tui/src/markdown_render/math/`, the streaming math preview.
  - Tests: Codex's math tests.
  - Acceptance: inline and display math render as Unicode; an unsupported
    expression stays verbatim.
- **M6 — mermaid**: `tui/src/mermaid/`.
  - Files: `tui/src/mermaid/`, `tui/src/markdown_render/mermaid.rs`.
  - Tests: Codex's families tests and snapshots.
  - Acceptance: a ` ```mermaid ` block of each supported family renders as
    a text diagram, else as code.
- **M7 — colour in command output** (§2.12).
  - Files:
    - `foundation/src/terminal_text.rs`;
    - `execution/src/{terminal_screen,service/mod}.rs`;
    - the progress event;
    - `contracts/src/tools/display.rs`;
    - the execution presenter;
    - `tui/src/history_cell/tool/{command,live_output}.rs`.
  - Tests:
    - SGR kept and every other control dropped, in pipe and PTY mode.
    - The model-visible output unchanged (goldens hold).
    - Replay shows colour; an old record shows plain.
    - The garbled-output tests still pass.
  - Acceptance: `ls --color=always` and `cargo build` show their colours,
    live and after a resume.
  - Hanafubuki: `AgentToolDisplay::Command.styled`, the progress field.
- **M8 — tool output formats**: JSON pretty-printing, MCP content summaries,
  the diff improvements.
  - Files: `tui/src/history_cell/tool/{text,patch}.rs`,
    `tui/src/history_cell/tool/mcp_result.rs`, `tui/src/diff_render/`, the
    patch presenter and `AgentPatchPreview` (`contracts/src/tools/display.rs`).
  - Tests: snapshots per format; a multi-megabyte MCP image never retained.
  - Docs: `terminal-interface#tools`.
  - Acceptance: a JSON output is indented, an MCP image is one summary
    line, and an updated file's diff has line numbers.

## 4. Decisions

"Given" decisions came with the request. "To confirm" decisions are this
plan's; the recommendation is what the plan assumes until you answer.

| # | Status | Question | Recommendation |
| --- | --- | --- | --- |
| **D1** | Given | Milestone order and shipping | Strictly sequential, one commit each on main with green CI (§3). |
| **D2** | Given | Where the structured-output capability is answered | Per route, as a learned fact with the 7-day TTL; native → tool → text; narrow refusal detection; no tool-name change. |
| **D3** | To confirm | Which rung the fact records | **The best rung that worked**, not only "native refused", so a gateway that refuses both pays its failed requests once a week, not on every title. |
| **D4** | To confirm | The reviewer | **Native → text only, strict reading kept.** It has no tool rung today, and approval stays strict. |
| **D5** | Given | Titles | Retry on later user turns while the title is the provisional one; normalise like Codex instead of refusing. |
| **D6** | To confirm | How "provisional" is known | **Compare with `provisional_agent_title` of the first user message**, with no new `AgentSessionTitleSource` value: no format or host change. A generated title equal to the provisional one is remembered for the run, so it is retried at most once more after a restart. The alternative is a `Provisional` source, explicit but a contract change for Hanafubuki. |
| **D7** | Given | Context window | Refresh bound sessions at a listing, keep usage in sync, stop hiding a resolved window, add Hoshimi's fields. |
| **D8** | To confirm | Hoshimi windows that need another request (Ollama `/api/show`, LM Studio `/api/v0/models`) | **Not in this plan**: each is an extra request per model in discovery. A later plan if you use those servers. |
| **D9** | Given | The thinking fix | In the kernel, no TUI guard. |
| **D10** | To confirm | Goals' token budget after C1 | **Count the prompt total**, what the model read, for every protocol. Anthropic budgets then fill faster than before. The alternative, counting uncached input only, would be a second definition. |
| **D11** | Given | Cache accounting | Prompt total = uncached + read + write; context consumed = prompt total; old usage decodes with defaults. |
| **D12** | Given | `/context` | As §2.5. |
| **D13** | Given | `/effort` | Options from reported or configured levels only; Default and Custom always; refused mid-turn; a new `SettingKind`. One change from the request's "reported plus configured": configured levels **replace** the reported ones, as the metadata's precedence already does (§1); a union would need a second accessor. |
| **D14** | To confirm | How `!command` runs | **A no-model turn that runs the planned `exec_command` handler, approved by the person who typed it, in the session's sandbox, recorded as a `UserShellCommand` item the model reads.** Codex runs it outside the sandbox without asking. Ours keeps the sandbox, since a typed command can still surprise (an alias, a script). The cost: with no model to ask for escalation, a command the sandbox blocks (`!npm install` offline, a push outside the writable roots) just fails, with a hint. Say if you want Codex's unsandboxed behaviour. |
| **D15** | To confirm | `!command` during a running turn | **Refused**, not Codex's auxiliary mode: running beside a turn needs its own interleaving of history items. |
| **D16** | To confirm | Ctrl+R's sources | **This run's messages, then the user messages in the history store** (a bounded facade read), not a new history file. |
| **D17** | To confirm | The external editor's process | **A foreground child the TUI waits on, as `/diff`'s `git`**, outside `ExecutorManager`: the person's own program, holding the terminal, never left running. |
| **D18** | To confirm | `?` and the agent keys as rebindable actions | **An `empty_composer_only` flag** that lets an action take a text key, and `SHARED_KEYS` pairs with word motion. Without it `?` cannot have a default. |
| **D19** | To confirm | Scrollback on resize | **Port Codex's reflow**: clear the terminal's scrollback and visible screen and write the transcript again from source cells, under its row cap. The premise that Scrollback cannot re-render was wrong; the cost is that the terminal's scrollback before the TUI started is cleared too, as in Codex. Alternative: re-render only the tail. |
| **D20** | To confirm | Math with pulldown 0.13 | **Keep Codex's masker, `ENABLE_MATH` off**, so Codex's tests port unchanged. |
| **D21** | To confirm | Colour on replay | **Persist the styled copy as display metadata of the tool output**, bounded and tolerant. The alternative, live-only colour, shows plain output after a resume. |
| **D22** | To confirm | Mermaid's home | **A module of the TUI** (`tui/src/mermaid/`), not a published crate: only the TUI uses it, and a new crate needs a description, publishing and a layer entry. |
| **D23** | Given | Images | Paths through `prepare_and_stage_images` (no bytes variant needed, §1); capability gate; placeholders; steers; backtrack; pasted paths; WSL fallback; Windows and macOS covered by unit tests of the non-clipboard logic only. |
| **D24** | Given | Formats | Every Codex format listed as ported or not, with its reason (§2.12). |

## 5. Not in this plan

- F8 voice, pets, sparkles.
- Inline image display through terminal graphics protocols.
- Codex's file citations and follow-up directives.
- Window discovery through extra requests (Ollama `/api/show`, LM Studio
  `/api/v0/models`) (D8).
- Running `!command` beside a turn (D15).
- A persistent prompt history file (D16).
- Codex's vim composer modes.
