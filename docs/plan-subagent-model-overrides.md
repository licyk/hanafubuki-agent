# Plan: a sub-agent's model, Provider and reasoning effort, chosen at each spawn

Repositories, read from source on 2026-11-13:

- The Agent: `/root/code_workspace/hanafubuki-agent`, main `be56861`.
- Codex: `/root/code_workspace/codex`, `57a808a392` (2026-11-07), the
  revision the subagent plugin's "Derived from" headers name. Paths below
  are under `codex-rs/core/src/` when they are Codex's.

Code is named by symbol, not by line: the milestones of
`plan-shortcuts-rendering-and-context.md` land first and move lines.

The request has four parts. Numbers in brackets refer to them throughout:

- [1] The main agent chooses a sub-agent's **model** and **reasoning
  effort** freely at each `spawn_agent`, with no predefined role.
- [2] How it passes them: the tool's arguments, and what it is told is
  available.
- [3] How they reach the child: resolved, checked, and applied while the
  child session is initialised, before its first turn.
- [4] How this fits the existing design: roles, the model selection, the
  model metadata, the guidance, the facade, the terminal interface, the
  goldens and the docs.

The `todo.md` section "Plan: a sub-agent's model, Provider and reasoning
effort, chosen at each spawn" tracks the milestones. They run after the
shortcuts plan's last milestone (M8).

A path this plan names that does not exist yet is a file its milestone
creates; it is marked "(new)".

## 0. Terms

- **Parent**: the top-level session whose model calls `spawn_agent`.
  **Child**: the sub-agent session it creates (origin `Subagent`).
- **Pair**: a Provider id and a model id. The child runs a pair; so does
  the parent.
- **Recorded listing**: the model ids of a Provider's latest listing as the
  kernel's model metadata keeps it (`ListingState.models` in
  `crates/kernel/src/model_metadata/state.rs`), read without a request.
  A live listing (`list_provider_models`) is a network request and is never
  made by a spawn.
- **Known levels**: the reasoning efforts a model is known to take, by the
  rule of the shortcuts plan's §2.6 (E1): the resolved reasoning fact's
  efforts when its source is `Configured` or `Reported`.
- **Explicit**: a value the model passed as an argument of this
  `spawn_agent` call, as opposed to one the role or the parent gives.

## 1. The request against the code

The survey this plan started from was checked against the code. Where the
code says otherwise, the design follows the code:

| Premise | What the code says | Consequence |
| --- | --- | --- |
| `spawn_agent` takes `task_name`, `message`, `fork_turns`, `agent_type` | Correct (`spec` and `spawn_arguments` in `crates/agent/src/plugins/subagents/tools.rs`; `SpawnRequest` in `service.rs`). The file's Codex notice says "so there is no `model` or `reasoning_effort`". | Three properties added (§2.1); the notice is rewritten in O1. |
| `spawn_arguments` is where the arguments are validated | It is synchronous and holds only the role catalog. A Provider, a recorded listing or the parent's pair needs the settings and the kernel. | `spawn_arguments` parses and bounds the strings; the check against Providers, listings and levels runs in `SubagentService::spawn` through the session driver, **before `create_child`**, with field-named errors as `agent_type`'s (§2.3). |
| The role's model is applied with the parent's Provider | Correct (`AgentSessionDriver::select_role_model`, `crates/agent/src/session_driver/model.rs`). But it skips the selection when the model id equals the parent's (`model.filter(\|model\| *model != parent.model)`). | Compare the **pair**. Otherwise `provider` with the parent's model id would silently keep the parent's Provider (§2.4). |
| A failed role selection is a tool error | It is, but `spawn` has already created and renamed the child (`driver.create_child`, `driver.rename`), so a durable, idle child keeps the task name. | Everything is resolved and checked before the child exists. Only a race remains (D16). |
| The child copies the parent's config, budget and input capabilities | Correct (`child_record`, `crates/kernel/src/service/session/child.rs`). But when the model changes, `select_model` replaces more than the model: `set_session_model_configuration` (`crates/kernel/src/service/session/configuration.rs`) takes the **global** model policy's effort, input-capability policy, compaction settings and turn budget. There is no per-Provider policy (`AgentModelPolicySettings` is one, in `crates/config/src/providers/mod.rs`). | The "budget question" is narrower than assumed: a child on another pair gets the current global turn budget, a child on the parent's pair keeps the parent's copy, and both come from the same policy (§2.7). |
| The child's context window and image capability must follow the new model | They do, inside `set_session_model_configuration`: `session_context_window`, `usage.context_window_tokens` and `effective_input_capabilities` (`session_input_capabilities`) are refreshed there. | The spawn path is covered **only when it calls the model selection**, hence the pair comparison above. No new refresh code. |
| The Provider's known listing can be checked | The kernel keeps it per endpoint, but exposes only `listed_at_ms` (`ModelMetadataRegistry`, `crates/kernel/src/model_metadata/mod.rs`). | A new kernel read, `listed_models` (§2.3). |
| Effort levels come from `ModelCatalogEntry.supported_reasoning_efforts` | The listing's levels are recorded as the route's `Reported` reasoning fact, resolved per fact with configuration first (`ModelMetadata.reasoning`, `crates/model-api/src/model_metadata.rs`). E1 already defines which sources count. | Reuse E1's rule through one model-api accessor that E1's `effort_options` moves onto (D9). |
| Providers are named by id | Ids are allocated as `provider-1` … `provider-33` (`allocate_provider_id`, `crates/agent/src/service/provider/registry.rs`): opaque. | The guidance shows the display name beside the id; a unique display name is also accepted (D12). |
| `exec --json` may report sub-agents | It does not: `json-events` and `exec` document no sub-agent event, and `crates/cli/src` names none. A child's events land in its own journal (`embedding#feature-calls`). | Nothing to change there. |
| 13 goldens carry `spawn_agent` | Correct. Only 11 carry the guidance (`## Sub-agents`): `side_conversation_request.json` and `review_request.json` are children, which see the spec but not the guidance. | O1 changes 13 goldens (the spec), O2 changes 11 (the guidance). |
| The tool description should list the models (Codex) | A `ToolDescriptor` is fixed per registration (plugin rules, "Tools"); `spawn_agent`'s already names no role for that reason and points at the guidance, which reads the catalog at each step (`SubagentGuidancePrompt`). | The models go in the guidance (§2.5, D7). |
| Codex's precedence is ours | Not quite. Codex applies the requested model, then the role over it (`prepare_agent_spawn_config`, `agent/child_config.rs`), so a role's model beats the argument; and an explicit model without an explicit effort takes **the model's default effort**, not the parent's (`apply_requested_spawn_agent_model_overrides`). Codex also lists at most 5 models (`MAX_SPAWN_AGENT_MODEL_OVERRIDES`) from an offline listing (`RefreshStrategy::Offline`). | Ours differs on both points, by the given decision D2; flagged for confirmation in D2's row. The offline listing matches our recorded listing. |
| `plugin-rules` describes `agent_type`'s roles | Its "Host, configuration and status" bullet still says the spec lists the roles of the plugin's start; the code and the "plugins today" row say the spec names none. | O1 fixes the stale sentence with the rest. |

