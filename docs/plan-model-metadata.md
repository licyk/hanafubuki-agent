# Plan: model metadata from the Provider, with fallbacks

Date: 2026-11-11. Status: accepted. Every §9 decision was accepted as
recommended (2026-11-11); the milestones land one by one, and `todo.md` records
what each did and how it deviated from this plan.

The Agent should learn what the model a session samples with can do (its context
window, whether and how it reasons, whether it reads images) from the Provider
itself, instead of from settings the person fills in or defaults. It should then
act on what it knows:

- compact before the window fills;
- send only reasoning efforts the model takes;
- send images only to a model that reads them.

Where nothing is known, it should keep working as it does today, and learn from
the Provider's refusals.

This plan is a handoff to whoever implements it. Each milestone in §8 names its
files, contracts, tests and documents.

**Sources.**

- **Hoshimi** at `/root/code_workspace/Hoshimi`, commit `83f7813` (Python). Cited
  as `hoshimi/<path>:<line>`.
- **This repository** at `6e36de1`. Every Agent path below was read at that
  commit.

## 0. Terms

| Term | Meaning here |
| --- | --- |
| **route** | A model as the Agent reaches it: the protocol, the canonical endpoint and the model id. This is `ModelRouteKey` (`crates/model-api/src/image_input.rs:13`), which the image capability cache already keys by. |
| **metadata** | What is known about a route: context window, max output tokens, reasoning support and levels, image input. Each fact is optional ("unknown") and carries its **source**. |
| **source** | Where a fact came from. In precedence order: `configured` > `learned` > `reported` > `catalog` > `built-in`. See §4.2. |
| **listing** | A Provider's model list (`GET {base}/models`), which `ModelCatalogClient` already fetches (`crates/model-api/src/model_catalog.rs`). |
| **learned** | A fact the Agent inferred from a Provider's refusal: a smaller window, no image input, no reasoning parameter. |
| **models.dev** | The public model catalogue at `https://models.dev/api.json`, which Hoshimi uses as a fallback (`hoshimi/core/modelsdev.py:48`). |

## 1. Goal and non-goals

**Goal.**

1. One resolver gives each route its metadata from:
   - the person's configuration;
   - the Provider's listing;
   - what the Agent learned from refusals;
   - optionally, models.dev;
   - the built-in table.

   Each fact keeps its source.
2. Discovery runs by itself, in the background, and is cached on disk. It never
   delays a session's creation or a turn.
3. **Context window.** A known window drives proactive compaction (this exists).
   An unknown one is learned from the Provider's context-length error, and the
   reactive compaction that already exists keeps the turn going.
4. **Reasoning.** Known levels narrow the effort the Agent sends and the efforts
   the interface offers. A Provider that refuses the parameter gets one retry
   without it, which is remembered.
5. **Vision.** A model known to read images gets them. A model known not to
   never receives one: no failed round trip, and `view_image` is not offered. An
   unknown model gets them, as today. A refusal is remembered across restarts.
6. **Nothing known means today's behaviour, byte for byte.** This is an
   acceptance test (§7.1).

**Non-goals.**

- Hoshimi's per-lane catalogues for free and reseller stations, its probing
  (availability, not capabilities: `hoshimi/core/probe.py:78-146`), and its
  account rotation.
