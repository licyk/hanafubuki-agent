# Plan: background compaction from Codex, hierarchical compression from billion-context, and a choice between them

Repositories:

- **The Agent:** `/root/code_workspace/hanafubuki-agent`, main at `2907aee`.
- **Codex:** `/root/code_workspace/codex`, checked out at `57a808a392`.
- **billion-context:** `/root/code_workspace/billion-context`, at `4be21b08e`, with its kernel `acp-kernel` 0.0.189.

Hanafubuki embeds the Agent through the facade, so this work affects it through:

- the facade;
- the settings;
- the history format;
- event and snapshot types;
- the goldens.

Each phase lists its notes for Hanafubuki.

The request has six parts:

1. Port Codex's `async_compaction` in full, including its asynchronous execution.
2. Port the context-compression algorithm of billion-context in full.
3. Integrate both into the Agent's context management.
4. Choose and switch the compaction algorithm through the application settings.
5. Handle compatibility, concurrency, errors and resources.
6. Make the architectural changes that let both coexist without interfering.

Each part maps to sections of this plan:

| Part | Sections |
| --- | --- |
| 1. Codex's background compaction | §1.1, §3 |
| 2. billion-context's algorithm | §1.2, §4 |
| 3. Integration into context management | §1.3, §2, §3.4, §4.3 |
| 4. Selection in the settings | §2.2, §2.3 |
| 5. Compatibility, concurrency, errors, resources | §5 |
| 6. Coexistence and architectural changes | §2.1, §2.4, §5.5 |

Phases are in §7 and decisions in §8. What is not ported is in §9.

## 1. What exists today (verified)

### 1.1 Codex's `async_compaction`

**What it is.** Async compaction is speculative remote compaction. It is not a separate compactor: it is a background input mode, transport and install mode of Codex's remote compactor v2 (`codex-rs/core/src/compact_remote_v2*.rs`). The code is in:

- `core/src/session/async_compaction.rs`;
- the turn loop in `core/src/session/turn.rs`;
- the prefix branch of `Session::replace_compacted_history` in `core/src/session/mod.rs`.

**Switches.** There are two feature flags, both under development and both off by default:

- `async_compaction`, which turns the feature on;
- `async_compaction_root_only`, which keeps child agents out.

The threshold is set in `[features.async_compaction] model_async_compact_threshold_percent`. It defaults to 65 and must be between 1 and 100, even when the feature is off.

**When it is allowed.** Async compaction is eligible only when the provider has `RemoteCompactionSupport::V2`, the `TokenBudget` feature is off, and the session is a root session when `async_compaction_root_only` is on. Codex has **no background variant of its local summarizing compaction**.

**Launch.** Async compaction launches at two points:

- before a turn's first request, once the routing token exists;
- after each sampling request.

It launches when the active tokens reach 65% of the raw window while still under the foreground limit. A launch is skipped in these cases:

- the turn is cancelled;
- the session is shutting down;
- the history is empty;
- the credentials changed;
- the same `(history_version, model, compaction hash, auth owner)` was already tried. A failed attempt is not retried for the same history generation.

**Snapshot.** A launch takes the **whole history** plus `(history_version, len)`. `history_version` changes only when the history is replaced (compaction, rollback, audio rejection), never when items are appended. So "same version and `len` ≤ the current length" proves that the snapshot is still a prefix of the live history.

**Request.** The task sends one Responses compaction-v2 request: the snapshot plus a `compaction_trigger` item, answered by exactly one `compaction` item with opaque `encrypted_content`. It goes over HTTP rather than the turn's websocket, emits no interface events and makes no stream retries. The usage is charged to the launching turn's accounting, never to the thread's displayed usage.

**Result slot.** The result goes into a single `oneshot` slot, which is read without blocking at these checkpoints:

- the next turn's pre-sampling;
- mid-turn, when a follow-up request is needed;
- after the turn, only when `model_post_turn_compact_threshold_percent > 0` (it defaults to 0), auto summarization is on, no input is pending, and the turn was not cancelled.

**Install.** The result is installed on the turn task, under the history lock, as:

```
prefix_context ++ [retained user messages…, compaction] ++ initial_context ++ live_suffix
```

`initial_context` is rendered against the world state captured at launch. It goes after the compaction only when the live suffix holds a real message or a kept tool output; otherwise it goes before the last real user message (`session/mod.rs:4797-4808`). A result made for another model or compaction hash counts as rejected.

**Fallback.** When a foreground compaction is *required* while a compatible background request is still running, the foreground **waits for it** instead of sending its own (`wait_for_pending`). The fallback matrix:

| Case | Result |
| --- | --- |
| Result valid and installed, context now under the limit | Done. |
| Result valid and installed, context still over the limit | One foreground pass as well. |
| Result rejected, compaction optional | Skip; the history is unchanged. |
| Result rejected, compaction required | Foreground compaction. |
| Background failed or incompatible | Foreground compaction. |

**Interrupts and shutdown.** Interrupting a turn blocks only new launches; the running task belongs to the session. Shutdown cancels the task and then waits for it, so completed usage is still charged.

**Persistence.** Nothing about background compaction is persisted. An adopted result writes the same rollout records as a synchronous compaction.

**Size.** About 450–550 production lines are background-only. They sit on about 1.6k lines of v2 compactor, much of it specific to Codex: Guardian, audio rejection, incremental tools, analytics. Codex's tests for it are about 1.3k lines.

The tests that define the contract:

- `session/async_compaction_tests.rs`;
- `compact_remote_v2_installation_tests.rs`;
- `core/tests/suite/scenarios.rs:3583-4334`, with eleven `async_compaction_reuses_pending_work` cases and four live-suffix cases;
- `scenarios_guardian_reconsideration.rs`.

### 1.2 billion-context

**Two parts.** billion-context is a pure kernel, `acp-kernel` (`kernel/src`, about 25.9k lines of TypeScript, about 11.5k of them the core algorithm), and a large proxy host (`src/`, about 161k lines). A native agent needs the kernel plus a thin host loop.

**The algorithm: model-driven incremental hierarchical compression.**

- **Who writes the summaries.** The **agent's own model** writes them during its normal turn by calling a `compress` tool. Its single string argument holds one or more ranges, each a header line `mNNNNN–mNNNNN topic` followed by its summary. No separate summarizing request is made.
- **Blocks and tiers.** The kernel records a block per range. Blocks form tiers: tier 1 summarizes raw messages, tier 2 distils tier-1 blocks (ranges named by block refs, `bN–bM`), and tier 3 condenses tier-2 blocks.
- **Per request.** `processTurn` runs a pipeline of 18 steps (`compress.ts:1088-1108`; the 9-step order in `kernel/DESIGN.md` is stale) over the **full, unpruned** message log and the plain-data state. It does the following:
  - mints refs that are never reused;
  - keeps blocks in sync with what the log still contains;
  - prunes covered messages;
  - keeps tool-call and reasoning pairs whole;
  - hides consumed `compress` calls;
  - computes the compressible ranges;
  - decides a growth-gated *nudge*, in a pressure band at 75% of the window, with tier escalation;
  - clips tool outputs as a last resort at 95% (an emergency truncation, latched until the next fold);
  - renders `<acp tokens="…" type="…">mNNNNN</acp>` tags with frozen token counts.