## 2. Design

### 2.1 The arguments [1][2]

`spawn_agent` gains three optional string properties. Each is read with
`nullable_bounded_string` and trimmed; missing, `null` or empty is absent
(plugin rules, "Model tool arguments"). An unknown key is still ignored.

| Property | Bound | Description (model-visible) |
| --- | --- | --- |
| `model` | 256 bytes (the kernel's model id bound) | "Model id for the new agent: one the sub-agent guidance lists, or another its Provider serves. Defaults to the role's model, else yours." |
| `reasoning_effort` | 64 bytes (`MAX_MODEL_REASONING_EFFORT_BYTES`) | "Reasoning effort for the new agent, one its model takes (the guidance lists them where known). Defaults to the role's, else yours." |
| `provider` | 128 bytes | "Provider of `model`: the id (or name) of an enabled Provider, as the sub-agent guidance lists them. Defaults to yours." |

- They work with and without `agent_type`.
- `reasoning_effort` is parsed at once with `ModelReasoningEffort::parse`
  (`crates/foundation/src/agent_settings.rs`). Its refusal ("`none` is not
  supported", a bad character) is the field's error.
- `SpawnRequest` gains `model: Option<String>`, `provider: Option<String>`
  and `reasoning_effort: Option<ModelReasoningEffort>`.
- `SPAWN_AGENT_DESCRIPTION` says the agent runs "with your model and your
  tools unless `model`, `provider`, `reasoning_effort` or its `agent_type`
  say otherwise". `agent_type`'s description drops "and may run another
  model" for "and may set a model".
- The wording is neutral (D4): neither Codex's "do not set unless the user
  asks" nor an encouragement.

### 2.2 Precedence [3]

Per field, explicit over the role over the parent:

| `provider` | `model` | Child's Provider | Child's model | Pair checked (§2.3) |
| --- | --- | --- | --- | --- |
| — | — | parent's | role's, else parent's | no (as today) |
| — | `M` | parent's | `M` | `M` on the parent's Provider |
| `P` | — | `P` | role's, else the parent's model id | that id on `P` |
| `P` | `M` | `P` | `M` | `M` on `P` |

- **Effort**: explicit, else the role's, else the parent's, whatever the
  model. So an explicit model without an explicit effort keeps the role's
  effort, else the parent's (D2). Narrowing at request time
  (`request_reasoning_effort`, `crates/model-api/src/reasoning_effort.rs`)
  stays the safety net for an inherited effort the new model does not take.
- A role's `model` is a bare id; with `provider` it is read on `P`. A role
  gains no `provider` key in this plan (§5).
- **No change**: when the resolved pair equals the parent's, no model
  selection runs. The child keeps the parent's copy of the configuration
  and budget; only a different effort is set.

### 2.3 Resolution and the checks [3]

A new driver call, `AgentSessionDriver::resolve_child_model(parent,
ChildModelRequest) -> Result<ChildModel, ChildModelError>`
(`crates/agent/src/session_driver/model.rs`), runs on the blocking service
before `create_child`. `ChildModelRequest` carries the explicit values and
the role's `model` and `reasoning_effort`. `ChildModel` is the typed result
the rest of the spawn uses: the resolved Provider (`ResolvedAgentProvider`),
the model id, the effort, and whether the pair differs from the parent's.
`ChildModelError` names its field (`provider`, `model`,
`reasoning_effort`) and becomes `ToolCallValidationError::InvalidArgumentAt`
in the plugin, so the model reads "invalid tool argument model: …" as the
tool result. Nothing panics, and no child is made.

**Provider** (only when explicit):

- The registry is read through `AgentConfig::providers` (cached).
- `P` matches a Provider's id, else exactly one Provider's display name,
  case-insensitively (D12).
- It must be enabled. The error lists every enabled Provider (at most
  `MAX_AGENT_PROVIDERS`, 32) as `` `provider-2` (Local) ``: "`X` is not an
  enabled Provider; use one of …".
- Resolution reuses `resolve_settings_provider`
  (`crates/agent/src/service/provider/resolution.rs`), so a token or header
  that no longer parses is refused here too, never after the child exists.

**Model** (only when `model` or `provider` was explicit, per §2.2's last
column):

- The **known models** of a Provider are the union of:
  - its recorded listing, through a new kernel read:
    - `ModelMetadataRegistry::listed_models(&ModelEndpointKey) ->
      Option<BTreeSet<String>>`, the `models` of the endpoint's
      `ListingState`;
    - `AgentService::listed_models(protocol, base_url)` in
      `crates/kernel/src/service/model_facts.rs`;
  - the ids the person configured for it (`models."<id>"`,
    `AgentProviderSettings.models`), a cheap escape for a gateway whose
    listing is incomplete (D8);
  - the global policy's model, when `P` is the default Provider;
  - the parent's model, when `P` is the parent's Provider.
- **No recorded listing** (discovery off, not yet run, failed): the model is
  passed through. An unknown id then fails at the child's first request, as
  a role's does today, and `wait_agent` reports the error.
- **A recorded listing without the id**: the error lists up to 20 known
  ids, those sharing the longest prefix with the request first, then by id
  (D19), and how many more there are: "`gpt-5-mini` is not a model
  Provider `provider-1` (OpenAI) lists; closest: `gpt-5.4-mini`, … (and 212
  more)". With `provider` and no `model`, the message says the inherited id
  is missing on `P` and asks for a `model`.

**Effort** (only when explicit):

- Checked against the **known levels** of the resolved pair: the route's
  `model_metadata` (kernel `AgentService::model_metadata` with
  `configured_model_facts`) read through a new model-api accessor,
  `ModelMetadata::known_reasoning() -> KnownReasoning { Levels { efforts,
  default }, Unsupported, Unknown }`, over the resolved fact. That fact is
  one winner: configured, then learned in the safe direction, then
  reported, then models.dev (`crates/kernel/src/model_metadata/resolve.rs`).
  The accessor follows E1's rule (D9):
  - `Levels`: a `Configured` or `Reported` fact with levels. The effort
    must be one of them, exactly (D10). The error lists them with the
    default marked: "`xhigh` is not a reasoning effort `gpt-5.4-mini`
    takes; use one of low, medium (default), high".
  - `Unsupported`: a `Configured` (`reasoningEfforts = []`), `Reported` or
    `Learned` (a refusal within its 7 days) "no reasoning". The error is
    "`M` takes no reasoning effort; omit `reasoning_effort`": the effort
    would be dropped from every request anyway.
  - `Unknown` (no listing, levels not reported, only models.dev): passed
    through, and narrowed at request time as today.
- A role's effort and the parent's are never checked (D11): neither is the
  model's choice, and narrowing covers them as it does today.

Every check reads in-memory state: the cached registry and the kernel's
model metadata. A spawn makes no network request. It does ask discovery for
`P`'s listing (`ModelDiscovery::request`, as `select_model` does), so a
Provider never listed is checked from its next spawn on.

### 2.4 Applying it while the child is initialised [3]

`SubagentService::spawn`, in order:

1. The existing refusals: a child cannot spawn, the task name is taken,
   capacity.
2. `resolve_child_model` (§2.3). A refusal ends the call here, with nothing
   created.
3. `create_child` and `rename`, as today: the child is the parent's copy,
   narrowed by the role's scope, with the role's instructions.
4. `apply_child_model(child, ChildModel)`, replacing `select_role_model`:
   - **Pair differs**: `select_model` is split into the Provider's
     resolution and `apply_model_selection(session, generation, revision,
     resolved, model, reasoning_effort)`. That second half runs
     `set_session_model_configuration` with `P`'s protocol, base URL,
     credential and headers, the global policy, `configured_model_facts`
     for `M`, and the effort given (the facade's `set_session_model` passes
     the policy's, unchanged behaviour). One commit sets the pair, the
     effort, the context window and its source, `usage`'s window, the
     effective input capabilities, the compaction settings and the turn
     budget (D13).
   - **Pair equal, effort differs**: `set_session_reasoning_effort`, as
     today.
   - Base instructions are `resolve_instructions(None)` of the new model;
     the role's instructions are kept (`role_instructions`, untouched by
     the model selection), as the role tests already assert.
5. `start`: the child inherits the parent's permission mode, and its turn
   runs through `run_turn`, whose `activate_provider_session` binds the
   child to **its own** Provider at every turn (credential, headers,
   compaction capability, configured facts). A later `followup_task` runs on
   the child's pair. A Provider disabled meanwhile fails that turn ("Agent
   Provider … is disabled"), reported by `wait_agent`, as for any session.
6. The result: `{ task_name, agent_id, provider, model, reasoning_effort }`,
   read from the child's snapshot after step 4, so the model learns what
   was inherited (`reasoning_effort` is `null` for the Provider's default)
   (D18).

**Forked history on another pair.** With `fork_turns` `all` (the default)
the child's history is the parent's before the running turn. A `/model`
switch on a long session puts history on another pair through the same
`set_session_model_configuration`, so this plan designs no new handling.
What that path is expected to do, which this plan did not trace to the end
and O1's tests pin:

- images in history follow the child's effective input capabilities (the
  request builder, `crates/kernel/src/service/sampling/images.rs`, loads
  what the history references; the capability gate is the existing one);
- reasoning replay blocks go back as stored (`reasoning_input` in
  `crates/kernel/src/model/conversation_item.rs` filters by turn, not by
  route), so a replay another Provider rejects is the existing `/model`
  case, not a new one;
- the turn loop's `maybe_auto_compact` compacts against the child's window
  if the history no longer fits.

If a test shows one of these does not hold, O1 records it and fixes it on
the shared path, since `/model` has the same gap. The parent's conversation
reaching another endpoint without an approval is open question Q2.

### 2.5 What the model is told [2]

**Where.** In the guidance (`subagent_guidance`, `crates/agent/src/plugins/subagents/prompt.rs`),
not the tool description (D7):

- `spawn_agent`'s descriptor is fixed per registration and shared by every
  session. Listing models there means restarting the plugin at each catalog
  change, and the tool definitions sit at the front of every request's
  cached prefix, ahead of the conversation, so each change would re-bill
  all of it for every session.
- The guidance is a prompt fragment, read at each sampling step and shown
  only to a session that may spawn. That is where the roles already are.

**What.** Under the role list, a "Models" part:

```text
`spawn_agent`'s `model`, `provider` and `reasoning_effort` choose a sub-agent's model and reasoning effort; without them it runs its role's, else yours (`gpt-5.4` of `provider-1`).
- `provider-1` (OpenAI), yours:
  - `gpt-5.4`: reasoning efforts low, medium (default), high, xhigh
  - `gpt-5.4-mini`: reasoning efforts low, medium (default), high
  - … and 212 more it lists
- `provider-2` (Local): no listing known; a model id is passed to it as given
```

- **Providers**: the enabled ones, the parent's first, then by display name
  (lowercase) and id, as `list_provider_models` orders them.
- **Models per Provider**, deduplicated, in this order:
  - the parent's model (on its Provider);
  - the configured ids (`models."<id>"`, in key order);
  - the policy's model (on the default Provider);
  - the recorded listing, by id.
- **Efforts**: `known_reasoning` (§2.3). `Levels` are shown with the
  default marked; `Unsupported` reads "takes no reasoning effort"; nothing
  is shown when unknown.
- **Bounds** (D7):
  - at most 8 Providers and 8 models each, then "… and N more it lists";
  - at most 8 efforts per model, each cut at 64 characters (Codex's);
  - an id over 128 characters is left out rather than cut, since a cut id
    cannot be passed;
  - display names on one line, cut at 64 characters;
  - **the Models part at most 4 KiB** (`MAXIMUM_MODELS_GUIDANCE_BYTES`,
    about 1 K tokens): lines are added in the order above and the first
    that would pass the cap ends the part with "… more models left out;
    the Provider's other ids are still accepted". The per-item bounds alone
    allow tens of KiB (64 lines of 128-character ids with 8 efforts each),
    so the byte cap is what bounds the request.
  - Typically a few hundred bytes: a Provider or two, the parent's model
    and some configured or listed ones.
- **Never shown**: base URLs, tokens, headers.

**How.** `contribute` is pure (plugin rules, "Prompt fragments"), so:

- `SubagentGuidancePrompt::prepare`, which already reads the session on
  the blocking service, also builds the bounded `SpawnModelCatalog`
  through a new driver call, `spawn_model_catalog(parent)`. It reads the
  cached registry, `listed_models` and `known_reasoning` per listed model:
  at most 64 in-memory resolves per step.
- It stores the catalog in the prepared value `MaySpawn`, and `contribute`
  renders it. The code lives in a new module,
  `crates/agent/src/plugins/subagents/models.rs` (new), shared with the
  checks of §2.3, so the list and the checks cannot disagree.

**The provider's cached prefix.** The fragment joins the conversation's
opening run (Capabilities slot, order 130), so any byte that changes in it
re-bills the prefix from there on. The rendering therefore depends only on:

- the person's settings: Providers, `models."<id>"`, the policy's model;
- the recorded listings;
- a learned "no reasoning", the one learned fact `known_reasoning` reads
  (D9);
- the parent's pair.

It never depends on times, the other learned facts or the parent's effort.
Order is by id, never by observation time. A change costs one cache miss
when:

- the person edits Providers;
- a Provider refuses a listed model's reasoning parameter: its levels give
  way to "takes no reasoning effort", and come back when the learned fact
  expires 7 days later, or is forgotten. Two misses a week at most per
  such route; reading below the learned layer would avoid them, but needs
  the second accessor E1 decided against;
- a listing with different content is recorded. Discovery lists a Provider
  at most once a day, and on a fresh data directory the first listing
  lands during the first turn: one miss;
- the parent switches pair. That already changes the cache, which providers
  key by model.

The same rendering serves every request of a step (plugin rules). Tests
run with discovery off, so the goldens are deterministic.

### 2.6 Seeing a child's model [4]

`AgentSessionSummary` already carries `provider_id`, `model` and
`reasoning_effort` (`crates/kernel/src/model/session.rs`), so no new read
is needed:

- **`list_agents`**: each agent gains `provider`, `model` and
  `reasoning_effort` beside `agent_type`.
  - `SubagentService::agents` keeps each child's `ChildModelView` (the
    three fields) from its summary beside its `ListedAgent`, so the values
    are the session's current ones (a person's `/model` on a child shows),
    never a copy kept at spawn.
  - `wait_agent`'s output is unchanged.
- **The facade**: `AgentSubagentStatus` gains `provider_id: String`,
  `model: String`, `reasoning_effort: Option<ModelReasoningEffort>`
  (camelCase in JSON), filled by `SubagentService::statuses`. Public:
  Hanafubuki adapts at its next pin.
- **The terminal interface**:
  - `/subagents`' status cell shows `title  state · model · effort` (the
    effort `default` when none);
  - the picker's description starts with `state · model · effort`;
  - files: `crates/tui/src/commands/subagents/{mod,picker}.rs`.
  - The command center already shows a child's model and reasoning from its
    summary (`crates/tui/src/agents_center/details.rs`): unchanged.
