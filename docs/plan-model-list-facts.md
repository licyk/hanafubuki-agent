# Plan: the model list's facts and its loading state

Repositories, read from source on 2026-11-14:

- The Agent: `/root/code_workspace/hanafubuki-agent`, main `f1d5808`. Every
  `file:line` below is at that commit. The files the shortcuts plan's
  M6–M8 list (its §3: `tui/src/mermaid/`, `markdown_render/mermaid.rs`,
  the execution service, `contracts/src/tools/display.rs`,
  `history_cell/tool/`, `diff_render/`) are none of those cited here; O1–O3
  land after this plan. A milestone still re-reads the symbol named beside
  each reference.
- Codex: `/root/code_workspace/codex`, `57a808a392`, for its loading view
  (`codex-rs/tui/src/app/provider.rs:90-113`: a static "Loading model
  providers…" title over a Back item; closing it drops the request).
- Hanafubuki (`/root/code_workspace/Hanafubuki-dev`), only to measure what a
  facade change costs it.

The user agreed to the decisions of §2 on 2026-11-14 and decided the
questions of §5, each as recommended, and the milestones' place in the
queue: after M8, before O1 (§6).

The request has three parts. Numbers in brackets refer to them throughout:

- [1] **Facts per model.** Each entry of the model list shows, besides its
  Provider and model id, its **context window**, whether it supports
  **thinking**, and whether it supports **vision** (image input).
- [2] **A loading state.** While the models are fetched (a listing can take
  seconds per Provider), the list gives clear, live feedback instead of
  showing nothing.
- [3] **Fit.** Find the components and data sources involved, keep the
  facts consistent wherever the Agent shows them, and fit the work into the
  queue.

**Scope.** `/model`, the Provider page's **Choose the model**, the settings
center's **List models** busy notice, `doctor --online`'s model lines, the
model policy form's window summary, and the facade's
`list_provider_models`. **Non-goals:** a `models` CLI subcommand, a cached
model list shown before the live one (§5 Q7), new capability sources (the
built-in table stays window-only, §5 Q9), and any model-visible change.

The `todo.md` section "Plan: the model list's facts and its loading state"
tracks the milestones L1–L3 (§6). A path this plan names that does not
exist yet is a file its milestone creates; it is marked "(new)".

## 0. Terms

- **Listing**: one Provider's `GET …/models`, through the facade's
  `list_provider_models`. Always a live request; only its facts are kept.
- **Resolved facts**: a `ModelMetadata` for a Provider and model id, each
  fact with its source, in the precedence configured > learned (only in the
  safe direction) > reported > models.dev (only adds a "yes") > built-in
  (window only) (`crates/kernel/src/model_metadata/resolve.rs:15-56`).
- **Three states**: a capability is **yes**, **no** or **not known**; a
  window is a size or **not known**.
- **Pinned row**: the session's own model, first in `/model`'s list.
- **Section**: one Provider's rows in `/model`: pending, listed or failed.

## 1. The request against the code

| Premise | What the code says | Consequence |
| --- | --- | --- |
| `/model` lists the models | `/model` sends `AgentCommand::ListModels` (`crates/tui/src/commands/model.rs:23-31`); the bridge calls `list_provider_models` for every enabled Provider and replies **once**, with every catalog (`crates/tui/src/agent_bridge/commands.rs:105-120`); only then does `show_model_choices` open a view (`crates/tui/src/chatwidget/commands.rs:164-166`). The facade waits for **all** Providers in a `JoinSet` and sorts them (`crates/agent/src/service/provider/registry.rs:96-129`). | **Nothing is shown while the slowest Provider answers**: no view, no indicator. A listing retries per the Provider's policy, 3 attempts with a 30 s establishment timeout each by default (`crates/model-api/src/provider.rs:183,220`), so one dead Provider hides every other for up to about a minute and a half. The view must open at once and fill per Provider (§3.4). |
| The rows show a window, `vision`, `reasoning` | `entry_badges` (`crates/tui/src/model_facts.rs:28-37`) reads only the listing's own entry: the window, `vision` when `image_input == Some(true)`, `reasoning` when `reasoning == Some(true)`. A "no" and a "not known" both show nothing. | [1] asks whether a model supports each: "no" and "not known" must differ (D3). |
| The listing is where the facts come from | It is one source of five. OpenAI's own `/v1/models` and OpenAI-compatible local servers report an id and little else (the fields read are in `crates/model-api/src/model_catalog/shapes.rs:25-64`), so today `gpt-5.1` on OpenAI shows no badge, although the built-in table gives it 272K (`crates/kernel/src/model_metadata/built_in.rs:7-26`) and a model's own settings or models.dev may say more. | Rows show the **resolved** facts (D1). |
| Resolution is cheap and synchronous | It is: `ModelMetadataRegistry::resolve` (`crates/kernel/src/model_metadata/mod.rs:238-258`) locks once and reads memory. The facade's `model_metadata` is async only because it reads `providers.toml` first (`registry.rs:177-194`). | Facts can be resolved inside the listing call for every model, with one settings read (D2). |
| Resolve after the listing is recorded | Not reliable. The shrink guard does not record a listing with fewer than half the previous models (`crates/kernel/src/model_metadata/state.rs:170-179`), and the registry keeps at most 4 096 routes (`state.rs:27`, `bound` at `state.rs:257-283`) while one listing has up to 1 000 entries (`crates/model-api/src/model_catalog/mod.rs:27`): several large listings at once can evict a just-listed model's reported facts. | The listing's **own entry is the reported layer** of the resolution, with the stored learned facts, the model's settings, models.dev and the built-in window (D2). |
| The same answer as `/status` | Not exactly. A session folds in the policy's `contextWindowTokens` and `imageInput = "disabled"` (`crates/kernel/src/service/model_facts.rs:59-68,159-174`); the facade's `model_metadata` passes no policy window (`registry.rs:191`) and no policy image switch. | Rows show the **model's** facts, as `model_metadata` and `doctor --online` do; the policy's overrides are said once in the subtitle (D9). |
| Many models make resolution slow | `doctor --online` calls `model_metadata` per model (`crates/cli/src/doctor.rs:253-258`), each of which reads `providers.toml`; `list_provider_models` reads it once (`registry.rs:71`) and then once more per Provider in `resolve_provider` (`registry.rs:105`, `crates/agent/src/service/provider/resolution.rs:27-33`). | One read per call: the Provider is resolved from the settings already read, and its models' facts with it. `doctor` uses the listing's facts and makes no per-model call (L1). |
| The facts have one vocabulary | Three copies of the source names: `source_name` (`crates/tui/src/model_facts.rs:15-23`), `sourced` (`crates/cli/src/doctor.rs:357-366`), and the model policy form, which prints the serde name (`name_of`, `crates/agent/src/service/settings/model_policy.rs:287`, `crates/agent/src/service/settings/mod.rs:408`): `built_in` and `catalog` where the rest say `built-in` and `models.dev`. `doctor` says `images`/`reasoning`, the TUI `vision`/`reasoning`. | One label per source and per capability, in model-api, which every caller already reaches through the facade's re-exports (D10). |
| A loading indicator to reuse | `spinner(start, animations_enabled)` (`crates/tui/src/shimmer.rs:62-74`: `•`/`◦` every 600 ms, a dim `•` when not animated) and `fmt_elapsed_compact` (`crates/tui/src/status_indicator_widget.rs:49`), which the status indicator animates by scheduling its next frame on render (`status_indicator_widget.rs:243-258`). The Provider page's `Listing` stage is static text, "Listing the models… esc closes" (`crates/tui/src/commands/providers/models.rs:141-153`); the hooks browser says "Reading the hooks…" (`crates/tui/src/commands/hooks/browser.rs:331`). | A shared loading line (§3.4), animated like the status indicator. |
| The animations setting | **There is none.** The status indicator is built with `true` (`crates/tui/src/bottom_pane/mod.rs:526-534`), a running tool's bullet too (`crates/tui/src/history_cell/tool/mod.rs:437`). | The loading line takes the same constructor flag, `true` in production and `false` in snapshot tests; no setting is added. |
| A selected-row detail through `selected_description` | `selected_description` **replaces** the row's description when it is selected (`crates/tui/src/bottom_pane/list_selection_view.rs:216-219`), so the selected row would lose its own facts. | The detail line is rendered by `/model`'s own view under the list, from `selected_index()` (`list_selection_view.rs:280-284`). |
| Enter on a pending row; retry on a failed one | A disabled row is skipped by the cursor (`skip_disabled_down/up`, `list_selection_view.rs:294-330`) and does nothing on Enter; today's failed Provider is a disabled row (`commands/model.rs:50-57`). | Pending rows stay disabled. A failed row becomes selectable, and Enter lists that Provider again (D8). |
| Updating a list in place | `ListSelectionView` has no item replacement; the sub-agents picker rebuilds the whole view keeping the selection index (`crates/tui/src/commands/subagents/picker.rs:88-95`), which loses a search query. Selection is kept by index (`apply_filter`, `list_selection_view.rs:137-187`). | `replace_items` keeping the query, the selection (by the caller's index) and the selected row's place on screen (§3.4). |
| Settings center's **List models** | Runs through `send` (`crates/tui/src/settings_center/mod.rs:408,444-454`), whose busy notice says **"Saving…"** (`crates/tui/src/settings_center/render.rs:299`). It reports a count and up to 10 ids, no facts (`crates/agent/src/service/settings/providers.rs:260-292`). | The notice names the action (L3). Facts stay in **Choose the model**: List models is the Provider's test. |
| `/model` during a turn | Refused before it runs: "'/model' is disabled while a task is in progress." (`crates/tui/src/chatwidget/commands.rs:36-40`, pinned at `crates/tui/src/chatwidget/tests/mod.rs:161`). | No turn starts from the composer while the list is open; only background work can learn a fact meanwhile (§4). |
| Search | A case-insensitive substring of `search_value` (`list_selection_view.rs:149-161`); `/model` sets it to the model id (`commands/model.rs:90`). With search on, the current item is not selected by itself (`list_selection_view.rs:142-146`), and `/model` passes no initial index, so today the cursor starts on the first row. | Search also matches the display name and the Provider's name (D11); the pinned row starts selected (D6). |
| A model in no listing | Inserted first, described by its Provider's **id** (`commands/model.rs:41-48`, `choices.provider_id.clone()` at 45). | The pinned row names the Provider's display name, and has facts at once (§3.4). |
| Hosts read the result | Hanafubuki passes `Vec<AgentProviderModelCatalogResult>` through a Tauri command unchanged (`Hanafubuki-dev/src-tauri/src/agent/commands/providers.rs:11-22`) and its TypeScript reads the entry's flat fields (`Hanafubuki-dev/src/types/agent/provider.ts:87-100`). | The entry is flattened into the JSON beside a new `metadata` field: additive for its frontend, a pass-through for its Rust (§3.8). |
| Tests | No snapshot of `/model` or the Provider page; content checks in `crates/tui/src/chatwidget/tests/mod.rs:216-260` and `crates/tui/src/app/tests/settings.rs:277-318` (which pins `"200K · vision"` and `"Unset, the model's: 200000 (reported)"`); `model_facts/tests.rs` pins the badges. The fake server can hold a listing (`FakeListing::held`, `crates/agent/src/test_support/model_server.rs:66-72`). | L2 adds snapshots at three widths and an end-to-end test with a held listing. |

**Corrections to the direction this plan was given.**

1. *"Resolved after the listing has been recorded"*: the store can lack a
   just-listed entry (the shrink guard, the 4 096-route bound). The entry
   itself is the reported layer instead (D2).
2. *"The same answer `/status` uses"*: `/status` is a session's, with the
   policy's window and image switch folded in. The rows show the model's
   facts and say the policy's overrides once (D9).
3. *"Respects the animations setting"*: no such setting exists (§1).
4. *"`selected_description` for the detail line"*: it would replace the
   selected row's own facts; the view renders the detail itself.

## 2. Decisions

Each gives what is decided, why, and the alternative rejected. The user
agreed to every one on 2026-11-14, those that were also questions (§5) as
recommended.

- **D1 — Rows show resolved facts.** The window, thinking and vision of a
  row are the model's resolved facts: its own settings (`models."<id>"`),
  what its refusals taught, what this listing reports, models.dev when it is
  on, the built-in window. *Why:* the person asks what the model supports;
  the Agent answers that question the same way when it compacts and gates
  images. *Rejected:* the listing's fields only (today): an OpenAI or
  local Provider shows nothing although the Agent knows its window.
- **D2 — Resolved inside `list_provider_models`, with the entry as the
  reported layer.** Each Provider's task resolves its models right after
  its listing, with the settings the call read once, through one kernel call
  that locks the registry once. The reported layer is the entry itself; the
  learned layer is the stored one. *Why:* identical facts for the TUI, a
  host and `doctor` from one call; immune to the shrink guard and the route
  bound; no extra settings read. *Rejected:* resolving from the store after
  recording (stale or missing facts for a held or evicted listing); a
  separate `model_metadata` call per model (a settings read each, as
  `doctor` does today).
- **D3 — Three states, words in columns.** Thinking and vision are `yes`,
  `no` or `?`; the window is a compact size (`200K`, `1.05M`) or `?`. The
  hint says `? not known`. *Why:* the person asked "whether" — a blank
  cannot answer; words read without a legend. *Rejected:* `✓`/`✗`/`?`
  (narrow glyphs, `✓` U+2713 and `✗` U+2717 are East Asian Width N and
  already used by command cells, but under the selected row's cyan they
  read worse than words, and a ✗ looks like an error); "unknown" spelled
  out (doubles the Vision column).
- **D4 — Per-Provider arrival, composed in the TUI.** The view opens at
  once. The bridge reads the enabled Providers, then sends one
  `list_provider_models` request per Provider, and each answer fills its
  section as it arrives. *Why:* one slow or failing Provider no longer hides
  the others; the facade already serves one Provider per request
  (`crates/tui/src/agent_bridge/providers.rs:49`). *Rejected:* a streaming
  facade call (a callback or channel in the public API, which a host can
  already reproduce with one request per Provider); waiting for all behind a
  spinner (still hides the fast ones).
- **D5 — Fixed order.** The pinned row, then the sections in the facade's
  order (display name, case-insensitive, then id: `registry.rs:123-128`),
  each section's models in the listing's order. A section replaces its
  pending row in place. *Why:* arrival order is random; a fixed order keeps
  rows where the person expects them. *Rejected:* appending sections as they
  arrive.
- **D6 — The session's model is pinned first.** Its row is first from the
  start, selected, with facts from `model_metadata` (no network), replaced by
  its listing's when its Provider answers; it is left out of its Provider's
  section. *Why:* the row exists before any listing, and later arrivals land
  below it, so nothing moves under the cursor at open. *Rejected:* the row
  in its section (unknown until the section arrives, and it moves).
- **D7 — The selection holds its place.** An update keeps the search text,
  the selected row (by Provider and model id) and that row's line on screen:
  rows inserted above it move the scroll by as much. *Rejected:* rebuilding
  the view (loses the search) or keeping the index (selects another row).
- **D8 — A failed Provider is one selectable row; Enter lists it again.**
  The row says `not listed: <error>` and the hint says `enter lists again`;
  the detail line shows the whole error. *Why:* a retry key costs almost
  nothing, and letters type into the search, so Enter is the only free key.
  *Rejected:* `r` (types into the search); no retry (reopen `/model`, which
  lists every Provider again).
- **D9 — The model's facts, the policy's overrides said once.** The rows
  never fold in the model policy; when the policy sets `contextWindowTokens`
  or `imageInput = "disabled"`, the subtitle says so once ("The model policy
  sets every session's window to 128K; images are never sent."). *Why:* the
  list compares models; the policy applies to whichever is chosen.
  *Rejected:* folding the policy in (every row shows the same window, which
  answers nothing).
- **D10 — One vocabulary, in model-api; layout in the TUI.**
  `ModelMetadataSource::label()` (`configured`, `learned`, `reported`,
  `models.dev`, `built-in`) and `ModelMetadata::supports_reasoning()` /
  `supports_image_input()` (`Option<bool>`) live beside `known_reasoning`
  (`crates/model-api/src/model_metadata.rs:28-57`), re-exported by the
  facade for the TUI, the CLI, the settings summaries and hosts. Columns,
  compact sizes and the detail line stay TUI-private in `model_facts.rs`.
  *Rejected:* a facade formatting module (the facade formats nothing else);
  moving `format_tokens_compact` (Codex's, TUI-only; the settings form and
  `doctor` keep exact digits, which is what a person types into
  `contextWindowTokens`).
- **D11 — Search matches the id, the display name and the Provider's
  name.** `search_value` is the three joined. *Why:* "openrouter" or
  "Sonnet" find what the person means. *Rejected:* the id only (today).
- **D12 — A late answer is dropped, the request left to finish.** Closing
  the view, or opening `/model` again, does not cancel the listings: each
  view has a listing number, and an answer for another number is ignored.
  The listing still records its facts, as discovery would. *Why:* the facade
  has no cancellation for a listing (`registry.rs:159` passes
  `CancellationToken::default()`), and adding one to its public request
  buys little. *Rejected:* Codex's cancel on dismiss (`app/provider.rs:99-111`),
  which needs that API.
- **D13 — "Thinking: yes" means the model reasons.** `supports_reasoning()`
  is `Some(true)` for any resolved `Supported` fact, levels or not (a
  models.dev "reasons" included); `Some(false)` for `Unsupported`
  (configured `reasoningEfforts = []`, reported, or a learned refusal);
  `None` otherwise. Which levels it takes stays `known_reasoning()`'s
  stricter answer, for `/effort` and the detail line. *Rejected:* deriving
  the column from `known_reasoning()` (a models.dev "yes" would show `?`).
- **D14 — The facade keeps no list cache.** Every list is live, as today.
  (§5 Q7 for a stale list from memory.)

## 3. Design

### 3.1 Data: the facade's result [1]

`AgentProviderModelCatalogResult` (`crates/agent/src/service/provider/contracts.rs:19-26`)
keeps its shape; its `models` become listed models:

```rust
/// A model of a Provider's listing, with what is known of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentListedModel {
    /// What the listing reported, as it reported it.
    #[serde(flatten)]
    pub entry: ModelCatalogEntry,
    /// Its resolved facts, each with its source (`model_metadata`'s
    /// answer, with this listing as the reported source).
    pub metadata: ModelMetadata,
}

pub struct AgentProviderModelCatalogResult {
    pub provider_id: String,
    pub display_name: String,
    pub models: Vec<AgentListedModel>,
    pub error: Option<String>,
}
```

`list_provider_models` (`registry.rs:64-130`), per Provider task:

1. resolve the Provider from the settings read at the start
   (`resolve_settings_provider(&settings, id)`, `resolution.rs:38-55`), not
   from a second read (`resolve_provider`);
2. list (`provider_models`, unchanged; the kernel records the listing);
3. resolve every entry through one kernel call, with the Provider's
   `models."<id>"` (`configured_model_facts(provider, None, id)`,
   `resolution.rs:88-100`, as `model_metadata` does);
4. zip entries and facts into `AgentListedModel`s.

**Kernel.** `AgentService::resolve_listing(provider_id, protocol,
base_url, entries, configured: impl Fn(&str) -> AgentConfiguredModelFacts)
-> Vec<ModelMetadata>` in `crates/kernel/src/service/model_facts.rs`, beside
`model_metadata` (`model_facts.rs:22-39`): it names the Provider for
models.dev as `model_metadata` does, then calls
`ModelMetadataRegistry::resolve_listing(endpoint, entries, configured,
now)` (`crates/kernel/src/model_metadata/mod.rs`). That method takes the
lock **once** to copy what it needs (the models.dev copy's `Arc`, the
Provider's names, each listed route's live learned facts), releases it, and
resolves each entry with `resolve::resolve` over `RouteFacts { reported:
reported_facts(entry, now), learned }` (`state.rs:325-343` becomes
`pub(super)`). An invalid base URL gives no facts (`ModelMetadata::default()`
each), as `model_metadata` does.

**Cost.** Per model: a hash lookup of the learned facts and models.dev's
lookup, which visits each of its providers once (`crates/model-api/src/models_dev.rs:155-178`).
1 000 entries (a listing's cap) take well under the listing's own time, and
none of it holds the lock.

**Unchanged:** the request type, `test_provider_draft` (a draft has no id
and no per-model settings; it keeps returning entries), discovery, what is
recorded.

### 3.2 The vocabulary [1][3]

In `crates/model-api/src/model_metadata.rs`:

| Item | Returns |
| --- | --- |
| `ModelMetadataSource::label(self) -> &'static str` | `configured`, `learned`, `reported`, `models.dev`, `built-in` |
| `ModelMetadata::supports_reasoning(&self) -> Option<bool>` | D13 |
| `ModelMetadata::supports_image_input(&self) -> Option<bool>` | the resolved `image_input` fact's value |

In `crates/tui/src/model_facts.rs` (TUI-private), replacing `source_name`
and `entry_badges`:

| Function | Gives |
| --- | --- |
| `fact_cells(&ModelMetadata) -> FactCells` | the three cell texts: `format_tokens_compact(window)` or `?`; `yes`/`no`/`?` twice |
| `fact_columns(&FactCells) -> String` | the tabular text, `{window:>7}  {thinking:<8}  {vision:<6}` (25 columns, under `Context  Thinking  Vision`) |
| `fact_line(&FactCells) -> String` | the stacked text, `context 200K · thinking yes · vision yes` |
| `fact_details(model, provider, &ModelMetadata) -> String` | the detail line (§3.3) |

The detail line, every fact with its source:

- window: `context 200K (reported)`, else `context not known (compacts on
  the Provider's error)` (the words of `/status`,
  `crates/tui/src/history_cell/status.rs:119-125`);
- largest output: `max output 64K (reported)`, left out when not known;
- thinking: `thinking low, medium, high, default medium (reported)` when
  `known_reasoning()` names levels; `thinking yes, levels not known
  (models.dev)`; `thinking no (configured)`; `thinking no (learned: the
  Provider refused it)`; `thinking not known`;
- vision: `vision yes (reported)`, `vision no (learned)`, `vision not
  known`.

`/status` already says the window with `source_name`; it moves to
`label()`. The policy form's window summary (`model_policy.rs:283-290`)
uses `label()` instead of `name_of`, so it reads `models.dev` and `built-in`;
its number stays exact. `doctor --online`'s line keeps exact numbers and
its format, with `label()` and the two capability words (`window 200000
(reported), max output 64000 (reported), thinking low/medium/high
(reported), vision no (learned)`).

### 3.3 Presentation: rows, columns, narrow widths [1]

Three options were weighed. Each mock-up follows `ListSelectionView`'s
geometry: rows are drawn `width - 2` wide from the left edge, the selected
row's prefix is `› `, at most `MAX_POPUP_ROWS` = 8 rows show
(`crates/tui/src/bottom_pane/popup_consts.rs:7`), and the description column
starts two columns after the widest visible name, under the name when that
leaves it fewer than 10 columns (`compute_desc_col`,
`crates/tui/src/bottom_pane/selection_popup_common.rs:76-111`). They were
drawn by a script that applies those rules, not by the renderer: the
milestone's snapshots are the real thing, and may differ by a column. In
them, "answered" counts listed and failed Providers alike.

**Option A (chosen, §5 Q1): fact columns at the right edge, with a header.**
Needs a small extension of the shared row renderer (§3.3.1).

```text
--- 100 columns ---------------------------------------------------------------------------------
  Select Model
  Switch the model of this session.

  Type to search models
                                                                         Context  Thinking  Vision
› claude-sonnet-4-5 (current)              Anthropic                        200K  yes       yes
  claude-haiku-4-5                         Anthropic                        200K  yes       yes
  qwen3-coder:30b                          Local                               ?  ?         ?
  gpt-4.1                                  OpenAI                          1.05M  ?         ?
  gpt-5.1                                  OpenAI                           272K  ?         ?
  meta-llama/llama-3.1-405b-instruct:free  OpenRouter                       131K  no        no
  Team gateway                             listing…
  Work proxy                               not listed: 401 Unauthorized

  • Listing models… 5 of 6 Providers answered · 3s
  claude-sonnet-4-5 · Anthropic: context 200K (reported) · max output 64K (reported) · thinking
  low, medium, high (reported) · vision yes (reported)
  enter choose · esc close · ? not known
```

```text
--- 80 columns -----------------------------------------------------------
  Select Model
  Switch the model of this session.

  Type to search models
                                                     Context  Thinking  Vision
› claude-sonnet-4-5 (current)            Anthropic      200K  yes       yes
  claude-haiku-4-5                       Anthropic      200K  yes       yes
  qwen3-coder:30b                        Local             ?  ?         ?
  gpt-4.1                                OpenAI        1.05M  ?         ?
  gpt-5.1                                OpenAI         272K  ?         ?
  meta-llama/llama-3.1-405b-instruct:f…  OpenRouter     131K  no        no
  Team gateway                           listing…
  Work proxy                             not listed: 401 Unauthorized

  • Listing models… 5 of 6 Providers answered · 3s
  claude-sonnet-4-5 · Anthropic: context 200K (reported) · max output 64K
  (reported) · thinking low, medium, high (reported) · vision yes (reported)
  enter choose · esc close · ? not known
```

```text
--- 50 columns ---------------------------
  Select Model
  Switch the model of this session.

  Type to search models
› claude-sonnet-4-5 (current)  Anthropic
    context 200K · thinking yes · vision yes
  claude-haiku-4-5  Anthropic
    context 200K · thinking yes · vision yes
  qwen3-coder:30b  Local
    context ? · thinking ? · vision ?
  gpt-4.1  OpenAI
    context 1.05M · thinking ? · vision ?
  gpt-5.1  OpenAI
    context 272K · thinking ? · vision ?
  meta-llama/llama-3.1-405b-instruct:free
    context 131K · thinking no · vision no
  Team gateway
    listing…
  Work proxy
    not listed: 401 Unauthorized

  • Listing models… 5 of 6 Providers answered · 3s
  claude-sonnet-4-5 · Anthropic: context 200K
  (reported) · max output 64K (reported) ·
  thinking low, medium, high (reported) ·
  vision yes (reported)
  enter choose · esc close · ? not known
```

The `?` rows are realistic: OpenAI's own listing and a local server report
no capabilities, and models.dev is off by default (§7, §5 Q3).

**Option B: the facts lead the description, labelled.** No renderer
change: the description is `{window:>5}  thinking {t:<3}  vision {v:<3}
{provider}`, aligned among visible rows because `desc_col` is shared.

```text
--- 80 columns
› claude-sonnet-4-5 (current)   200K  thinking yes  vision yes  Anthropic
  gpt-5.1                       272K  thinking ?    vision ?    OpenAI
  deepseek/deepseek-r1          164K  thinking yes  vision no   OpenRouter
--- 50 columns
› claude-sonnet-4-5 (current)   200K  thinking
                               yes  vision yes
                               Anthropic
  gpt-5.1                       272K  thinking
                               ?    vision ?
                               OpenAI
```

The column moves sideways whenever a longer name scrolls into view
(`desc_col` follows the visible names), there is no header, and below
about 75 columns each row wraps into three lines.

**Option C: today's badges with negatives.** No renderer change: a
capability is named when yes, `no …` when no, `… ?` when not known.

```text
--- 80 columns
› claude-sonnet-4-5 (current)  Anthropic · 200K · thinking · vision
  gpt-5.1                      OpenAI · 272K · thinking ? · vision ?
  qwen3-coder:30b              Local · context ? · thinking ? · vision ?
  deepseek/deepseek-r1         OpenRouter · 164K · thinking · no vision
--- 50 columns
› claude-sonnet-4-5 (current)  Anthropic · 200K
                               · thinking ·
                               vision
  gpt-5.1                      OpenAI · 272K ·
                               thinking ? ·
                               vision ?
```

Nothing aligns, so every row must be read to compare, and narrow rows wrap
into three lines as in B.

**A was chosen** (Q1) because the three facts are compared down a column,
which only A lays out at every width, and its renderer change is opt-in
(§3.3.1).

**Priorities as the width shrinks** (A):

1. The display name is never in the row (it is in the detail line and
   searchable).
2. The Provider column shrinks to 10 columns, then is cut with `…`.
3. The model id is cut with `…` (its whole id is in the detail line), down
   to 20 columns.
4. Below that (under about 51 columns, when `width - 2 - 25 - 2` leaves the
   prefixed name fewer than 22), each row stacks: the name and, when it
   fits, two spaces and the Provider on the first line; the facts, labelled,
   on the second, cut with `…` rather than wrapped. The header row is not
   drawn in this layout.

`(current)` stays in the name, as `ListSelectionView` adds it
(`list_selection_view.rs:198-205`). A pending row is the Provider's name
with `listing…`, a failed one `not listed: <error>`, both in the Provider
column's place, dim, with no fact cells.

#### 3.3.1 The renderer extension

Shared by every picker, so it is opt-in and leaves every existing row
unchanged (their snapshots too):

- `SelectionItem` and `GenericDisplayRow` gain `facts: Option<RowFacts>`,
  `RowFacts { columns: String, stacked: String }`: the caller's fixed-width
  tabular text and its labelled one-line form.
- `SelectionViewParams` gains `facts_header: Option<String>`, drawn
  right-aligned on the line above the rows when the rows are tabular.
- `selection_popup_common.rs`: a row with `facts` draws `columns`
  right-aligned at the row's right edge; its name and description are cut
  (not wrapped) to the space left of it, the description keeping at least
  `MIN_DESCRIPTION_WIDTH`. When the space left would give the prefixed name
  fewer than 22 columns, it draws the stacked form under the name at
  `STACKED_INDENT`. A row without `facts` is drawn as today.
  `measure_rows_height` counts the same lines.
- `ListSelectionView::replace_items(items, selected: Option<usize>)`: new
  items with the search text kept and re-applied, `selected` (the caller's
  index of the row to keep) selected, and the scroll moved so that row stays
  on the same line.

`list_selection_view.rs` is at about 537 of its 600 code lines; the
extension adds about 30. The row drawing stays in
`selection_popup_common.rs` (about 285).

### 3.4 Loading and arrival [2]

**The loading line** (`crates/tui/src/bottom_pane/loading_line.rs`, new):
`LoadingLine::new(frames: FrameRequester, animations: bool)` remembers its
start, and `line(text) -> Line` gives `spinner(start, animations)`, the
text, and, from the first second on, `· 3s` (`fmt_elapsed_compact`). Drawing
it schedules the next frame in 200 ms (the spinner turns every 600 ms, the
counter every second), as the status indicator does; nothing is scheduled
once no line is drawn. No shimmer: it would redraw the list 30 times a
second. A view builds its `FrameRequester` from the `AppEventSender` it is
given (`crates/tui/src/tui/mod.rs:321-347`).

**`/model`'s view** (`crates/tui/src/commands/model/view.rs`, new;
`commands/model.rs` becomes `commands/model/mod.rs`):

```text
ModelListView
  listing: u64                      // this view's number (D12)
  loading: LoadingLine
  current: Option<CurrentModel>     // the pinned row (D6)
  sections: Vec<Section>            // in the facade's order (D5)
  list: ListSelectionView           // rows built from the above
Section { provider_id, name, state: Pending | Listed(Vec<AgentListedModel>) | Failed(String) }
```

Its states:

| State | Shows |
| --- | --- |
| Opened, nothing heard | Title and subtitle, then `• Listing models…`, then `esc close`. |
| Started | The pinned row with its facts, one pending row per enabled Provider, the loading line `Listing models… 0 of 6 Providers answered`. |
| Some answered | Those sections' rows or failed rows; the line counts both as answered. |
| All answered | The loading line goes; the detail line and the hint stay. |
| No Provider enabled | The pinned row, and `No Provider is enabled: /providers adds one.` in the loading line's place. |

**Events.** One update type on the existing reply path, replacing
`TurnUpdate::ModelChoices` (`crates/tui/src/agent_bridge/mod.rs:150`) so that
file, at about 597 of 600 lines, does not grow:

```text
AgentCommand::ListModels { listing: u64, provider_ids: Option<Vec<String>> }
    // None: start (Started, then every enabled Provider); Some: a retry
TurnUpdate::ModelList(ModelListUpdate { listing, event })
ModelListEvent::Started { providers: Vec<(id, name)>, current: CurrentModel, policy_note: Option<String> }
ModelListEvent::Listed(AgentProviderModelCatalogResult)   // one Provider's
CommandReply::ModelList(ModelListUpdate)
```

- `/model` (`dispatch`) shows the view and sends `ListModels` with a new
  number from a counter.
- The bridge (`crates/tui/src/agent_bridge/model_list.rs`, new) reads the
  registry snapshot and the session's facts (`model_metadata` for the
  session's pair; no network), replies `Started`, then spawns one
  `list_provider_models { provider_ids: Some([id]) }` per enabled Provider,
  each replying `Listed` when it ends. The calls run on the Agent's runtime
  as every bridge call does (`AgentBridge::spawn`, `agent_bridge/mod.rs:679-694`);
  `model_list.rs` sends several replies through the same sender.
- The chat widget hands each update to the open view
  (`context.offer_reply`, as `providers::on_models` does:
  `crates/tui/src/commands/providers/mod.rs:60-64`); with none open it is
  dropped. The view ignores another number's updates.
- On each update the view rebuilds its items, finds the selected row's key
  (Provider id and model id, or the failed Provider's id) in them and calls
  `replace_items` (D7).

**Keys.** Up/Down move (pending rows are skipped); typing searches; Enter
on a model sets it (`AgentCommand::SetModel`, as today) and closes; Enter
on a failed row turns it pending and sends `ListModels { provider_ids:
Some([id]) }` with the same number; Esc and Ctrl-C close. Choosing a model
while others still list is allowed; their answers are then dropped.

**Rows.** The pinned row: the session's model, `(current)`, the Provider's
display name, the facts `Started` carried, replaced by its listing's entry
when its Provider lists it. A section's model rows: `name` the id,
`description` the Provider's name, `facts` from `fact_cells`, `search_value`
id, display name and Provider name (D11). The same pair is never shown
twice; the same id on two Providers is two rows.

**Subtitle.** "Switch the model of this session." and, when the policy
overrides (D9), the sentence naming it.

**Hint.** `enter choose · esc close · ? not known`; on a failed row `enter
lists again · esc close`.

### 3.5 Failure [2]

- A Provider whose listing fails (an error, a timeout after its retries, a
  Provider disabled or deleted since the snapshot: `resolution.rs:42-53`) is
  its failed row; the others are unaffected.
- The registry snapshot failing: `Started` is not sent; the view shows the
  error in the loading line's place (`The Providers were not read: …`) and
  the pinned row is absent. Esc closes.
- `SetModel` failing is today's error event (`agent_bridge/commands.rs:121-137`).

### 3.6 The Provider page and the settings center [2]

- **Choose the model** (`ModelPicker`, `commands/providers/models.rs`):
  `Stage::Listing` draws the title and the loading line `Listing the
  models…` (§3.4) in place of its static text; `Stage::List`'s rows get the
  fact columns (the description is the display name, as today, where `/model`
  has the Provider), the detail line for the selected model, and its existing
  "Enter a model id…" row. One Provider, so no arrival per section.
- **List models** (settings center): the busy notice names what runs:
  `List models…` for an action (its label), `Saving…` for an edit
  (`Pending::Action` carries the action, `settings_center/mod.rs:135-141`).
  Text only: the center has no frame source, and the action is one listing.

### 3.7 Consistency: what moves where [3]

| Surface | Before | After |
| --- | --- | --- |
| `/model` rows | listing's window, `vision`, `reasoning` badges | resolved facts in three columns (L2) |
| Provider page rows | the same badges after the display name | the same columns (L3) |
| `/status` window | `source_name` | `label()` (L1) |
| Policy form window summary | `200000 (built_in)` | `200000 (built-in)` (L1) |
| `doctor --online` model line | its own source names; `images`, `reasoning` | `label()`; `vision`, `thinking`; facts from the listing call (L1) |
| A host | entries only | entries plus `metadata`, and the same accessors (L1) |

### 3.8 The host contract [1][3]

- `list_provider_models` returns each model's resolved facts
  (`AgentListedModel`); the JSON is the entry's fields plus `metadata`, so a
  frontend reading the flat fields keeps working.
- A host wanting per-Provider arrival sends one request per Provider, as the
  TUI does; the embedding page says so.
- `ModelMetadataSource::label`, `supports_reasoning` and
  `supports_image_input` are public on re-exported types.
- Hanafubuki: its Rust passes the result through unchanged
  (`agent_list_provider_models`); its TypeScript `AgentProviderModelCatalogEntry`
  may add `metadata` when it wants the facts. Nothing breaks at its next pin.

### 3.9 Text that becomes stale

Both languages (`x.en.mdx` and `x.mdx`):

- L1: `embedding#configuration-in-a-host` (a `list_provider_models` row:
  each model's `metadata`, one request per Provider for arrival; the
  `model_metadata` row's mention of the vocabulary); `architecture`'s model
  metadata paragraph (a listing's facts are resolved in the call, the entry
  as the reported source); `cli-reference#doctor` (`vision`, `thinking`);
  `providers-models#model` ("each model with what is known of it").
- L2: `terminal-interface`'s `/model` row (`:132`: the columns, `?`, the
  loading line, Enter on a failed Provider) and its list paragraph (`:46`);
  `providers-models#model` (`:97`, what `/model` shows and that it fills per
  Provider); `internals`' command map (`:515`: `/model` lists per Provider).
- L3: `providers-models#interface` (`:83-84`: **Choose the model**'s
  columns, the loading line; **List models**' notice); the settings
  center's paragraph in `terminal-interface#config` only if it names the
  busy notice.

## 4. Edge cases

| Case | Behaviour |
| --- | --- |
| No Provider enabled | `Started` with none: the pinned row and `No Provider is enabled: /providers adds one.` |
| One Provider failing | Its failed row; Enter lists it again; the others list normally. |
| One Provider slow (up to its retries, about 90 s by default) | Its pending row, the loading line counting; every other section is usable; Esc closes at any time. |
| Hundreds or thousands of models | Resolution: one lock to copy, then per-entry lookups outside it (§3.1). Rendering: `build_rows` is linear per frame and frames come every 200 ms only while a Provider is pending; the search is a linear substring scan per key, as today. 32 Providers × 1 000 entries is the theoretical bound. |
| A model id longer than the screen | Cut with `…` in its row; whole in the detail line, which wraps. |
| The session's model in no listing | The pinned row keeps `Started`'s facts (its settings, learned, models.dev, built-in). |
| Facts change while the list is open | `/model` is refused during a turn and the open list keeps the composer from submitting, so only background work (a sub-agent's turn, discovery) can learn or record a fact. The list shows the facts of its own listings; reopening shows newer ones. |
| `/model` during a running turn | Refused, as today. |
| `/model` before any session exists | `needs_session` sends `Op::EnsureSession` and then runs `dispatch` at once (`crates/tui/src/chatwidget/commands.rs:24-32`), so the view opens at once; the bridge takes `EnsureSession` before `ListModels`, so `Started` follows the session's creation. Until then the view shows `• Listing models…`. |
| The same id on two Providers | Two rows, each with its Provider and its route's facts; `(current)` only on the session's pair. |
| Discovery off | No effect: `/model` lists live, as the docs say (`providers-models#model`). |
| models.dev off (the default) | Thinking and vision are `?` wherever the listing says nothing and no setting does (§5 Q3). |
| Animations | No setting exists; production animates, snapshot tests construct the line with `false` (§6). |
| A second `/model` while one loads | The new view replaces the old (`show_command_view`, `crates/tui/src/bottom_pane/mod.rs:312-324`) with a new number; the old listings finish and their answers are dropped. Both may list one Provider at once; harmless. |
| Closing mid-fetch | The listings finish and record their facts (D12); their answers find no view and are dropped. |
| A held (shrink-guarded) listing | The rows show what this listing says (D2); sessions keep the stored facts until the next listing is trusted. Documented in `architecture`. |
| `/model` in a side conversation | Unavailable there, as today (`crates/tui/src/app/tests/side.rs:95-96`). |
| The policy disables images or sets a window | Rows show the model's facts; the subtitle says the policy's (D9). |

## 5. The questions, answered

The user answered every question on 2026-11-14, each as recommended. Each
entry keeps the question and records the decision; none is open.

- **Q1 — Presentation.** A, B or C (§3.3)? **Decided by the user on
  2026-11-14, as recommended: A**, aligned columns with a header, `yes`,
  `no` or `?`, and the detail line.
- **Q2 — The "not known" mark.** `?`, `—`, or `unknown`? **Decided by
  the user on 2026-11-14, as recommended: `?`**, with `? not known` in the
  hint.
- **Q3 — A models.dev hint.** With models.dev off (the default) and a shown
  row at `?`, add `· models.dev can fill these: /config modelMetadata` to the
  hint? The bridge reads `modelMetadata.modelsDev` from `settings_view()`
  and `Started` carries it; a dozen lines in L2. Turning it on sends a daily request to a third party,
  so the hint only names it. **Decided by the user on 2026-11-14, as
  recommended: yes.**
- **Q4 — Pinning.** The session's model first (D6), or in its section?
  **Decided by the user on 2026-11-14, as recommended: first.**
- **Q5 — Search scope.** Also the display name and the Provider's name
  (D11)? **Decided by the user on 2026-11-14, as recommended: yes.**
- **Q6 — Cancel on close.** Leave the listings to finish (D12), or add
  cancellation to the facade's request? **Decided by the user on
  2026-11-14, as recommended: finish**; no cancellation is added.
- **Q7 — A stale list at once.** Show the last recorded listing (model ids
  from `ListingState.models`, `state.rs:55-64`, which O1 exposes as
  `listed_models`) at once, marked "listed 3 h ago · refreshing…", then the
  live one? It needs a facade read, the display names (not recorded), and a
  section whose rows change under the cursor when the live listing arrives.
  The pinned row and per-Provider arrival already give feedback at once.
  **Decided by the user on 2026-11-14, as recommended: not now;** revisit
  after O1, if a slow Provider still annoys.
- **Q8 — A `models` CLI subcommand.** **Decided by the user on
  2026-11-14, as recommended: no.** The request is the terminal
  interface; `doctor --online` already prints the default Provider's models
  with their facts.
- **Q9 — Capabilities in the built-in table.** Mark the families it lists
  (gpt-5, o3, claude-, gemini-2.5) as reasoning or reading images, so
  OpenAI's rows lose their `?`? It changes `resolve`'s capability rule and
  what the image gate and `/effort` see, beyond display. **Decided by the
  user on 2026-11-14, as recommended: no;** Q3's hint points at models.dev
  instead.
- **Q10 — `doctor`'s words.** `images`/`reasoning` become `vision`/`thinking`
  (§3.2)? The CLI's text output has no stability rule (only `exec --json`
  does). **Decided by the user on 2026-11-14, as recommended: yes.**

## 6. Milestones

Strictly in order, each one commit on main with green CI on Linux, Windows
and macOS before the next: **L1 → L2 → L3**. They touch none of the files
M6–M8 list. L1 shares four files with O1's list, each for a different
function: `model-api/src/model_metadata.rs` (O1: `known_reasoning`, which
already exists; L1: `label` and the two accessors),
`kernel/src/model_metadata/mod.rs` and `kernel/src/service/model_facts.rs`
(O1: `listed_models`, the ids of the **recorded** listing; L1:
`resolve_listing`, the facts of a **live** listing's entries: the names
differ on purpose), and `tui/src/model_facts.rs` (O1: `effort_options`;
L1–L3: the cells and the detail line). Whichever lands second rebases.
Their place in the queue, decided by the user on 2026-11-14: after M8,
before O1 (§8). Each milestone records in `todo.md`: what landed and any
deviation, the validation commands run and skipped, the CI run's URL (no
Windows or macOS claim from a Linux run), whether goldens changed, what
Hanafubuki must change at its next pin, and any module-size exception.

Every milestone runs AGENTS.md's matrix (`cargo fmt --all --check`;
`RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
--all-features`; `cargo clippy --workspace --all-targets --all-features --
-D warnings`; `cargo xtask check`; `cargo test --workspace --all-features`)
and, since each changes pages, the site's (`bun install --frozen-lockfile`,
`bun run format:check:site`, `bun run site:check`, `bun run site:test`,
`VITE_BASE_PATH=/hanafubuki-agent/ bun run site:build`, every page
prerendered).

**Goldens: none** in any milestone. Nothing here reaches a model request;
the tests' discovery is off and the listings are the tests' own.

- **L1 — every listed model with its resolved facts** (§3.1, §3.2, §3.7).
  - Files:
    - `crates/model-api/src/model_metadata.rs` (`label`,
      `supports_reasoning`, `supports_image_input`);
    - `crates/kernel/src/model_metadata/{mod.rs,state.rs}`
      (`resolve_listing`; `reported_facts` `pub(super)`) and
      `crates/kernel/src/service/model_facts.rs` (`resolve_listing`);
    - `crates/agent/src/service/provider/{contracts.rs,registry.rs}`
      (`AgentListedModel`; the per-Provider resolution from the settings
      read once), `crates/agent/src/lib.rs` and
      `crates/agent/src/service/{mod.rs,provider/mod.rs}` (re-exports);
    - `crates/agent/src/service/settings/{providers.rs,model_policy.rs}`
      (`.entry.id`; `label()` in the window summary);
    - `crates/cli/src/doctor.rs` (the result's facts, no per-model call;
      `label()`; `vision`, `thinking`);
    - `crates/tui/src/model_facts.rs` (`source_name` goes for `label()`;
      the badges read the resolved facts, still inline),
      `crates/tui/src/commands/{model.rs,providers/models.rs}`,
      `crates/tui/src/agent_bridge/{providers.rs,status.rs}`,
      `crates/tui/src/history_cell/status.rs` (mechanical).
  - Tests:
    - model-api (`crates/model-api/tests/model_metadata.rs`): `label` for
      each source; both accessors for each source and value, a models.dev
      "reasons" without levels being `Some(true)` while `known_reasoning`
      stays `Unknown`;
    - kernel (`crates/kernel/src/model_metadata/tests/listing.rs`):
      `resolve_listing` takes the entry over a stored reported fact, keeps a
      learned "no" over a reported "yes" and a smaller learned window, takes
      configured facts first, fills from a recorded models.dev copy and the
      built-in window; a listing the shrink guard held still resolves from
      its entries;
    - facade (`crates/agent/src/tests/model_discovery.rs` or a new
      `crates/agent/src/tests/model_list.rs`), over the fake server: an
      OpenAI-shaped listing without a window gives `gpt-5.1` 272K
      `BuiltIn`; a `models."<id>"` window and `reasoningEfforts = []` win
      (`Configured`); a learned image refusal shows `Some(false)`; an
      OpenRouter-shaped entry's facts are `Reported`; a failing Provider
      keeps its `error` and the others their models; the JSON of a result
      has the entry's fields flat beside `metadata`;
    - per-Provider independence (D4's premise), over two fake servers, the
      first's listing held (`FakeListing::held`): a single-id
      `list_provider_models` for the second returns, with its facts, while
      the first is still pending; releasing the first then answers its own
      request;
    - `doctor` (`doctor_online_lists_each_model_with_what_is_known_of_it`,
      `crates/cli/tests/exec.rs:495-505`): the model line's words and
      sources;
    - the TUI's existing checks hold, `"Unset, the model's: 200000
      (reported)"` included; the badges' tests in `model_facts/tests.rs`
      move to resolved facts (a built-in window now shows).
  - Docs (§3.9, L1).
  - Acceptance: one `list_provider_models` call gives every model with the
    facts `model_metadata` would give it (with this listing as reported),
    reading `providers.toml` once; `doctor --online` makes no per-model call;
    every surface names sources alike.
  - Module size: `registry.rs` is at about 379 code lines; the resolution
    adds about 20. No exception.
  - Hanafubuki: nothing to change (its Rust passes the result through; the
    JSON only gains `metadata`).
- **L2 — `/model` opens at once, fills per Provider, shows three columns**
  (§3.3, §3.4, §3.5).
  - Files:
    - `crates/tui/src/bottom_pane/{selection_popup_common.rs,list_selection_view.rs}`
      (`facts`, `facts_header`, `replace_items`);
    - `crates/tui/src/bottom_pane/loading_line.rs` (new) and
      `bottom_pane/mod.rs` (its module line);
    - `crates/tui/src/commands/model/{mod.rs,view.rs}` (`model.rs` moved;
      `view.rs` new);
    - `crates/tui/src/model_facts.rs` (`fact_cells`, `fact_columns`,
      `fact_line`, `fact_details`; `entry_badges` goes once L3 drops its
      last caller);
    - `crates/tui/src/agent_bridge/model_list.rs` (new),
      `agent_bridge/{mod.rs,commands.rs}` (the variant replaced, the old
      arm removed), `crates/tui/src/app_event.rs` (`ListModels`'
      fields), `crates/tui/src/commands/mod.rs` (`CommandReply::ModelList`),
      `crates/tui/src/chatwidget/{commands.rs,turn.rs}`.
  - Tests:
    - `crates/tui/src/bottom_pane/list_selection_view/tests.rs`: every
      existing snapshot unchanged; new snapshots of rows with `facts` at
      100, 80 and 50 columns (header, cut names, stacked form); `replace_items`
      keeps the query, the selected row by the caller's index, and its line
      when rows are inserted above it;
    - `crates/tui/src/commands/model/tests.rs` (new), insta snapshots of
      `ModelListView` at 100, 80 and 50 columns: opened, started with
      pending rows, partly answered with a failed row, all answered; the
      loading line built with animations off and drawn within its first
      second, so it shows a dim `•` and no counter; the counter's text is
      tested through `fmt_elapsed_compact`'s existing tests;
    - the view's logic: an update for another number is ignored; the pinned
      row is not repeated in its section and takes its listing's facts; the
      selection survives a section arriving above it; Enter on a failed row
      sends a retry for that Provider only;
    - end to end (`crates/tui/src/app/tests/model_list.rs`, new), on the
      harness with two Providers on two fake servers, the second's listing
      held (`FakeListing::held`; the harness gains a constructor taking
      listings, as `FakeServer::with_listings` does): `/model` shows
      `Listing models…` and the first Provider's rows while the second shows
      `listing…`; releasing it fills its rows; Esc before the release, then
      the release, leaves no view and no error; a failing listing, Enter on
      its row, and a second listing request reaches the server;
    - `chatwidget/tests/mod.rs`'s `ModelChoices` test moves to the new
      update; `app/tests/settings.rs`' `"200K · vision"` becomes the
      column check.
  - Docs (§3.9, L2).
  - Acceptance: `/model` shows its view within one frame with a moving
    indicator; each Provider's models appear as it answers, in a fixed
    order, without moving the selected row; every model row shows a window
    or `?` and thinking and vision as `yes`, `no` or `?`; a failed Provider
    can be listed again with Enter; the selected model's facts and sources
    are under the list.
  - Module size: `list_selection_view.rs` about 537 → 570 of 600;
    `agent_bridge/mod.rs` unchanged at about 597 (a variant replaced, the
    code in `model_list.rs`). No exception; if the extension does not fit
    `list_selection_view.rs`, `replace_items` goes to a child module rather
    than a raised limit.
  - Hanafubuki: nothing (TUI only).
- **L3 — the Provider page and the settings center** (§3.6).
  - Files: `crates/tui/src/commands/providers/models.rs` (the loading line,
    the columns, the detail line); `crates/tui/src/model_facts.rs`
    (`entry_badges` removed); `crates/tui/src/settings_center/{mod.rs,render.rs}`
    (the busy notice names the action).
  - Tests: `crates/tui/src/commands/providers/tests.rs`: snapshots of the
    Listing stage (loading line, animations off) and the list at 80 and 50
    columns; `app/tests/settings.rs`: **List models** shows `List models…`
    while the held listing waits, then its message; an edit still shows
    `Saving…`.
  - Docs (§3.9, L3).
  - Acceptance: **Choose the model** shows the same indicator and columns as
    `/model`; **List models** no longer says `Saving…`.
  - Module size: `settings_center/mod.rs` about 555 of 600, plus a few
    lines.
  - Hanafubuki: nothing.

## 7. Risks

- **Many `?` rows.** OpenAI's own listing and local servers report no
  capabilities, and models.dev is off by default, so thinking and vision
  are often `?`. The list is honest about it, the detail line says why, and
  Q3's hint names the remedy; a fuller answer (Q9) is out of scope.
- **The shared renderer.** Every picker draws through
  `selection_popup_common.rs`. The extension is opt-in and the existing
  snapshots are the guard; a regression would show in them.
- **A public type change.** `models`' element type changes for Rust callers
  (`.entry.id`). Hanafubuki's Rust only passes it through; another host
  would adapt in one line.
- **Divergence with a held listing.** The list shows what a listing said
  while sessions keep the trusted facts until the next listing (§4).
- **Time in tests.** The spinner and the counter depend on the clock; the
  snapshot rule (animations off, drawn within the first second) and a
  non-snapshot end-to-end check keep CI deterministic on slow runners.
- **Module limits.** `agent_bridge/mod.rs` is at its limit and must not
  grow; `list_selection_view.rs` and `settings_center/mod.rs` are close.
  The plan puts new code in new modules.

## 8. Fit with other work

- **The queue.** After M8, before O1, as decided by the user on
  2026-11-14. The L milestones share no file with M6–M8 (mermaid, command
  colour, JSON and diffs); with O1 they share four files, for different
  functions (§6). Before O1 because O2's guidance and O3's
  `/subagents` name models and should use the same vocabulary, and because
  O1's `listed_models` then lands beside `resolve_listing` knowing it.
- **O1–O3** (`docs/plan-subagent-model-overrides.md`): O2's model list in
  the guidance uses `known_reasoning` for levels and may use
  `supports_reasoning` / `supports_image_input` and `label()` where it
  names a capability; O3's `/subagents` picker, if it shows a child's model
  facts, uses `fact_cells`. A future picker of a sub-agent's model is
  `ModelListView` with another action. O1's `listed_models` is what Q7
  would build on.
- **R1–R8** and the remaining M milestones are unrelated.