- **Per `compress` call.** `applyCompression` applies one call's ranges all-or-nothing. Its refusals are precise texts written for the model to read.
- **Prefix cache.** The README claims a 95–97% prefix-cache hit rate for a healthy session. The paper measures 88.3–90.7% blended across two hosts, and 94.2% when calls are under two minutes apart. The main mechanism is *compress-as-anchor*: the summary rides in the arguments of the `compress` call already in history, so a fold only deletes a span. Bytes elsewhere change only at a fold, which is what the frozen token counts, the latches and the fold-gated orphan sweep ensure.
- **Other tools.** The core set (`ACP_TOOL_NAMES`, `compress-tools.ts:585-591`) also has `decompress` (cache-safe expansion), `search_context` (BM25 and fuzzy search over summaries and folded originals), `acp_status` and `acp_cache` (the cache report). `absorb` and `acp_retrieve` are opt-in, and `acp_rule` exists only when rules are enabled.

**What the host provides** (from the proxy's request preparation, `src/server/prepare-*.ts`, and its native adapters; billion-context has no single document of this contract):

- the full log with stable ids;
- the provider-reported token count;
- the tools;
- a byte-stable system prompt;
- the nudge as an ephemeral trailing user message;
- persistence of the state;
- the `compress` call and result kept verbatim in history;
- the host's own automatic compaction **turned off** while the kernel owns the context.

**Licence.** MIT plus an "Additional Term — Attribution on Product Surfaces". A product end users see must credit billion-context (and acp-kernel) with links, on its home page, in its documentation or on an About page. Shipped documentation is enough for embedded uses (`LICENSE:42-43`). The kernel descends from an AGPL plugin (DCP). Its `PROVENANCE.md` says files with no DCP counterpart (the author's own work) were carried over verbatim, and DCP-derived files were reimplemented in fresh expression. No file in the repository declares AGPL. See decision 7.

### 1.3 The Agent

**Implementations.** There are three compaction implementations, all in `crates/kernel/src/service/compaction/`, chosen by `AgentCompactionImplementation` (`model/turn.rs:135`):

- `LocalSummary`, Codex's summarizing prompt;
- `ResponsesCompact`;
- `ResponsesCompactV2`.

The choice comes from `providers.toml` `[modelPolicy] remoteCompaction`. It is frozen into `AgentSessionConfig.compaction_capability` at session creation and at `/model`.

**Triggers.** There are five, all awaited **in line**:

1. pre-turn at the threshold (`autoCompactThresholdPercent`, 80);
2. mid-turn at the threshold;
3. the model's `new_context` tool;
4. manual `/compact`;
5. one compaction and retry after a provider context-window error.

Automatic compaction runs on the turn's task (`turn/loop_driver.rs:16-59`). No compaction has a task of its own.

**One operation per session.** `has_active_operation` (`service/state/validation.rs:47`) counts a running compaction. While one runs, these are refused: turns, steering, `/model`, Skills, context changes, reviews, `/side` and `set_working_directory`.

**Install.** The install is a compare-and-swap (`commit.rs:16-30`): any commit to the session during the compaction fails it. The result is a `Compacted { checkpoint }` boundary whose `replacement_history` becomes the window's model context. The transcript keeps every raw item.

**Validation.** `validate_turn_boundary_refs` requires the following:

- the *compacted prefix* holds only messages and summaries, with at least one summary and no tool call;
- after the first `TurnStart`, the history has full turn structure.

**Request building.** `step_model_request` (`sampling/mod.rs:100`) builds every request: the step's own, the threshold estimate and the post-install check. Context sections are rendered where they last changed, in slot order, so the prefix stays stable. Historical tool outputs are capped by `project_model_tool_outputs`.

**Settings.** `config.toml` (`AgentSettings`) is layered, has `config get/set` and Hanafubuki's settings form, and is read per call. `providers.toml` `[modelPolicy]` holds the compaction keys today, frozen per session.

**Bug: changing `remoteCompaction`.** Changing it breaks the next compaction of every existing Responses session. `activate_model_session` installs a transport with the new value, but the session config keeps the old one. `compaction_request` then fails "compaction transport capability changed after session creation" (`prepare.rs:359-363`) until `/model` is selected again.

**History format.** The `AgentSessionConfig` fields have no `#[serde(default)]`. A new field without one makes every existing session, and the history corpus shared byte-for-byte with Hanafubuki, fail to open. The corpus contains no compaction records, so new enum variants and new items are safe. The format version must not be raised: raising it quarantines every session.

**Module size.** The compaction modules are close to the limits. `commit_compaction` is about 190 of the 200 lines allowed per function, and the five files total about 1,675 lines.

**Events.** Only a compaction's final status reaches the event sink (`compaction_updated`). The terminal interface shows "Context compacted" only for a manual `/compact` and when replaying a transcript. The architecture page lists "background compaction" under *Not built*.

## 2. Architecture

### 2.1 One owner of context reduction per session

A session has one **compaction strategy**. It decides what *starts* a reduction of the context:

| Strategy | Starts reductions | Reduces by |
| --- | --- | --- |
| `standard` (default, today's behavior) | the threshold, `new_context`, a provider context error, `/compact` | a compaction (`LocalSummary`, `ResponsesCompact` or `ResponsesCompactV2`), in line |
| `background` (Codex's async compaction) | a lower threshold starts a compaction in the background; the `standard` triggers remain for when a reduction is required | the same compaction implementation, run on a snapshot and adopted at checkpoints |
| `hierarchical` (billion-context) | nudges ask the model; the model calls `compress` | a request-time projection of the history over model-written blocks |

All three keep the **standard compaction as the floor**: `/compact`, `new_context` and the provider context-error retry always run a standard compaction, whatever the strategy. So every strategy has the same last resort, and the two new ones never need a recovery path of their own.

The strategies cannot interfere with each other because they own different things:

- **`background`** owns a session slot that is not persisted (§3.2) and installs through the compaction commit, now able to rebase (§3.4).
- **`hierarchical`** owns a persisted state field and the request projection (§4.3). It never replaces history.
- Only one strategy is active per session, and §2.3 defines every switch.

### 2.2 The setting

`config.toml` gets a `[compaction]` table (decision 1):

```toml
[compaction]
strategy = "standard"            # standard | background | hierarchical

[compaction.background]
thresholdPercent = 65            # 1..=100; launches only below the foreground threshold
childSessions = true             # Codex's async_compaction_root_only, inverted

[compaction.hierarchical]
maxContextLimitPercent = 75      # pressure band (nudge.maxContextLimitPct)
emergencyThresholdPercent = 95   # emergency nudge and truncation
nudgeGrowthTokens = 50000
preserveRecentMessages = 3
preserveRecentTokens = 5000
minCompressRangeChars = 5000
minSummaryLength = 50
maxSummaryLength = 20000
tiers = true
protectedTools = []
messageTimestamps = true
# opt-in features (§4.6), off by default: absorb, contentStore, crush, rules
```

The keys follow billion-context's host `compress.*` keys (its `docs/configuration.md`), with the kernel's defaults where they differ, in camelCase like the rest of the file.

Decoding is per field: a bad value falls back to its default with a warning, as in the rest of `AgentSettings`. The `[modelPolicy]` keys stay where they are:

- `autoCompactionEnabled = false` turns off the automatic triggers of `standard` and `background`, including background launches. It does not stop `hierarchical` nudges, which ask the model and change nothing by themselves.
- `autoCompactThresholdPercent` is the foreground threshold of `standard` and `background`. `hierarchical` does not use it.
- `remoteCompaction` chooses the implementation that `standard` and `background` run.

A `compaction.background.thresholdPercent` at or above the foreground threshold never launches, and a warning says so.

The setting reaches every place settings are edited:

- `config get/set` (from `AgentSettings::default()`);
- `AgentSettingsPatch` and `agent_settings_fields()` for Hanafubuki's form, with `settings.fields.*` keys;
- a row in the terminal interface's `/config`;
- the configuration reference pages.

### 2.3 Applying it and switching

**Where the strategy is stored.** It is a per-session value, `AgentSessionConfig.compaction_strategy: AgentCompactionStrategy`, with `#[serde(default)]`, persisted in every runtime checkpoint. The transport does not report it, so it cannot have the `remoteCompaction` bug.

**When a change applies.** The facade reads `[compaction]` per turn, as it reads the permission mode. It applies a changed strategy to the session at turn admission, when the session is idle, through a kernel call `set_session_compaction_strategy`, which is persisted. A `/compact` reads it the same way.

**New sessions.** A new session takes the setting. A child session takes it unless it is `background` with `childSessions = false`, in which case the child runs `standard`. Ephemeral sessions (reviews, `/side`, recaps) always run `standard`.

**Every transition:**

| From → to | What happens |
| --- | --- |
| `standard` → `background` | Eligible at the next checkpoint. |
| `background` → `standard` or `hierarchical` | The running task is cancelled and its slot cleared. A finished result is discarded; its usage is still recorded (§3.6). |
| `standard`/`background` → `hierarchical` | The state starts empty, and refs are minted over the current window. A compacted prefix (`CompactionSummary`, `ProviderCompaction`, the world-state checkpoint and the retained messages) is protected. The first fold busts the prefix cache once. |
| `hierarchical` → `standard`/`background` | Active blocks **keep projecting** exactly as before, otherwise the next request would carry every folded message. Only the nudges, the static prompt and the hierarchical tools stop, so no new block is made. The `compress` calls stay in the request as summary carriers, so the switch costs no cache miss beyond the changed instructions and tools. The blocks end at the next standard compaction: once the covered items leave the window, the blocks deactivate. |

A switch never rewrites the stored history.

**A carrier call to a tool the request no longer declares is accepted.** No encoder filters history tool calls by the declared tools (Anthropic `encode.rs:83-104`, Chat Completions `:86-115`, Responses `:313-343`). Two cases already send such calls today:

- a deferred tool no longer selected after a compaction (`sampling/request.rs:352-374`);
- `LocalSummary`, which sends history `ToolCall`s with `tools: []`.

If the model calls `compress` after the switch, it gets the ordinary unknown-tool error and the turn goes on. The empty-`tools` case already exists for `LocalSummary` on Anthropic, independently of this work. Anthropic has rejected requests with `tool_use` blocks and no `tools` key; this is from memory, not checked here. P2 checks it against the Anthropic fake and records it in `todo.md` if it is real.

### 2.4 Where the code lives

| Piece | Home | Why |
| --- | --- | --- |
| Strategy enum, session field, switching | kernel `model/session.rs`, `service/session/configuration.rs` | It is per-session state, persisted with the runtime checkpoint. |
| Today's compaction, split along responsibilities | kernel `service/compaction/{admission, prepare, execute, commit}` plus a `source.rs` for the snapshot and replacement build | The files are at the size limits and the background path reuses them (P1). |
| Background compaction | kernel `service/compaction/background/` | It is session concurrency and needs the record lock, the cell signals and the managed tasks. |
| The hierarchical algorithm | new crate `crates/compression` (`hanafubuki-agent-compression`), pure: no I/O, no async, no kernel, model-api or contracts dependency (it uses foundation only if needed) | billion-context's kernel is pure, so the port stays pure and carries its tests. |
| Hierarchical integration | kernel `service/compression/`: adapter from `ConversationItem`, the request projection, the core tools, the nudge, the fallbacks | It needs the session record and the request builder. |
| Settings | config crate, a new `settings/compaction.rs` beside `settings/{mod, decode, schema}.rs` | The table's types, defaults and per-field decoding. The web keys sit in `settings/mod.rs` today; the compaction table is larger. |
| Interface | `crates/tui/src/commands/compression.rs` (`/compression`), the status line, the compaction cells | |

**Fault isolation (decision 5).** The kernel calls the algorithm crate under `catch_panic`. A panic in the projection makes that request use the full view and the session fall back to `standard` for the turn, with a warning event and a kernel log record; the next turn tries again. A panic in `compress` becomes a tool error. The crate follows the plugin rule on panics. `cargo xtask check-plugin-panics` scans `crates/plugin-*` plus the hard-coded `crates/agent/src/plugins` (`xtask/src/plugin_panics.rs:13-34`). `crates/compression` is added to that list, and the plugin-rules page says so.

**The rejected alternative** is a kardia plugin with a new contracts registry (`ContextProjectionRegistry`). The kernel would depend on that plugin for *every* request, against composition rule 1. And the contracts crate cannot name model-api types (report §5.2). A built-in strategy is the smallest coherent design. Panic containment at the call boundary gives the same "one part fails, the rest runs" property.

The new crate needs these registrations:

- a row in the plugin-rules page's "Crates and dependencies" table (both languages);
- the layer rule in `cargo xtask check-dependencies`;
- the workspace dependency;
- a `description`.

## 3. Codex's background compaction (`background`)

### 3.1 What is kept and what is adapted

The port keeps Codex's design:

- a task the **session** owns;
- a snapshot plus a prefix proof;
- a single result slot, read without blocking at checkpoints;
- `wait_for_pending` when a compaction is required;
- the fallback matrix (§1.1);
- one foreground pass when the adopted result still leaves the context over the threshold;
- validation under the lock at install;
- events only at install;
- usage charged even for a discarded result;
- shutdown waiting for the task;
- interrupts blocking only new launches;
- the root-only switch;
- no launch on budget pressure, Codex's `TokenBudget` exclusion.

The adaptations, each forced by this repository (decision 3):

1. **Any implementation, any provider.** Codex runs only remote v2. Here the task runs the session's own implementation (`LocalSummary`, `ResponsesCompact` or `ResponsesCompactV2`) on a detached `CompactionPlan`. The Agent's compaction is already planned from a snapshot under the lock (`begin_compaction` builds `CompactionPlan` with the source items, ids, images and request). P1 makes building the replacement a pure function of that plan, so a background run never reads the live record. This works with every provider, which Codex's version does not.
2. **The snapshot ends at the latest `TurnStart`.** The validation (§1.3) forbids tool pairs in the compacted prefix and requires full turn structure after it. So a live suffix that begins mid-turn cannot be installed. A launch compacts only the **completed turns**: everything before the current turn's `TurnStart`, the boundary that `PreTurn` already uses (`compaction_source`, `prepare.rs:298`). The suffix then always starts at a `TurnStart`. Codex also compacts the turn in progress; this port cannot. One long turn therefore falls back to the in-line mid-turn compaction, which compacts everything.
3. **A slot of its own, not `runtime.compaction`.** If the background compaction counted as an active operation, it would refuse turns, steering and `/model` for its whole duration. It lives in `SessionRuntime.background_compaction`, which is not persisted, as in Codex. A restart loses it harmlessly and leaves no recovery obligation.
4. **The prefix proof is the window and the item ids.** The Agent has no in-session rollback (editing forks), and a compaction install opens a new window. So "the same `context_window` id, and the snapshot's history item ids are a prefix of the live history" proves what Codex's `history_version` proves. The Agent's own `history_version` (`service/mod.rs:126-140`) changes on every append, so it cannot serve as Codex's counter, and no new counter is added.
5. **Its usage goes to the session, not to a turn budget.** Codex charges its accounting under the launching turn. Here the usage goes to the session's usage. It is stored in the compaction snapshot when the result is adopted, or recorded with the next commit when it is discarded. It does not count toward a turn's `turnBudget`, so a background run never makes a turn fail on its budget.

### 3.2 State

```rust
/// Not persisted: a restart drops it.
pub(crate) struct BackgroundCompaction {
    attempt: BackgroundAttempt,          // window id, source item ids, model, implementation
    cancellation: CancellationToken,     // a child of the service's stop token
    outcome: BackgroundOutcome,          // Running | Ready(CompactionSample) | Failed
    abort: Option<AbortHandle>,
}
```

- **Where it lives.** It is a field of `SessionRuntime`, under the session record's `parking_lot` lock.
- **Its task.** It is spawned like a managed turn (`spawn_managed_turn`, `service/turn/admission.rs:129`) and registered so that shutdown waits for it: either in `managed_turns` under a session-scoped identity, or in a sibling `managed_background` map with its own condvar (decided in P3, by whichever keeps `stop_operations` simplest).
- **Deduplication.** An attempt key `(window id, source item ids, model, implementation)` prevents a relaunch with the same inputs. As in Codex, a failed attempt is not retried until the window, the model or the implementation changes.

### 3.3 Launch and adoption

**Launch.** It happens in `run_loop` (`turn/loop_driver.rs`) at two places:

- after `prepare_sampling_step` on the pre-turn step;
- after every sampling request.

A launch needs all of these:

- the strategy is `background`, and automatic compaction is enabled;
- the step's estimate (`active = max(estimate, context_consumed_tokens)`) is at or over `background.thresholdPercent` and under the foreground threshold;
- the turn is not cancelled, and the service is accepting;
- there is at least one completed turn before the current `TurnStart`;
- the attempt key is new;
- the session is not a child with `childSessions = false`.

The plan is built under the lock, as `begin_compaction` builds it today, but nothing is reserved in `runtime.compaction` and nothing is persisted. The task runs `execute_compaction_inner` on the plan and stores the sample in the slot. When it finishes, it notifies the session cell so a waiting `wait_for_pending` wakes.

**Adoption.** Before `maybe_auto_compact`, and before `/compact`, a non-blocking `adopt_background_compaction` runs:

- **Ready, prefix proof holds, model and implementation unchanged:** install it (§3.4). Then estimate the next request. If it is still at or over the threshold, run one in-line compaction too.
- **Ready but stale, or failed:** discard it and clear the slot. The attempt key stays, so the same inputs don't relaunch; a new window or model makes a new key.
- **Still running, and a compaction is required now** (the threshold, a provider error, `/compact`): `wait_for_pending` waits on the cell's terminal signal, cancellably, with the turn's token. On a usable result it installs it; otherwise it runs the in-line compaction. **Still running, and no compaction is required:** carry on.

`new_context` always runs in line, because it asks for a new window *now*.

### 3.4 The rebasing install

`commit_compaction` gains a second mode, `InstallMode::Rebase`. The CAS mode remains for in-line compactions. A rebase install checks:

- that `context_window` still has the snapshot's window id;
- that the live history begins with the snapshot's item ids. `CompactionSource.item_ids` is the **whole** history at launch, not only the compacted part (`prepare.rs:327-333`);
- that the live suffix (the snapshot's current turn plus everything appended since) passes `validate_turn_boundary_refs`. It does by construction, because it starts at a `TurnStart`; this is only a guard.

It replaces `operation_revision == status.revision` with these checks, and drops the test that the item ids are equal. The installed history is:

```
[world-state checkpoint] + replacement + live suffix
```

- **The world-state checkpoint** is rebuilt from the **live** history (`full_world_state_checkpoint`), as Codex rebuilds its baseline from the current checkpoint. So sections that changed after the snapshot are not lost.
- **The replacement** is the retained user messages and the summary or payload.
- **`CanonicalInitialContext`** is not inserted: the suffix starts at a `TurnStart`, which is what the pre-turn install already does.

**The live suffix is stored the way today's pre-turn install stores the current turn.** It goes into the checkpoint's `replacement_history` with its original ids (`commit.rs:63-64`, `:98`), and the cutoff `future_items_after_item_id` stays the display `CompactionSummary` item's id, the newest (`commit.rs:69-70`, `:104`).

- `validate_context_window_checkpoint` (`service/validation/history.rs:74-77`) requires every replacement id to be at or below that cutoff. A cutoff at the snapshot's last item would fail it, because the replacement items get fresh, higher ids (`compaction/mod.rs:230-243`).
- So `load_latest_model_context` replays a rebased install exactly as it replays a pre-turn compaction, and no rule changes.
- Item ids are allocated in increasing order, but they are not in history order after a compaction: the carried suffix keeps lower ids than the replacement before it. No code may assume "a higher id is later in the history".

The rest is unchanged: the replacement's ids, the `Compacted { checkpoint }` boundary, and the display `CompactionSummary` item. Resume and fork see a rebased install as an ordinary compaction. P3 checks this with `compaction_contract/recovery_fork.rs` and `persistence_contract/compaction_recovery.rs`.

### 3.5 Interaction with the rest of the session

| Event | Effect on a running background compaction |
| --- | --- |
| A turn is admitted or steered, a Skill or context change, `/model`, a review, `/side` | Allowed: the slot is not an active operation. `/model` makes the result incompatible, so it is discarded at adoption. |
| The turn is interrupted (`cancel_turn`) | No effect, as in Codex. Only new launches from that turn are blocked. |
| An in-line compaction installs (`new_context`, a provider error) | The window id changes, so the result goes stale and is discarded. |
| The strategy changes away from `background` | Cancelled, and the slot is cleared. |
| Archive, delete, `close_ephemeral_session` | Cancelled and awaited before the session is disposed. |
| `stop_operations` (shutdown, restart) | Cancelled. Shutdown waits for it within the operations deadline (`embedding.en.mdx`, 5 s), so finished usage is recorded. |

### 3.6 Events, usage, interface

**Events.** A new phase, `AgentCompactionPhase::Background`, with trigger `Automatic`. The install commit records the snapshot as `Completed` with that phase and journals and emits `CompactionUpdated`. The background run has no `Reserved` or `Running` record, because nothing was persisted. A discarded or failed result emits nothing, as in Codex, and is logged at debug.

**Usage.** It is recorded as described in §3.1(5). For a discarded result it is added to the session's usage with the next commit, so `/status` stays exact.

**Interface.** The terminal interface gets a "Context compacted" cell for **every** compaction install, automatic and background included. Today only a manual `/compact` gets a live cell (§1.3). `/status` shows "compacting in the background" while the slot runs. `exec --json` emits `compaction_updated` with `phase: "background"`, which counts as an *appear* under the event stream's stability rule.

### 3.7 Tests

The tests port Codex's contract onto the kernel's fakes (`crates/kernel/tests/compaction_contract/background.rs`):

- **Launch gating:** a cancelled turn, under the threshold, at or over the foreground threshold, no completed turn, a child with `childSessions = false`, and the deduplication of a failed attempt.
- **Adoption:** pre-turn, mid-turn, and adopting after work was added since the snapshot. The live-suffix case checks the item order and the world-state checkpoint.
- **`wait_for_pending`:** success, failure, stale (model changed), interrupted while waiting (no in-line request after the interrupt), and still over the threshold (two installs).
- **Manual `/compact`:** reuses a pending run. Its fallback has trigger `Manual`.
- **Validation at install:** a window that changed between the poll and the install is refused.
- **Usage:** a discarded result is recorded once and never counted toward a turn budget.
- **Shutdown and lifecycle:** shutdown waits for a running task, and a restart leaves no recovery obligation.
- **Implementations:** each runs in the background (`LocalSummary`, v1, v2) against the HTTP fakes.

## 4. billion-context's algorithm (`hierarchical`)

### 4.1 Projection, not rewrite

This is the decision the whole integration hinges on. The session's history stays the **full log** of the current window, exactly as stored today, which is the first thing billion-context needs from its host. The request builder applies a pure

```
project(history_items, state, config, token_count) -> Projection { view, nudge, status_line, emergency }
```

and sends `view`. Stored items never change, so:

- `validate_persisted_history`, the history format, the corpus, replay, resume and fork are untouched;
- the standard compaction still works on the real items.

The state is the only new persisted data.

### 4.2 The algorithm crate (`crates/compression`)

The crate ports billion-context's kernel module by module. Line counts are TypeScript with comments; expect roughly half in Rust.

| Rust module | Ports | Contents |
| --- | --- | --- |
| `model` | `types.ts`, `state.ts`, `config.ts` | `Message` (id, role, kind, text, tool name and call id, image handles), `Block`, `State` (every field `#[serde(default)]`, not the lossy `mergeCompressionState`), `Config` with the kernel's defaults |
| `refs` | `refs.ts` | ref minting with protection (`BLOCKED`, deferred tags); never reused |
| `boundaries` | `boundaries.ts`, `block-map.ts` | ref and block-ref resolution, nested blocks, reversed pairs, orphan snapping |
| `parse` | `parse-compress-input.ts` | the lenient `compress` argument parser: the line form, JSON salvage, truncated streams |
| `project` | `unit-projection.ts`, `tool-pairs.ts`, `reasoning-pairs.ts`, `turn-integrity.ts`, `segment.ts`, `viable.ts` | the one definition of what a range folds, shared by apply and recommend |
| `protect` | `protected.ts` | protected tools, the recent zone, aged and failed `compress` exchanges |
| `apply` | `compress.ts` `applyCompression` + `applySingleRange`, `refold.ts` | all-or-nothing batches, tier resolution, refold in place, the refusal texts |
| `prune` | `prune.ts`, `sync.ts`, `hide-consumed.ts` | block activity, covered-message removal, summary anchors (pair-safe, first-user pin), carrier hiding and compaction, the fold-gated orphan sweep |
| `recommend` | `recommend.ts` | compressible ranges, merging, screening |
| `nudge` | `compress.ts` `decideNudge`, `nudge-text.ts` | the growth gate, the pressure band, tiers, the harvest stall, the estimate hold; the nudge and status texts |
| `truncate` | `truncate-tools.ts`, `truncate.ts`, the emergency-truncate step | clipping with a latch, the terminal-escape count |
| `latch` | `view-latch.ts` | fold-gated replay, needed by the opt-in transforms |
| `render` | `render-refs.ts`, `tokenize.ts` | `<acp>` tags with frozen token counts; the CJK-aware estimate |
| `turn` | `compress.ts` `processTurn`, `pipeline.ts` | the per-request pipeline, in the code's order (`compress.ts:1087-1110`), not the stale 9-step order in the docs |
| `decompress` | `decompress.ts` | `collect_block_content` (one tier or full), restored-inline marking, permanent expansion |
| `search` | `search.ts`, `search/` | BM25, fuzzy, substring and hybrid search over block summaries and folded originals (semantic search is not ported, §9) |
| `media` | `media-refs.ts` | a MEDIA REFS section so folded images can be reached again |
| `rebuild` | `rebuild.ts` | rebuilds the state by replaying `compress` calls (§4.5) |
| `report` | `report.ts` (`buildStatusReport`), `panel/panel.ts`, `cache-report.ts` | the `acp_status` text, the status panel, and the `acp_cache` cache report and fold economics |
| `prompts` | `compress-tools.ts` (function protocol), `compression-rules.ts`, `prompts.ts` | tool schemas, the static system prompt, the tier-2 and tier-3 rules, the language-preservation rule |

**Tests.** The crate's tests port the kernel's contract tests group by group (billion-context report §7): apply, parser and schema, prune, sync and refs, pairs and integrity, protection, recommend and nudge, cache stability, refold and decompress, prompts.

The byte-exact prompt fixture `kernel/tests/fixtures/prompt-function-default.txt` becomes a golden of this crate. Tests of the wire codecs, the text and hybrid protocols, packs and the proxy are not ported (§9).

### 4.3 Kernel integration

**Adapter.** It maps history items to the crate's `Message`s:

| History item | Messages |
| --- | --- |
| `UserMessage`, `SteeringMessage` | a user text (images as handles) |
| `AssistantMessage` | an assistant text |
| `ToolCall` | a tool call (`call_id`, name, argument text) |
| `ToolOutput` | a tool result |
| `CompactionSummary`, `ProviderCompaction` | a protected message |
| `TurnStart`, `WorldStateUpdate`, `ToolSelection`, `PermissionReview`, `SkillContext`, `CanonicalInitialContext`, `ContinuationContext` | not in the log; they keep their place in the request around the projected items |

`Reasoning` is not sent to the model here, so the reasoning-pair logic only keeps pairs whole when reasoning items are added later.

Message ids are `(source_session_id, item_id)`, unique within a session's lineage (forks keep them, §4.5). **Refs are not the item ids.** Item ids are not in history order after a compaction (§3.4), and ranges (`mA–mB`) need refs that increase along the history. Refs are minted by the crate in history order with billion-context's `assignRefs`, in the state's `messageRefs` map.

**Request.** `regular_model_request` builds its input from the projection's view:

- **Covered items** are left out.
- **A live `compress` call** stays, with its arguments compacted to the ranges still live (the stored item is unchanged).
- **A block without a visible carrier** is rendered at its anchor as a user-role text: "`[Compressed conversation section] — topic`" plus the summary.
- **Ref tags** are prepended to the text of user messages, assistant messages and tool outputs, **never to tool-call arguments**, which would corrupt the JSON (decision 6).
- **The static prompt** (`build_compress_system_prompt`) is appended to the instructions, byte-stable for a session.
- **The nudge or the status line** is appended as a trailing user input that is **never persisted**.

`project_model_tool_outputs` runs after the projection on what remains. The emergency truncation runs before it and is latched.

The threshold estimate in `maybe_auto_compact` uses the projected request, because every estimate goes through `step_model_request`.

**Tools.** The core set, `compress`, `decompress`, `search_context`, `acp_status` and `acp_cache`, becomes kernel core tools in the tool plan while the strategy is `hierarchical`. The precedent is `request_new_context` (`tool_execution/fixed.rs:86`).

- **`compress`** runs under the session record's lock: parse, apply to the full window, and stage the new state with the tool output in the same commit. A crash cannot separate the fold from its receipt. The receipt follows billion-context's contract: `[Compression FAILED`, `[Compression refused` or `[Compression no-op` for zero-applied calls (`protected.ts:34-36`), with the errors, warnings and notes verbatim. The success wording is the host's; the port uses billion-context's (`src/stream.ts:1612`).
- **`decompress`, `search_context`, `acp_status` and `acp_cache`** are read-only. They run on a snapshot of the state and the window. `acp_cache` reports from the usage the Agent already records per response (cache read and write tokens).

**Automatic compaction in this strategy.** Threshold compaction (pre-turn and mid-turn) is **off**, as billion-context's adapters turn off their host's own compaction. These remain:

- **A provider context-window error** runs one standard compaction and retries.
- **`/compact` and `new_context`** run a standard compaction.
- **Terminal escape** (decision 10): when nothing has been reclaimable for three requests and the context is over the emergency threshold, the kernel runs one standard compaction instead of telling the user to start over.

A standard compaction compacts the **projected** view: its source is the projection, with summaries as text. It installs as today; the covered items leave the window, and the blocks deactivate.

### 4.4 State and persistence

- **Where it is stored.** `PersistedAgentRuntimeState` gets `compression: Option<CompressionState>` with `#[serde(default)]`, written in every runtime checkpoint. Nothing else is stored; there is no separate file or debounce.
- **Reading it.** A state that cannot be decoded is dropped with a warning and rebuilt from the history (§4.5), never failing the session, per the rule on decoding our own data.
- **Size.** The state grows with the number of blocks. Blocks are never deleted, only deactivated, as in billion-context. P8 measures a long session's checkpoint size. If the state turns out to be a significant share of each record, it is written only when it changed (as a mutation), not with every checkpoint.

### 4.5 Forks, resume, children, model switches

- **Resume:** the state comes back from the runtime checkpoint.
- **Forks keep item ids (verified).** `ForkScan` clones the parent's items with their ids, swapping in the checkpoint's replacement items (`service/session/mod.rs:96-106`). `next_history_item_id` carries on from the parent (`fork.rs:65`, `:136`; `child.rs:188`, `:204`).
- **Fork at head** (`fork_session_at_head`): copies the boundary and history as they are, so it copies the state too.
- **Fork before a turn, and `edit_from_turn`:** the fork's history is rebuilt from the transcript (`load_fork_material`), with a `Full` boundary. The fork keeps the parent's blocks whose covered items are all present and drops the rest, the way billion-context's host adopts blocks on a fork (`src/fork-adoption.ts`). It keeps the parent's refs, so they stay the same for the model. When the parent's state is missing or cannot be read, the fork rebuilds it by replaying the `compress` calls in its history (`rebuild`), at the cost of one cache miss.
- **Child sessions:** a child with history (`child.rs:188`) copies the state as a fork at head does; a child without history starts empty.
- **Model switch:** the context limit changes and with it the thresholds. Refs stay valid, because the session continues.

### 4.6 Opt-in features

The kernel's opt-in features are ported and stay off by default (decision 8):

- **`absorb`** (`absorb.ts`): distil one big tool result in place.
- **The content store** (`ccr.ts`, `content-store.ts`, tool `acp_retrieve`): a big tool result is replaced on arrival by a placeholder, and the original is kept. The originals are stored as files under the session's data directory through `foundation::paths`, written by the history writer's thread, and removed when the session is deleted.
- **`crush`** (`crush.ts`): deterministic folding of JSON, code and logs.
- **`acp_rule`** (`rules.ts`): persistent reminders, offered only when `rules` is on.
- **`filter`** (`filter/`): the message-filter step, with no built-in filters.

### 4.7 Interface and events

- **`/compression`** shows the status panel: session, blocks by tier, compressible ranges, usage, savings. It is the native `/acp` (from `kernel/src/panel/panel.ts`, `report.ts`). With `cache`, it shows the cache report.
- **The status line** gets an optional item with the active blocks and tokens saved.
- **A new event kind, `context_compressed`,** is emitted after a successful fold with the blocks created, the tokens compressed and the tier. It is an *appear* in `exec --json`.
- **The `compress` call** shows in the transcript as a tool cell with its receipt. The terminal interface renders its summary text folded.
- **The attribution** (decision 7) appears in the site's documentation.

## 5. Compatibility, concurrency, errors, resources

### 5.1 Compatibility

**History format.** The format version does not change. These additions are all `#[serde(default)]` or additive:

- `AgentSessionConfig.compaction_strategy`;
- `PersistedAgentRuntimeState.compression`;
- `AgentCompactionPhase::Background`.

Existing sessions, the corpus shared with Hanafubuki and the other corpora open unchanged. **A regression test** decodes the corpus after each phase that touches these types. An older build cannot read data written by a newer one, which the Development Principles allow.

**Goldens.** The goldens of the `standard` strategy stay **byte-stable**: no phase may change what the model sees under `standard`. New goldens pin the following:

- the `hierarchical` request: the static prompt, the tools, the tags, a nudge, a fold, a tier-2 distillation, and the request after a switch away;
- a rebased install (the background suffix layout).

Hanafubuki copies the new goldens.

**Events.** These are *appears* under the event stream's stability rule:

- `phase: "background"`;
- `context_compressed`;
- new snapshot fields.

Hanafubuki's typed decoding must tolerate them; each phase lists the change in its Hanafubuki notes.

**Settings.** The new `[compaction]` table, its fields in `agent_settings_fields()`, and its `settings.fields.*` keys for Hanafubuki's form.

### 5.2 Concurrency

- **`background`:**
  - The task owns only the slot and a detached plan.
  - Install happens on the turn task (or on the `/compact` caller) under the record lock, with the prefix proof checked under that lock. This is Codex's "validate immediately before slicing".
  - The slot is the only shared state. It is written under the lock, and readers never block.
  - `wait_for_pending` waits on the cell's terminal signal with the turn's token, so an interrupt ends the wait.
  - Locks are `parking_lot`; critical sections compute, then assign.
- **`hierarchical`:**
  - The projection is pure, computed under the lock on the record's history and state, as the request is built today. It is not a separate step.
  - `compress` mutates the state in the same staged commit as its tool output.
  - Parallel tool calls in one step are serialized by the record lock. Two `compress` calls in one response apply in order, the second against the first's state.
- **Both:** the strategy changes only when the session is idle (§2.3), so a switch never races a turn.

### 5.3 Errors and fallbacks

| Failure | Handling |
| --- | --- |
| A background run fails (network, protocol, a context error before output) | Logged at debug and discarded. The attempt key prevents a retry loop. A required compaction runs in line. |
| A background result is stale | Discarded at adoption, without events. |
| The adopted result leaves the context over the threshold | One in-line compaction. If that one does not reduce the context either, the turn fails as today (`CompactionDidNotReduceContext`, `ContextBudgetExceeded`). |
| `compress` is invalid (unknown refs, too small, a summary too short or too long, protected) | A zero-applied receipt with the kernel's texts. The state is unchanged (all-or-nothing). |
| The projection panics | The full view is used for the request, the session falls back to `standard` for the turn, and a warning is emitted. The panic is caught at the call (§2.4). |
| The state cannot be decoded | Dropped with a warning and rebuilt from the `compress` calls (§4.5). |
| The context window is exceeded under `hierarchical` | One standard compaction and a retry, as today. A second error fails the turn (`ContextWindowExceededAfterCompaction`). |
| A settings value is out of range | Falls back to its default per field, with a warning. |

### 5.4 Resources

- **Model requests.** A background run is a second concurrent request to the same provider. The turn's request and the background request may compete for rate limits. A 429 on the background request is a failure (no retry storm); the foreground's own retry policy is unchanged.
- **Memory.** A background plan holds a copy of the compacted items and their images while it runs, as `CompactionPlan` does today. `hierarchical` keeps its state in memory with the record; its size is measured in P8.
- **Disk.** `hierarchical` writes its state with each checkpoint (§4.4). The content store, when enabled, writes files under the session's data directory, removed with the session.
- **Processes.** Neither strategy starts processes.

### 5.5 Coexistence

- **One strategy per session.** `background` and `hierarchical` never both act on a session.
- **The standard compaction is shared and unchanged.** Both strategies call it as their floor. The P1 refactor serves both without changing its behavior, and the `standard` goldens prove it.
- **Separate state.** The background slot, which is not persisted, the hierarchical state, which is persisted, and `runtime.compaction` are three separate fields.
- **Separate code.** `service/compaction/background/`, `service/compression/` and `crates/compression` are separate modules. Neither strategy imports the other.

## 6. Licence, notices, attribution

- **Codex** (Apache-2.0) is already a source of this repository. The ported background files carry the Codex revision marker `57a808a392` in `architecture/notices-policy.toml`, like the P0–P14 ports.
- **billion-context** (MIT plus the attribution term, decision 7):
  - `crates/compression` and the files that carry its prompt texts get a derived-from entry with a billion-context marker in `notices-policy.toml`.
  - `NOTICE` gets the copyright line, the MIT text and the additional term, with links to both repositories.
  - The site credits it (both languages), in the Licence list on the contributing page and on the configuration page for `[compaction]`. The term allows credit in the documentation, so that meets it.
  - `cargo xtask check-notices` checks the markers.
- Nothing is ported from DCP or `opencode-acp`, which are AGPL.

## 7. Phases

Each phase is committed and pushed. Each phase also does the following:

- **Validation:** runs the validation in AGENTS.md (format, check, clippy, `cargo xtask check`, tests; the site checks when the docs change). For Windows it runs the cross-check and the CI job.
- **Goldens:** regenerates them only on purpose.
- **Module size:** reports any exception.
- **Hanafubuki:** lists its pin notes in `todo.md`.
- **Docs:** updates both languages.

| Phase | Content | Done when |
| --- | --- | --- |
| **P1** Split compaction | `service/compaction/` split along responsibilities: source and plan (`source.rs`), replacement build as a pure function of the plan, install (`commit.rs` with `InstallMode::Cas` only), status machine. No behavior change. | Every compaction contract test and golden unchanged; `commit_compaction` under the function limit. |
| **P2** The strategy setting | `[compaction]` in the config crate (decode, defaults, patch, schema, `config get/set`); `AgentSessionConfig.compaction_strategy`; the facade applies it at turn admission; transitions; `/config` row; docs (configuration, reference, architecture). Fix the `remoteCompaction` bug (decision 11): `activate_model_session` updates the session's `compaction_capability` with the transport, when idle. Check the Anthropic empty-`tools` case (§2.3). | Corpus decodes; a session switches strategies between turns; the bug's regression test passes. |
| **P3** Background in the kernel | §3.2–§3.5: the slot, launch, adoption, `wait_for_pending`, the rebase install, cancellation, shutdown, usage, `AgentCompactionPhase::Background`. | §3.7 tests pass; `standard` goldens unchanged; a rebased-install golden. |
| **P4** Background in the interface and docs | A "Context compacted" cell for every install; `/status` background state; `exec --json` docs; the architecture page's "Not built" line (`architecture.en.mdx:250`, which also lists "a third compaction path") and Codex comparison row; internals (the rebase install). | A TUI test covering adoption while a turn runs. |
| **P5** The algorithm crate's core | `model`, `refs`, `boundaries`, `parse`, `project`, `protect`, `apply`, `prune`, `render` with their ported tests; crate registration (§2.4); notices (§6). | Ported tests pass. |
| **P6** Recommend, nudge, truncation, prompts | `recommend`, `nudge`, `truncate`, `latch`, `turn` (the full pipeline), `prompts`, `report`, with the byte-exact prompt golden. | Ported tests pass. |
| **P7** Kernel integration | §4.3: adapter, projection in the request builder, the five core tools, the nudge, thresholds off, the fallbacks, terminal escape; §4.4 state; hierarchical goldens. | A model-scripted session folds, distils to tier 2, decompresses, and keeps its `standard` goldens. |
| **P8** Forks, resume, measurements | §4.5: block adoption on forks and children, `rebuild` when the state is missing; state-size measurement and, if needed, writing the state only when it changed; switching away from `hierarchical` end to end. | Fork and resume tests on both sides of a fold. |
| **P9** Decompress, search, media | `decompress` (inline, full, `toFile` under the data directory), `search` (hybrid BM25), `media`. | Ported tests pass. |
| **P10** Opt-in features | §4.6: `absorb`, the content store, `crush`, `acp_rule`, `filter`, all off by default. | Ported tests pass; defaults change nothing. |
| **P11** Interface and docs | `/compression` panel and `cache`; status-line item; `context_compressed` event; the configuration, architecture, internals, terminal-interface, json-events and plugin-rules pages; attribution. | Pty smoke on the binary: a fold under nudges, `/compression`, a switch to `standard`. |

Each phase depends on the ones before it, except that P5–P6 depend only on P2 and can run alongside P3–P4.

**As built** (todo.md is the record):

- **Line references** are at `2907aee`. P1 moved the compaction code into `source.rs`, `replacement.rs`, `status.rs` and the rest, so the plan's references to `commit.rs`, `prepare.rs` and `compaction/mod.rs` predate that move.
- **Parallel work and splits.** P5–P6 ran alongside P3–P4. P9 and P10 each landed as a crate half and a kernel half.
- **P8b was added.** P8 measured the hierarchical state at over 90% of each checkpoint, so P8b persists it incrementally as a chain of snapshots and deltas. The history index went to schema 6.
- **P12 came after the plan.** It ports billion-context's message timestamps, so `messageTimestamps` acts.
- **§3.4 was redesigned before P3,** after the fact-check: the live suffix keeps its ids inside `replacement_history`, and the cutoff stays the display item.
- **P4 cells.** Every completed compaction's event renders the one "Context compacted" cell; `/compact`'s reply reports only failures.

## 8. Decisions

Each decision has a recommendation. **All were accepted as recommended (2026-11-09)**, except decision 12, decided as written there.

1. **Where the setting lives.** *Recommended:* `config.toml` `[compaction]`, because it is layered, has `config set`, reaches Hanafubuki's form, and is read per call, so a switch applies at the next turn. *Alternative:* `providers.toml` `[modelPolicy]` beside the other compaction keys. That file is frozen per session until `/model`, so a switch would apply only to new sessions.
2. **Names.** *Recommended:* `standard`, `background`, `hierarchical`. The model-facing names stay billion-context's (decision 6).
3. **What `background` runs.** *Recommended:* the session's implementation on any provider, with the snapshot ending at the latest `TurnStart` (§3.1). *Alternative:* Codex's scope exactly, remote v2 only, off for every other provider. That is less useful, and it still could not compact a turn in progress here.
4. **The default strategy.** *Recommended:* `standard`, so nothing changes for anyone who does not opt in. Codex ships async compaction off.
5. **Where `hierarchical` lives.** *Recommended:* a pure crate plus kernel integration, with panics contained at the call (§2.4). *Alternative:* a kardia plugin with a new registry, rejected because the kernel would depend on a plugin for every request.
6. **Model-facing texts.** *Recommended:* keep billion-context's tool names, schemas, prompt and rule texts, nudge texts and `<acp>` tags verbatim. They are tuned and refer to each other. Tags go on message and tool-output text, never on tool-call arguments. *Alternative:* rename them (for example `context_status`, `<ctx>`), which means editing tuned text.
7. **The licence term.** *Recommended:* accept it. Port the texts, add the `NOTICE` entry and the derived-file markers, and credit billion-context and acp-kernel with links on the site (§6). It reads as a GPLv3 §7(b) attribution term, but that is a reading, not legal advice; this decision is yours. *Alternative:* reimplement the algorithm from its description and write new prompt texts, which needs no licence but loses the tuned texts.
8. **Scope of the algorithm.** *Recommended:* port the core (P5–P9) and every opt-in kernel feature (P10), off by default. Exclusions are listed in §9.
9. **Child and ephemeral sessions.** *Recommended:* children take the setting (`background` respects `childSessions`); ephemeral sessions always run `standard`.
10. **Terminal escape under `hierarchical`.** *Recommended:* run one standard compaction automatically. *Alternative:* only warn, as billion-context does.
11. **The `remoteCompaction` bug (§1.3).** *Recommended:* fix it in P2. The fix touches the same code, and a strategy switch would otherwise meet the same failure.
12. **The previous plan.** Decided: `docs/plan-codex-tui-port.md` stays (P14e is open by decision in `todo.md`), and AGENTS.md now allows one plan per piece of work under `docs/`.

## 9. Not ported

**From Codex:**

- the remote-v2-only gating and `RemoteCompactionSupport` (replaced by decision 3);
- the routing-token replay (`x-codex-turn-state`) and the HTTP-versus-websocket split;
- Guardian's compaction hash and review checkpoints;
- uploaded-audio rejection and the image-store expiry lookup;
- incremental tools;
- MCP attribution and the model-search checkpoint;
- host usage accounting (`AgentUsageAccounting`), analytics and OTel metrics;
- Codex's `PreCompact`/`PostCompact` hooks (the hooks domain reports them as unsupported today);
- post-turn compaction (`model_post_turn_compact_threshold_percent`, default 0). The Agent has no post-turn compaction checkpoint, so a result launched at the end of a turn is adopted at the next turn, which is what Codex does by default.

**From billion-context:**

| Not ported | Reason |
| --- | --- |
| The proxy and everything in `src/`, the web panel, HTTP endpoints, updates, billing | Infrastructure for a proxy that sits outside a client. The Agent owns its history. |
| `kernel/src/wire/` and content-hash identity | The Agent's items have stable ids. |
| `kernel/src/persist/` (including the lossy `mergeCompressionState`, `persist/state-merge.ts:12`) and bili's SQLite/MySQL store | The state persists in the Agent's history store (§4.4), and every state field has a serde default. |
| `instance-reid.ts` | Only needed for content-hash ids. |
| The text and hybrid `compress` protocols | The Agent registers real tools. |
| Prompt packs and surface overrides | Only the default texts are kept. Overriding the tuned texts is not offered. |
| The external summarizer, preflight `autoFold`, `nudgeModelDecided` | Side paths with extra model calls, not the core design. `background` covers model-independent compaction. |
| `relocate-compress.ts` (`compressAtFoldPoint`) | Experimental and off in billion-context. |
| `output-steering.ts` | Steers verbosity and effort; unrelated to compression. |
| Semantic search in `search_context` | Needs embeddings the Agent does not have. BM25, fuzzy and hybrid search are ported. |
| `handoff.ts` | Markdown export, which `/export` already does. |