- **`exec --json`**: reports no sub-agent (§1): unchanged.

### 2.7 What does not change [4]

- **Approval**: `spawn_agent` stays `PermissionRequirement::NotRequired`;
  the child's own calls are approved in the child (Q1, Q2).
- **`followup_task` and `send_message`** take no model (D15). A child keeps
  its pair; a person can still change it with `/model` while viewing the
  child.
- **Budget**: the turn budget is the global policy's either way (§1). A
  child on a dearer model costs more per turn; no per-child cap is added
  (Q3).
- **Defaults**: no `[pluginOptions.subagents]` `defaultModel` or
  `defaultReasoningEffort` (D14, Q4).
- **Roles**: their keys, merge and checks are unchanged; a role's model is
  still unchecked when no argument names a pair (D11).
- **Depth and capacity**: unchanged.

### 2.8 Text that becomes stale

Each milestone rewrites, in both languages where it is a page:

- O1:
  - the Codex notice of `crates/agent/src/plugins/subagents/tools.rs` (the
    "so there is no `model` or `reasoning_effort`" clause) and the module
    docs of `plugins/subagents/mod.rs` and `service.rs`;
  - `plugin-rules`: the "Host, configuration and status" bullet about
    `agent_type`'s roles, and the `subagents` row of "The plugins today";
  - `architecture`: the "Subagents" paragraph (the model selection, the
    pair, the checks before creation);
  - `sessions`: `#children` ("with its parent's model and settings") and
    `#subagents`;
  - `roles`: `#subagents`, and the `model` row of `#keys` ("of the
    parent's Provider" unless `provider` says otherwise);
  - `tools#subagents`: the arguments column and the description.
- O2:
  - `prompt.rs`'s Codex notice (the guidance ends with the roles and the
    models);
  - `tools#subagents` (the Guidance bullet);
  - `roles#subagents` ("What the model reads");
  - the `subagents` row of `plugin-rules`.