- Describing images with another model when the session's model cannot read
  them (Hoshimi's `playground.py:503-524`). This is noted for later in §10.
- Native Gemini (`models.get`) or Bedrock listings. The Agent has no protocol for
  them (`crates/model-api/src/protocol.rs:12-18`).

## 2. What exists today

About half of the feature is already there. The work is closing gaps, not a port.

| Capability | Today | Gap |
| --- | --- | --- |
| **Listing** | `ModelCatalogClient::list` fetches `GET {base}/models`: Bearer authentication for Responses and Chat, Anthropic headers and paging (`limit=100`, `after_id`, up to 10 pages) for Anthropic; 2 MiB and 1000 entries at most (`model_catalog.rs:20-24, 53-153`). It decodes `id`, `display_name`, Codex's `default_reasoning_level` and `supported_reasoning_levels[].effort`, and a window from the first of `max_input_tokens`, `context_length`, `max_model_len`, `context_window` (`model_catalog.rs:280-289`). | It runs only when the person opens the picker, `/model`, **Test**, or `doctor --online`. **One bad entry fails the whole listing** (`model_catalog.rs:80-99`): no id, an invalid id, or an effort it cannot parse. That breaks the decoding rule in AGENTS.md, though `internals.en.mdx` documents it as current behaviour. Image input and max output are not read. |
| **Where facts are kept** | `kernel::ModelCatalog` (`crates/kernel/src/model_catalog.rs:17-41`): per raw `base_url`, the windows the last listing reported, in memory. The rest of each listing is dropped. | Not on disk, so a restart forgets it. Keyed by the raw URL, not the route. Window only. |
| **Context window** | Resolved as configured, then reported, then built-in table, when a session is created (`service/session/create.rs:34-38`) or its model is selected (`configuration.rs:269-273`). The result is written into `AgentSessionConfig.context_window_tokens` (`model/session.rs:15`) and persisted. Recovery refuses a usage record whose window differs from the config's (`validation/history.rs:135-147`). Proactive compaction runs at `auto_compact_threshold_percent` (default 80) of a known window (`model/turn.rs:199-236`, `compaction/admission.rs:189-332`). | A session made before its window was known never gets one (`architecture.en.mdx:338`). The configured value and the resolved value are the same field, so neither can be refreshed later. The built-in table is small (`model_catalog.rs:46-66`). |
| **Context-length error** | Classified as `ModelError::ContextWindowExceeded` by code (`context_length_exceeded`, `context_window_exceeded`, `prompt_too_long`) or, for status 400, 413 or 422, by five English phrases (`model-api/src/image_input.rs:166-188`). The turn compacts once and retries; a second error ends the turn (`turn/loop_driver.rs:10-62`, `compaction/admission.rs:334-369`). | Nothing is learned: the next turn overflows again. The phrase list misses wordings Hoshimi matches, including Chinese ones (`hoshimi/core/failure.py:336-342`). |
| **Reasoning** | The effort is the policy's, or a role's. Syntax is checked (`foundation/src/agent_settings.rs:117-147`). It is dropped unless the protocol supports it, and only Responses does (`model-api/src/protocol.rs:87-95`, `validation/config.rs:314-317`). It is sent as `reasoning: {effort, summary: "auto"}` (`responses/encode.rs:44-46`). | The levels the listing reports are decoded but never used. A Provider that refuses the parameter fails the turn as a generic `ModelError::Remote`, with no retry. Chat Completions and Anthropic refuse a request that carries reasoning (`chat_completions/encode.rs:11-15`, `anthropic_messages/encode.rs:12-15`). Anthropic `max_tokens` is fixed at 16384 (`anthropic_messages/mod.rs:18`). |
| **Vision** | The policy is `Auto` or `Disabled` (`config/src/providers/mod.rs:99-105`). The observed capability is `Unknown` / `Supported` / `Unsupported` (`model-api/src/model/input.rs:47-62`). A Provider refusal is classified (`image_input.rs:190-234`), then the turn retries once with images replaced by placeholders and records `Unsupported` (`sampling/request.rs:281-345`, `:374-397`). Gates exist at turn start, at steering, in the tool plan (`view_image`) and at request build. | `Supported` is never produced. The observed cache (`service/image_capability.rs`) is in memory only, with a 6 h TTL, so every restart pays the failed round trip again. |
| **Max output tokens** | Not configurable. Anthropic sends 16384; the other protocols send none. | No fact, no setting. |
| **Host** | `AgentHostPorts` has no model-metadata port (`crates/agent/src/service/mod.rs:96-111`). | None needed if discovery runs inside the Agent (§4.8). |

## 3. What Hoshimi does, and what to take from it

Hoshimi builds each model's spec per provider lane. It has no single resolver:
each lane's `specs()` layers its own sources. The capability-driven behaviour lives
only in its Playground (`hoshimi/api/playground.py`).

**Take**

- **Field names per listing shape.** These are cited from Hoshimi and need a
  recorded fixture before they are relied on (§7.3).

  | Shape | Fields | Hoshimi |
  | --- | --- | --- |
  | Generic OpenAI-compatible and OpenRouter | **Window:** `context_length`, `context_window`, `contextWindow`, then `top_provider.context_length`. **Max output:** `max_output_tokens`, `max_completion_tokens`, `maxTokens`, `top_provider.max_completion_tokens`. **Vision:** `architecture.input_modalities`, `input_modalities` or `input` contains `"image"`, or a boolean `vision` / `capabilities.vision`. **Reasoning:** `supported_parameters` contains `reasoning` or `reasoning_effort`, or a boolean `reasoning` / `capabilities.reasoning`. **Tools:** `tools` / `capabilities.tools` booleans. | `hoshimi/core/providers/keyed.py:33-69`, `hoshimi/core/catalog.py:226-269`, `hoshimi/core/providers/station_catalogs.py:114-133` |
  | Ollama, from `/api/tags` then `POST /api/show {"model"}` | `capabilities[]` containing `vision`, `thinking`, `tools`; the largest `model_info["*.context_length"]` | `hoshimi/core/providers/local.py:78-107` |
  | LM Studio, from `GET /api/v0/models` | `loaded_context_length` when `state == "loaded"` | `hoshimi/core/providers/local.py:52-76` |

  **Unverified:** whether Anthropic's `/v1/models` reports a window or
  capabilities. Hoshimi reads nothing special there. Our decoder already tries
  `max_input_tokens` first. This needs a recorded response.
- **Wordings that mean the context overflowed**, behind the same status gate we
  already apply. They include `context_length_exceeded`, `maximum context`,
  `exceed(s|ed) the context limit`, `input is too long`, `too many (input)
  tokens`, `token limit exceeded`, `exceeds the maximum number of tokens`,
  `request too large`, `maximum (input|prompt) length`, and the Chinese
  `上下文长度`, `上下文窗口`, `超出最大长度`, `超过最大长度`, `输入过长`,
  `token 数量超过` (`hoshimi/core/failure.py:336-342`).
- **Reading the limit out of the error** (`hoshimi/core/failure.py:354-383`). It
  matches `maximum context length is N`, `context (window|length|limit) (of|is) N`,
  `> N maximum`, and `maximum (number of tokens allowed|input tokens) (is|(|:) N`.
  A value is accepted only within 1024..=20,000,000, and the smallest value seen
  is kept (`hoshimi/core/agent/context_window.py:98-118`).
- **Output headroom** in the compaction limit:
  `limit = min(window × threshold, window − min(max_output, max(4096, window/10), window/2))`
  (`hoshimi/core/agent/context_window.py:81-95`). The Agent compacts on
  `window × percent` alone, which ignores the room the answer needs (D4).
- **Stale-while-revalidate disk cache.**
  - The cache is restored at start and refreshed at once in the background. A
    failed refresh keeps the previous data (`hoshimi/core/net/catalog_cache.py:24-68`,
    `hoshimi/core/discovery.py:39-87`).
  - **Shrink guard:** a refresh that loses more than half the ids is refused once
    (`hoshimi/core/providers/base.py:373-419`).
- **models.dev, used as an add-only source.** It may add a "yes" but never turns
  a "yes" into a "no" (`hoshimi/core/modelsdev.py:204-227`). It needs a real
  User-Agent and fails silently (`hoshimi/core/modelsdev.py:48-52, 145-157`).

**Don't take**

- **"Unknown" stored as "no".** `vision` and `reasoning` are booleans that
  default to `False`, and limits default to a fixed 131072 / 32768
  (`hoshimi/core/catalog.py:23-54`). Here every fact is an `Option` and carries
  its source.
- **About 15 per-lane builders whose row readers disagree.** For example,
  `spec_from_row` does not read `top_provider.context_length` and `kilo_spec`
  does. Here there is one row decoder.
- **models.dev matched by exact id, first provider wins**
  (`hoshimi/core/modelsdev.py:114-120, 192-196`). A models.dev `reasoning: true`
  also rewrites the wire (`hoshimi/core/effort.py:320-327`). A third-party flag
  must never change a request on its own (§4.6).
- **Forgetting refusals.** Learned windows live in memory only, keyed by the
  requested string. Image and tools refusals are not remembered at all, and there
  is no recovery when a reasoning parameter is refused (`hoshimi/core/agent/context_window.py:98-118`,
  `hoshimi/api/playground.py:667-671`).
- **Reactive compaction without the status gate**
  (`hoshimi/api/playground.py:135-137`). Ours keeps the typed error.

## 4. Design

### 4.1 Data model (`hanafubuki-agent-model-api`)

```rust
/// What is known about a route; every field may be unknown.
pub struct ModelMetadata {
    pub context_window: Option<Sourced<u64>>,
    pub max_output_tokens: Option<Sourced<u64>>,
    pub reasoning: Option<Sourced<ModelReasoningSupport>>,
    pub image_input: Option<Sourced<bool>>,
}

pub enum ModelReasoningSupport {
    /// It reasons, at these efforts (empty: levels not reported), with this default.
    Supported { efforts: Vec<ModelReasoningEffort>, default: Option<ModelReasoningEffort> },
    /// It takes no reasoning parameter.
    Unsupported,
}

pub struct Sourced<T> { pub value: T, pub source: ModelMetadataSource, pub observed_at_ms: u64 }

pub enum ModelMetadataSource { Configured, Learned, Reported, Catalog, BuiltIn }
```

- `ModelCatalogEntry` gains `max_output_tokens`, `image_input: Option<bool>` and
  `reasoning: Option<bool>`. A row that reports levels implies `true`.
- The tool-calling flag is not modelled. Every protocol the Agent speaks sends
  tools, and a Provider that refuses them already fails clearly. Hoshimi's tools
  gate (`hoshimi/api/playground.py:553-559`) is left for later.

### 4.2 Resolution and precedence

The resolver is one function, kernel-side, replacing `ModelCatalog::context_window`.
It is applied **per fact**, not per source:

1. **Configured** always wins: the global policy today, and per-model overrides
   if D3 is accepted.
2. **Learned** wins next, but only in the safe direction:
   - a learned window is used only while it is *smaller* than the reported,
     catalog or built-in window;
   - a learned `image_input = false` or reasoning `Unsupported` beats a reported
     "yes". The Provider's refusal is evidence; its listing is a claim.
   - Learned facts expire after 7 days, so a Provider that fixes its model is
     tried again.
3. **Reported**, from the route's own listing.
4. **Catalog**, from models.dev, only if D2 enables it. It is add-only: it fills
   an unknown fact and never contradicts a reported one.
5. **Built-in**: today's window table, kept as is.

Otherwise the fact is unknown.

### 4.3 Where it lives

| Crate | Change |
| --- | --- |
| `model-api` | **Listing:** the `ModelMetadata` types; tolerant decoding of `ModelCatalogEntry`, so a bad entry is skipped and counted, and the listing fails only when it has no `data` array; the new fields per §3's table. **Errors:** `ModelError::UnsupportedReasoning`, classified like the image refusal (D5); the widened context-overflow classifier; `context_limit_in(message) -> Option<u64>`. **models.dev:** optionally, a fetcher in its own module (D2). |
| `kernel` | **Registry:** `ModelCatalog` becomes `ModelMetadataRegistry`, keyed by `ModelRouteKey`, which removes the raw-URL keying. It holds reported, learned and catalog facts and resolves per §4.2. `ModelImageCapabilityCache` (`service/image_capability.rs`) is folded into it as the learned `image_input` fact. **Ports:** a `ModelMetadataStore` port, so the facade persists it without the kernel touching the data directory. **Sessions:** applies metadata to sessions (§4.4). **Learning:** learns from the refusals in §4.5–§4.7. |
| `agent` (facade) | Background discovery (§4.4). The store: a new file `model-metadata.json` in `AgentStoragePaths` (`crates/agent/src/storage_paths.rs:11-58`), decoded tolerantly and quarantined if unreadable, as the other state files are. The configuration mapping for D2, D3 and the discovery switch. New fields on `AgentModelInfo`, or a new snapshot type for the interface. |
| `config` | `modelMetadata.discovery = "auto" \| "off"` (default `auto`). `modelMetadata.modelsDev` (D2). Per-model overrides (D3). |
| `tui`, `cli` | Show what is known and from where: the model picker, `/status`, the `/providers` form's placeholders (`policy.rs:145-153`), the effort choices, and `doctor --online` (§4.9). |

`crates/foundation` changes only if D6 needs a new effort value. `agent_settings`
belongs to the Agent alone, not to the modules shared with Hanafubuki
(`contributing.en.mdx`, "Twins"), so that change is allowed. The shared
twins are untouched.

### 4.4 Discovery and how sessions take it

**When discovery runs**

- At facade start, for the default Provider, cache first.
- When a session's Provider or model is selected.
- On every listing the person already triggers: picker, Test, `/model`,
  `doctor`. Each now records its whole entries, not just windows.

Each Provider is listed at most once per 24 h after a success, and once per
15 min after a failure. Discovery runs on a facade background worker and goes
through the Provider's own client, with the same proxy and headers as its
requests. `discovery = "off"` stops every automatic listing.

**Merging a listing**

A new listing is merged per model, not swapped in whole:
- a model missing from the new listing keeps its old facts until they expire;
- a listing that loses more than half its ids is not trusted until it repeats
  (Hoshimi's shrink guard).

**How a session takes a fact**

Sessions never wait for discovery. A session is created with whatever is known
then. At **each turn start**, `prepare_sampling_step` re-resolves the session's
route (cheap: a map lookup). If the resolved window or image fact differs from the
session's, it applies the new value:

- **Window.** A session keeps the effective window in `context_window_tokens`, as
  today, so that recovery's check holds. It gains `context_window_source`
  (`#[serde(default, skip_serializing_if = "Option::is_none")]`), so a session
  written before stays byte-identical. A change rewrites the usage window in the
  same critical section, as model selection already does
  (`configuration.rs:282-285`). A session whose source is `Configured` is never
  rewritten. A session written before this change, with no source, counts as
  configured if it has a window, and as unknown if not. This fixes the known
  limit "a session made before its window was known keeps none"
  (`architecture.en.mdx:338`).
- **Image input.** The image fact sets
  `effective_input_capabilities.image_input` through the existing
  `ModelInputCapabilityPolicy::effective`. `Supported` is produced for the first
  time. The person's `Disabled` policy still wins.
- **Reasoning.** Reasoning is resolved per request (§4.7), not stored.

### 4.5 Context window

| Known | Proactive | Reactive |
| --- | --- | --- |
| **window known** (any source) | As today: compact at `percent × window`. With D4, also leave `min(max_output, max(4096, window/10), window/2)` of headroom. | As today: compact once, retry once. **New:** if the error names a smaller limit, learn it. |
| **window unknown** | None, as today. Budget pressure and `new_context` still compact (`compaction/admission.rs:13-46, 267-282`). | Compact once and retry, as today. **New:** learn the limit from the error (`context_limit_in`, 1024..=20M). If the error names none, learn the failed request's own estimated input size (`sampling/estimate.rs`) as an upper bound, marked `Learned`. From the next step on, the session compacts proactively at `percent ×` that bound. |

**The classifier.** It adds Hoshimi's wordings (§3) to the five phrases, keeps
the status gate (400, 413, 422, or an in-band error with no status), and keeps
the "before any output" rule (`sampling/stream/events.rs:31-35`). A 429 or a
quota error never counts, whatever its text says.

**After a learned window.** A learned window is persisted per route. The session
that learned it takes it at once, through the same rewrite as §4.4, so the retry
compacts against it. Every later session on that route starts with it.

### 4.6 Vision

| `image_input` | Behaviour |
| --- | --- |
| unknown | As today. Images are sent, `view_image` is offered, and a refusal falls back once and is learned. |
| `true` (reported, catalog or configured) | As unknown, with `Supported` recorded. |
| `false` (reported, learned or configured) | Images are replaced by placeholders before the first request, with no failed round trip. `view_image` is not planned (`plugins/view_image.rs:89-94`). Turn start and steering refuse a host's image with the existing message (`turn/state.rs:173-177`). |

- **Learning.** The learned `false` from the existing fallback
  (`sampling/request.rs:374-397`) now persists in `model-metadata.json`, for
  7 days instead of 6 h in memory.
- **Catalog.** A catalog `true` never overrides a learned `false`. A catalog
  `false` is ignored, since catalogs are add-only.

### 4.7 Reasoning

The effort sent on a request is resolved from the session's effort, the protocol,
and the route's reasoning fact:

| Reasoning fact | Effort sent |
| --- | --- |
| unknown | The configured effort, as today. A refusal is learned. |
| `Supported`, levels known | The configured effort if listed. Otherwise the nearest listed level, using Hoshimi's aliases (`minimal→low`, `xhigh→max`; `hoshimi/core/effort.py:294-328`), with one warning per session. With no configured effort: none, so the Provider's default applies. |
| `Supported`, levels not reported | The configured effort, as today. |
| `Unsupported` | None, whatever is configured. The status shows it was dropped. |

- **The refusal fallback** mirrors the image one (`sampling/request.rs:281-345`):
  - a new `ModelError::UnsupportedReasoning` (D5) leads to one retry without the
    reasoning field;
  - a `ModelRetryScheduled { stage: ReasoningFallback }` event;
  - a learned `Unsupported`.
- **Other protocols.** Responses is the only protocol that sends reasoning today.
  Extending it to Chat Completions (`reasoning_effort`) and Anthropic (`thinking`)
  is D1. Without D1, a reasoning fact on those protocols only shapes what the
  interface offers.
- **The interface.** The effort choices in `/model`, `/providers` and `/config`
  list the route's levels when they are known, and stay free text otherwise.

### 4.8 The host

No new port. Discovery happens inside the Agent, on the Provider client the host
already configures.

At its next pin, Hanafubuki:

- takes the new `AgentSessionConfig.context_window_source`. It is skipped when
  `None`, so its history corpus is unchanged.
- takes the new fields on `AgentModelInfo`, or the new metadata snapshot.
- gains `model-metadata.json` in its Agent data directory.
- may set `modelMetadata.discovery = "off"`, or leave models.dev off (D2).

### 4.9 Showing it

- **Model picker** (`tui/src/commands/providers/screen/models.rs:39-88`): the
  window (`200K`), and `vision` / `reasoning` badges, for entries that report
  them.
- **`/status`:** `Context window 200K (reported)`. The source is one of
  configured, reported, learned, models.dev or built-in, or the window is
  `unknown (compacts on the Provider's error)`.
- **`/providers` form:** the placeholders name the discovered value
  (`200K, reported`) in place of `catalog or built-in default`.
- **`doctor --online`:** each model's facts.
- **A learned fact** is written once into the conversation as a notice. For
  example: `The Provider refused 210000 tokens; this model's window is taken as 200000 from now on.`

## 5. Fallback matrix

| Situation | Behaviour |
| --- | --- |
| Discovery off, or the listing fails or times out | The cached facts, then built-in. Unknown otherwise. Nothing blocks. |
| Listing has an odd entry | That entry is skipped and counted. The others are used. |
| Unknown window, request too long | Compact, retry, learn a limit (named, or an upper bound). |
| Known window too large (Provider lowered it) | The same, and the learned smaller window wins. |
| Image refused by a model reported as reading images | Fall back once, learn `false`, and stop sending images for 7 days. |
| Reasoning parameter refused | Retry without it, learn `Unsupported`, and stop sending it for 7 days. |
| `model-metadata.json` unreadable | Quarantined (as the other state files are), and the run starts empty. |
| models.dev unreachable (D2) | Silent. It is a fallback source. |

## 6. Configuration

```toml
[modelMetadata]
discovery = "auto"   # "off": no automatic listing; the person's listings still record
modelsDev = false    # D2
```

Per-model overrides are D3. The global policy keeps its keys:
`contextWindowTokens` stays the *configured* window, and `imageInput = "disabled"`
still wins over every fact.

## 7. Tests

### 7.1 The invariant: nothing known means nothing changed

- With `discovery = "off"` (the facade's `test_support` default) and no facts,
  every request golden in `crates/agent/src/tests/golden/` is byte-identical, and
  the fake model server sees exactly the requests it sees today.
- The server pops one scripted response per connection
  (`test_support/model_server.rs:36-64`) and the goldens assert one request
  (`golden.rs:763`). An unplanned `GET /models` would therefore consume a turn's
  response. That makes the invariant a real check.
- The history corpora are untouched.

### 7.2 Behaviour

| Area | Tests |
| --- | --- |
| model-api | Tolerant listing: a bad entry is skipped, a bad page fails. Each listing shape from §3, from recorded fixtures. The overflow classifier: every wording, the status gate, 429 never. `context_limit_in`: each pattern, and the bounds. The reasoning refusal classifier. |
| kernel | Precedence per fact, including that learned only lowers a window, that catalog is add-only, and that facts expire. A session takes a late window at turn start, rewriting its usage. Recovery still holds after the rewrite. A configured window is never rewritten. Unknown window: an error with a limit compacts, retries and learns it, and the next step compacts proactively. Image `false` known: no failed round trip, and `view_image` is absent. Reasoning refused: one retry, then learned. Effort narrowing. |
| facade | Discovery runs once per Provider per period and backs off after failures. The store's round trip and quarantine. Merging with the shrink guard. Discovery never delays `create_session`. A test against a fake server that answers `GET /models` apart from the turn's responses: a small change to `model_server.rs` routes by path. |
| tui | The picker badges, the `/status` line and the form placeholders, as insta snapshots. |

### 7.3 Fixtures

The fixtures are real responses, recorded once and trimmed (ids, a few
fields), under `crates/model-api/tests/fixtures/listings/`:

- OpenAI;
- OpenRouter;
- Anthropic;
- Ollama `/api/show`;
- LM Studio `/api/v0/models`.

Any field name taken from Hoshimi and not confirmed by a fixture is left out
rather than guessed.

## 8. Milestones

Each milestone is one commit that passes the gates on its own. It updates the
pages it changes in both languages, with the same `[#id]`s, and adds a
`todo.md` entry.

**M1 — Tolerant, richer listing.** `model-api/src/model_catalog.rs`:
- per-entry tolerance;
- the §3 fields;
- `ModelMetadata` and its types.

The 394-line file is split along its parts, for example a `listing/shapes.rs`.
Docs: `internals` (the listing is no longer strict), `providers-models`.

**M2 — The registry and the store.**
- `kernel::ModelMetadataRegistry` replaces `ModelCatalog` and folds in
  `ModelImageCapabilityCache`.
- The `ModelMetadataStore` port, and the facade's `model-metadata.json`.
- `record_listing` keeps whole entries.

Docs: `data-directory`, `architecture` (context window, model API).

**M3 — Discovery and late resolution.**
- The facade's background discovery, with `modelMetadata.discovery`.
- `context_window_source`, and re-resolution at turn start.
- The §7.1 invariant test.

Docs: `config-reference`, `architecture` (removes the known limit at `:338`).

**M4 — Context.**
- The widened classifier and `context_limit_in`.
- The learned window, persisted, applied at once, and the notice.
- D4's headroom if accepted.

Docs: `architecture` (compaction), `internals` (provider errors).

**M5 — Vision.**
- The image fact seeds `effective_input_capabilities`.
- Known `false` skips the round trip and `view_image`.
- The learned `false` is persisted.

Docs: `providers-models`, `architecture`.

**M6 — Reasoning.**
- `UnsupportedReasoning` and its fallback.
- Effort narrowing to known levels.
- The interface's effort choices.
- D1's encoders if accepted. `encode_chat_request` and
  `encode_anthropic_request` are already over the function limit, at ceilings
  227 and 216 (`architecture/module-size-policy.toml:26-36`), so the reasoning
  encoding goes into its own helper rather than raising a ceiling.

Docs: `providers-models` (`:109`, `:117`), `architecture` (thinking).

**M7 — Showing it, and overrides.** §4.9, and D3's overrides. Docs:
`terminal-interface`, `cli-reference` (`doctor`), `config-reference`.

**M8 — models.dev (optional, D2).** The fetcher:
- a real User-Agent;
- a 24 h disk cache in `model-metadata.json`;
- add-only, behind reported and learned facts.

It matches ids by lowercasing, stripping a `vendor/` prefix and stripping
`-YYYYMMDD` and `-latest`, and prefers the entry under the Provider's own name.
It takes `limit.context`, `limit.output`, `reasoning` and
`modalities.input ∋ "image"`.

## 9. Decisions

| # | Question | Recommendation |
| --- | --- | --- |
| **D1** | Send reasoning on Chat Completions (`reasoning_effort`) and Anthropic (`thinking`, with a budget or adaptive), not only Responses? | **Yes, in M6.** Without it, reasoning awareness means one protocol. Anthropic also needs `max_tokens` to exceed the thinking budget, which ties into the max-output fact. |
| **D2** | Use models.dev? | **Optional, off by default, and last (M8).** It is a call to a third party from every machine, and from an embedding host. The request asks for metadata from the Provider's API, and the built-in table and learning cover the rest. |
| **D3** | Per-model overrides (`providers[].models."<id>" = { contextWindowTokens, maxOutputTokens, imageInput, reasoningEfforts }`) beside the global policy? | **Yes, in M7.** The global policy cannot describe two models of one Provider. Overrides are the `configured` source. |
| **D4** | Leave output headroom in the proactive limit, as Hoshimi does? | **Yes, in M4.** It changes when existing sessions compact, slightly earlier on a small window, so it is a decision, not a fix. The compaction goldens fix the window at 100000, so their limits would move: they are regenerated deliberately. |
| **D5** | How to recognise a reasoning refusal? | **Narrowly.** Status 400 or 422, with `unsupported_parameter` / `unsupported_value` naming `reasoning` or `reasoning_effort`, or a message that names one of those parameters and says unsupported, unknown or not allowed. A broad match would retry and mislearn on unrelated 400s. |
| **D6** | Effort levels outside today's set (`none`, `max`)? | **Take what the Provider lists through `ModelReasoningEffort::Custom`, and keep `none` refused.** Use `Unsupported` rather than an effort `none` to send no reasoning. No change to `agent_settings`. |
| **D7** | Ollama's `/api/show` and LM Studio's `/api/v0/models` when the Provider is local? | **Later.** They are not the protocol's endpoint, and need a guess at the server kind from its base URL. A local model's window is better set by a D3 override until then. |

## 10. Later

- Describe images with a vision model for a session whose model cannot read them
  (Hoshimi's `hoshimi/api/playground.py:503-524`).
- A tool-calling fact, and a session without tools for a model that refuses them.
- Raising `max_tokens` on Chat Completions from the max-output fact.