- O3:
  - `embedding#feature-calls` (the `subagents(parent)` row);
  - `terminal-interface#subagents`;
  - `tools#subagents` (`list_agents`).

`config-reference` changes only if Q4 adds options.

## 3. Milestones

Strictly sequential, after the shortcuts plan's M8. Each milestone is one
commit on main, pushed, with green CI, before the next starts:

**O1 → O2 → O3**

Each milestone, when done, records in `todo.md`:

- what landed and any deviation from this plan;
- the validation commands that ran and those skipped;
- the CI run's URL (Linux, Windows, macOS from the workflow, never claimed
  from a Linux run);
- whether goldens changed;
- what Hanafubuki must change at its next pin;
- any module-size exception added or raised. `service.rs` is close to its
  600-line limit, which is why the new code goes to `models.rs` and the
  driver.

Every milestone runs AGENTS.md's validation matrix:

- `cargo fmt --all --check`;
- `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
  --all-features`;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
- `cargo xtask check`;
- `cargo test --workspace --all-features`;
- when it touches `site/`, the site's: `bun install --frozen-lockfile`,
  `bun run format:check:site`, `bun run site:check`, `bun run site:test`,
  `VITE_BASE_PATH=/hanafubuki-agent/ bun run site:build`, every page
  prerendered.

A model-visible change regenerates the goldens **on purpose**
(`HANAFUBUKI_AGENT_UPDATE_GOLDEN=1 cargo test -p hanafubuki-agent --lib
golden`) and the fixture diff is reviewed. Each milestone changes the docs
of what it changes, in both languages.

- **O1 — the arguments, the checks and the child's pair** (§2.1–§2.4,
  §2.7).
  - Files:
    - `model-api/src/model_metadata.rs` (`known_reasoning`, `KnownReasoning`);
    - `kernel/src/model_metadata/mod.rs` (`listed_models`) and
      `kernel/src/service/model_facts.rs` (`AgentService::listed_models`);
    - `agent/src/session_driver/model.rs` (`select_model` split into
      resolution and `apply_model_selection`; `resolve_child_model` and
      `apply_child_model` replace `select_role_model`);
    - `agent/src/service/provider/selection.rs` (the facade's path over the
      split);
    - `agent/src/plugins/subagents/models.rs` (new: the known-models set,
      the closest-ids order, the error texts);
    - `agent/src/plugins/subagents/{tools,service,mod}.rs`;
    - `tui/src/model_facts.rs` (E1's `effort_options` over
      `known_reasoning`: one rule).
  - Tests:
    - `crates/agent/src/plugins/subagents/tests/models.rs` (new), over the
      fake server (`crates/agent/src/test_support/model_server.rs`) and a
      second Provider written beside `configure_test_provider`:
      - an explicit model without a role: the child's request carries it
        on the parent's Provider, with the parent's effort;
      - an explicit model with a role setting model and effort: the
        argument's model, the role's effort;
      - explicit effort over a role's;
      - `provider` alone: the parent's model id on `P` when listed, the
        field error when `P`'s listing lacks it;
      - cross-Provider: the child's first request reaches the second
        server with its model; the parent stays on the first; the child's
        snapshot names `provider-2`; its window and image capability are
        `provider-2`'s listing's;
      - the parent's model id on another Provider (the pair bug): the
        Provider changes;
      - an unknown model with a recorded listing (recorded by
        `list_provider_models` against the fake server): the error names
        `model` and lists the closest ids; no child exists and the task
        name is free;
      - an unknown model without a listing: the spawn succeeds and
        `wait_agent` reports the first request's error;
      - a configured `models."<id>"` counts as known;
      - an effort outside known levels: the error lists them;
        `reasoningEfforts = []`: "takes no reasoning effort"; unknown
        levels: passed through and narrowed in the request;
      - `none` refused by `parse`;
      - a disabled or unknown Provider: the error lists the enabled ones;
        a display name accepted;
      - an equal pair: no model selection (the child's revision after
        spawn shows a single effort change, or none);
      - the spawn result's `provider`, `model`, `reasoning_effort`;
      - a fork onto a model with a smaller listed window: the child's
        window is that one, and its first request is admitted against it
        (or compacts first);
      - a fork of a history holding an image onto a model configured
        without image input: the child's request follows the existing
        capability gate (§2.4's expectations, pinned).
    - `plugins/subagents/tests/roles.rs`: the existing role-model test
      holds unchanged (no argument: as today).
    - The kernel: `listed_models` before any listing (none), after one, and
      across the shrink guard (`kernel/src/model_metadata/tests`).
    - model-api: `known_reasoning` per source (`Configured`, `Reported`,
      `Learned`, `Catalog`, empty levels).
    - E1's option tests still pass over the shared accessor.
  - Goldens: the 13 carrying `spawn_agent`'s spec, regenerated on purpose.
  - Docs (§2.8, O1): `tools`, `roles`, `sessions`, `architecture`,
    `plugin-rules`.
  - Acceptance: a parent spawns a child on any enabled Provider's model with
    any effort, with no role. A wrong Provider, model (where listed) or
    effort (where known) is a field-named tool error and creates nothing.
  - Hanafubuki: its integration goldens (the spec). No facade type changes.
- **O2 — the models in the guidance** (§2.5).
  - Files:
    - `agent/src/plugins/subagents/{prompt,models}.rs`;
    - `agent/src/session_driver/model.rs` (`spawn_model_catalog`).
  - Tests (`plugins/subagents/tests/models.rs`):
    - the order: the parent's Provider first, then by name; models in
      §2.5's order;
    - the bounds: 9 Providers and 9 models each give 8 and 8 with "and N
      more"; a 200-character id is left out; efforts with the default
      marked; long ids and many efforts stop the part at 4 KiB with its
      closing line;
    - a Provider without a listing gets its line;
    - two steps with nothing changed render byte-identical guidance; a
      listing recorded between them changes it once; a learned "no
      reasoning" turns that model's levels into "takes no reasoning
      effort", and its expiry turns them back;
    - no token, header or base URL in the request (the configured token
      string absent);
    - the guidance absent in Plan mode, for a child and for a role without
      `spawn_agent` (the existing test extended).
  - Goldens: the 11 carrying the guidance, regenerated on purpose.
  - Docs (§2.8, O2): `tools`, `roles`, `plugin-rules`.
  - Acceptance: the model sees each enabled Provider with its known models
    and levels, bounded, and the parent's prefix is stable across steps.
  - Hanafubuki: its integration goldens (the guidance).
- **O3 — a child's model where agents are listed** (§2.6).
  - Files:
    - `agent/src/plugins/subagents/{mod,service,tools}.rs`
      (`AgentSubagentStatus`, `ChildModelView`, `list_agents`);
    - `tui/src/commands/subagents/{mod,picker}.rs`.
  - Tests:
    - `list_agents` shows the three fields, and a child's model changed by
      `set_session_model` afterwards;
    - `subagents(parent)` carries them;
    - `crates/tui/src/commands/subagents/tests.rs`: the status cell and
      picker snapshots;
    - `crates/tui/src/app/tests/subagents.rs` holds.
  - Goldens: none (`list_agents`' output is a tool result, not a spec).
  - Docs (§2.8, O3): `embedding`, `terminal-interface`, `tools`.
  - Acceptance: `list_agents`, the facade and `/subagents` show each
    child's model and effort.
  - Hanafubuki: `AgentSubagentStatus.{provider_id, model,
    reasoning_effort}`.

## 4. Decisions

"Given" decisions came with the request. "To confirm" decisions are this
plan's; the recommendation is what the plan assumes until you answer.

| # | Status | Question | Recommendation |
| --- | --- | --- | --- |
| **D1** | Given | The arguments | Optional `model`, `reasoning_effort` and `provider` (an enabled Provider; default the parent's) on `spawn_agent`, with or without `agent_type` (§2.1). |
| **D2** | Given, flagged | Precedence | Per field, explicit > role > parent. An explicit model without an explicit effort takes the role's effort, else the parent's; narrowing at request time stays the net (§2.2). **Flagged**: Codex differs twice. Its role overrides the requested model, and an explicit model without an effort takes the model's default. We keep the given rule: the argument is the model's choice at this call; a role is a preset. Say if the model's default effort should win instead. With the known levels that is `Levels.default`, else the Provider's default (`None`). |
| **D3** | Given | Where the checks run | At the tool boundary, before the child exists. Each refusal is a field-named tool error, never a panic. A Provider must be enabled. A model is checked when a listing is known and passed through otherwise. An effort is parsed, then checked when levels are known and passed through otherwise (§2.3). |
| **D4** | Given | Wording | Neutral: what is available and the defaults; not Codex's "do not set unless asked" (§2.1, §2.5). |
| **D5** | Given | Gating | No allow-list and no switch (Q1). |
| **D6** | Given | Surfacing | `AgentSubagentStatus`, `list_agents`, spawn's result and `/subagents`. `exec --json` reports no sub-agent, so it needs nothing. Docs in both languages; `config-reference` only with Q4 (§2.6). |
| **D7** | To confirm | Where the list goes and what it shows | **The guidance, bounded**: 8 Providers × 8 models, 8 efforts each, and the whole Models part at most 4 KiB. Models in the order parent's, configured, policy's, then the listing by id, then "and N more". The tool description is fixed per registration and sits at the front of the cached prefix. Alternative: only configured models and the parent's, plus counts (shorter, less useful with no configured models). |
| **D8** | To confirm | What "known model" means | **Recorded listing ∪ `models."<id>"` ∪ the policy's model on the default Provider ∪ the parent's model**. No listing means pass-through. Never a live request at spawn. |
| **D9** | To confirm | Known levels | **E1's rule in one model-api accessor, `ModelMetadata::known_reasoning`, which E1's `effort_options` moves onto**: levels from a `Configured` or `Reported` fact; "no reasoning" from `Configured`, `Reported` or `Learned`; models.dev never. Validation then agrees with `/effort`'s options. The resolved fact lets a learned refusal win over reported levels, so a refusal changes the guidance once, and its expiry once more (§2.5). Avoiding that means reading the reported layer under the learned one: the second accessor E1 decided against. |
| **D10** | To confirm | Effort matching | **Exact**, as Codex. The ladder aliases (`light` → `low`) stay a request-time tolerance. The error lists the levels, so one retry fixes it. |
| **D11** | To confirm | Inherited values | **Checked only when the model chose them**: a role's model and effort, and the parent's effort, are not checked, as today. A role's unknown model still fails at the child's first request. |
| **D12** | To confirm | Naming a Provider | **Its id, else a unique case-insensitive display name**: ids are `provider-N`, opaque to the model. |
| **D13** | To confirm | One commit | **`apply_model_selection` takes the effort**, so a changed pair and its effort are one kernel commit (the facade passes the policy's, as now). Two commits today. |
| **D14** | To confirm | `[pluginOptions.subagents]` defaults (`defaultModel`, `defaultReasoningEffort`, Codex's `agents.default_subagent_*`) | **Not added**: the request is the model's free choice, and nothing asks for a person's default. Cost in Q4. |
| **D15** | To confirm | Overrides on `followup_task` | **None**: a child keeps its pair; changing it mid-life would compact or re-bill its history unasked. A person can still use `/model` on the child. |
| **D16** | To confirm | A failure after the child exists | **Unchanged**. Everything that can fail is checked before `create_child`. What remains is a Provider deleted in the moment between the check and the selection, or a persistence error, and it leaves an idle child under the task name, as today. A rollback would need a durable-child delete in the driver. |
| **D17** | To confirm | Another Provider with `fork_turns` `all` | **Allowed without an approval** (Q2). The person configured and enabled that Provider; `/model` already moves a conversation to any of them. |
| **D18** | To confirm | Spawn's result | **`provider`, `model`, `reasoning_effort` added**, the child's values after initialisation, so the model sees what it inherited. `wait_agent` unchanged. |
| **D19** | To confirm | The unknown-model error's ids | **At most 20, longest shared prefix first, then by id**, with the count of the rest. A few lines of code; better than the first 20 by id. |

## 5. Open questions

- **Q1 — gating and an allow-list.** Nothing gates roles' models today, and
  AGENTS.md forbids speculative controls, so none is added.
  - What each would cost:
    - a switch: one boolean option in `options.rs`, and since the spec is
      fixed per registration, a plugin restart on change, a branch in
      `spec` and in the guidance, its tests and the `config-reference` row:
      about 40 lines;
    - an allow-list (`allowedModels`, `provider/model` patterns): a list
      option, a filter in the catalog and the check, tests and docs: about
      60 lines.
  - Ask if a cost ceiling per Provider matters to you.
- **Q2 — the parent's conversation on another endpoint.** With the default
  `fork_turns` (`all`), a cross-Provider spawn sends the parent's history
  up to the current turn to `P`, without an approval. This is a direct
  consequence of D1, not an edge case.
  - Options:
    - (a) allow it (D17);
    - (b) refuse `provider` ≠ parent's with `fork_turns` `all` unless the
      Provider is the default one;
    - (c) a one-time approval per Provider and session.
  - (b) is a few lines and an error text; (c) needs an approval action and
    its presenter.
- **Q3 — cost.** The per-turn budget is the global policy's for every
  child (§1). A dearer model chosen by the model is not capped. A per-child
  budget would need a budget field on `spawn_agent` or the role.
- **Q4 — default sub-agent model and effort.** Two nullable `Text` options
  in `[pluginOptions.subagents]`, slotted between the role and the parent,
  checked by §2.3 at spawn, shown by the guidance, documented in
  `config-reference`: about 50 lines and a `cargo xtask
  check-config-reference` update. Not needed for [1].
- **Q5 — a role `provider` key.** A role's model stays on the parent's
  Provider unless `spawn_agent` names one. A `provider` key would make a
  role portable across Providers; it touches the role reader, the merge
  table and the role docs.
- **Q6 — the guidance's size.** The Models part costs at most 4 KiB
  (about 1 K tokens) in every request of a session that may spawn, and
  typically a few hundred bytes. Lower the cap if that is too much.

## 6. Not in this plan

- Live listings at spawn time (§2.3: recorded only).
- Changing a running child's pair from the parent (D15).
- Codex's service tiers (`apply_spawn_agent_service_tier`): the Agent has
  none.
- Codex's `model_catalog_in_context` variant and its `<model_catalog>`
  block: the guidance is our one place.
- Per-child budgets (Q3), defaults (Q4), role `provider` (Q5).
