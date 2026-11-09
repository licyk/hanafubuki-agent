# Plan: visible thinking, and the Agent's documentation site

Repositories: the Agent (`/root/code_workspace/hanafubuki-agent`, main `edad969`)
and Hanafubuki (`/root/code_workspace/Hanafubuki-dev`, pinned to the Agent at
git rev `adc037a`). Release work (0.1.0 publishing, Phase 7) is postponed.

## 1. What happens today (verified)

**The model API drops the text.** Paths below are under `crates/model-api/src`.

| Protocol | What the provider can send | What is kept |
| --- | --- | --- |
| Responses | `reasoning_summary_text.delta`, `reasoning_text.delta` (raw, e.g. gpt-oss/vLLM), item `summary[]`/`content[]`, `encrypted_content` | summary deltas only (`responses/decode.rs:48-64`); raw text traced and dropped (`:36-47`). The request never sends `reasoning.summary` (`responses/encode.rs:44`), so OpenAI sends **no** summary at all. |
| Chat Completions | `delta.reasoning_content` (DeepSeek, Qwen, vLLM), `delta.reasoning` (OpenRouter) | `reasoning_content` traced and dropped (`chat_completions/mod.rs:171-175`); `reasoning` not read |
| Anthropic Messages | `thinking` blocks, `thinking_delta`, `signature_delta` (only when the request enables thinking) | traced and dropped (`anthropic_messages/mod.rs:255-313`); the request never enables thinking (`encode.rs:12-14`) |

**The kernel keeps only a headline.**
- `service/reasoning_headline.rs` turns the streamed summary into one ≤200-byte line, the latest bold or first line.
- That line goes out as `AgentEventKind::ReasoningHeadlineUpdated` and is held in `AgentTurnSnapshot.reasoning_headline`, in memory only.
- `model/reasoning.rs` keeps timing only (`AgentReasoningSnapshot`: phase, `active_since`, step and turn milliseconds).
- `ConversationItem` has no reasoning variant. Nothing is persisted or replayed. The code documents this "content-free" stance, and leak tests assert it:
  - `model-api/tests/http_contract/streaming.rs:149-210`;
  - `agent/src/tests/tools/execution.rs:4-58`.

**Where "Working" shows thinking.**
- **TUI:** `agent_bridge/mod.rs:235` maps the headline to `TurnUpdate::Status`. `chatwidget/turn.rs:48-52` then replaces the "Working" header with it.
- **Hanafubuki:**
  - `AgentPanel.vue:363` `workingLabel` puts `reasoningHeadline` first.
  - `AgentWorkingStatus.vue:51-78` adds a "Thinking {duration}" / "Thought for {duration}" timer.
  - The Tauri sink coalesces headline events (`src-tauri/src/agent/events.rs:329-339`).

The AGENTS.md content-logging rule limits **logs**, not typed history. A typed, canonical reasoning item is allowed. What stays forbidden is putting raw wire records or diagnostics into history.

## 2. Thinking: design

### 2.1 What text is shown

Show whatever thinking the provider streams, as Markdown:

- **Responses:**
  - Send `reasoning: {effort, summary: "auto"}` whenever an effort is set, as Codex does.
  - Keep both summary deltas and raw `reasoning_text` deltas, and stream whichever arrives into the block. Providers send one or the other.
  - If an item streams both, the stored text is the summary, decided at `output_item.done`.
  - Summary parts (`summary_index`) are joined with a blank line.
- **Chat Completions:** keep `reasoning_content` and `reasoning`.
- **Anthropic:** map `thinking_delta` when it arrives. **Do not** start requesting thinking now: with tools, Anthropic requires the signed thinking block to be sent back, which is replay (§2.4). Recorded as a follow-up.
- **Output budget.** This would otherwise be a regression:
  - Today raw thinking never reaches the kernel, and OpenAI sends no summaries. So in practice only the answer uses `maximum_model_output_bytes` (`kernel/.../stream/events.rs:81-86`).
  - Counted as they are now, a turn with 30 KB of DeepSeek-R1 or gpt-oss thinking would start failing.
  - Thinking deltas are therefore **taken out of** that budget and get their own cap per reasoning item (proposed: 256 KiB). Past the cap, the rest of that item is not shown or stored, and the stored text is marked as cut. The turn goes on.

### 2.2 Events and state (kernel, facade)

- **New `AgentEventKind` variants**, carrying no provider ids:
  - `ReasoningStarted { reasoning_id, sampling_step }`
  - `ReasoningDelta { reasoning_id, delta }`
  - `ReasoningCompleted { reasoning_id, duration_millis }`
- **`reasoning_id`:** the kernel's own id, one per provider reasoning item (or per stretch of Chat/Anthropic thinking).
- **Duration:** measured by the kernel from the first delta (or item start) to item completion. The existing `AgentReasoningSnapshot` timing stays, for status and recovery.
- **Removed outright** (AGENTS.md prefers clean removal):
  - `ReasoningHeadlineUpdated`;
  - `AgentTurnSnapshot.reasoning_headline`;
  - `TurnRecord.reasoning_headline`;
  - `reasoning_headline.rs` and its tests;
  - the `PendingModelItem::Reasoning.headline` projection.
- **Persisted** (recommended, decision 1): a new `ConversationItem::Reasoning { text, duration_millis, sampling_step, truncated }`.
  - It is committed at the step's finalize, with the step's other items, ahead of its assistant message. The switch from live to durable is then one commit, as for assistant messages. This lets a reloaded or resumed conversation show "Thought for 12s" and expand to the text.
  - **Cap:** the text is capped (the per-response output budget already bounds it); the cut is marked.
  - **Not model input:** `as_model_input` skips it, compaction ignores it, and it is not counted as context.
  - **Old history:** decoding is tolerant and the change is additive, so old history still opens. The existing history corpus is **not** re-recorded; it keeps proving that old files open.
  - **New fixture:** a new corpus fixture with a reasoning item is recorded once with `HANAFUBUKI_AGENT_UPDATE_GOLDEN_HISTORY=1`. This is an intended format change, so it needs your approval (decision 6). Hanafubuki's integration mirror gets the same fixture.
- **Turn page:** the `AgentTranscriptPage` serializer carries the item as it is; reasoning is meant to be shown.

### 2.3 Compatibility and versions

- **Breaking change:** removing the headline event changes the facade API and the `exec --json` stream. `docs/cli.md`'s stability rule allows removals only with a minor version before 1.0. So the next Agent release with this is **0.2.0**, not 0.1.1.
- **Hanafubuki:** it is still pinned to a git revision, so it moves to the new revision now. Phase 7 later takes `"0.2"` from crates.io.
- **Stale version notes:** both todo.md files say the configuration recovery "ships in 0.1.1". They are reworded to "the next release (0.2.0)".
- **T3 is the first pin past `76cf822`.** It therefore also maps `AgentConfig::recoveries()` into the Task Center's configuration recovery notice, the open item in Hanafubuki's todo.md.

### 2.4 Out of scope (recorded in todo.md)

- **Sending thinking back to the model:** Responses `include: ["reasoning.encrypted_content"]` plus a reasoning input item; Anthropic signatures. This changes every request golden and is a separate, model-visible decision.
- **Anthropic thinking requests:** they depend on that replay.

### 2.5 Snapshot tests that change

- **Request snapshots:**
  - Snapshots for sessions with an effort set gain `"summary": "auto"`. They are regenerated once, deliberately (`HANAFUBUKI_AGENT_UPDATE_GOLDEN=1`), and the diff is checked to contain only that.
  - Scripted streams without reasoning give unchanged snapshots.
  - Neither repository's history corpus changes.
- **Leak tests:** they are rewritten to assert the opposite. Thinking text reaches the typed events and the reasoning item. It still never reaches logs below `trace`, and never reaches model input.

### 2.6 TUI (`crates/tui`)

- **Removed:** the bridge's headline → `Status` mapping. The status header stays "Working" (or the retry header).
- **A `ThinkingCell` in the viewport while it streams:**
  - Collapsed (the default), it shows `• Thinking 5s` with the shimmer, counting up.
  - Expanded, it shows the streamed Markdown, dim, under the same header.
  - One key toggles it; Ctrl-O is proposed (checked against the composer's bindings) and listed in the footer and key hints.
- **On completion:** the cell is written to scrollback as it is shown, either `• Thought for 12s` or the header and text. In an inline terminal, scrollback can't be changed afterwards. The full text is always in the transcript pager (Ctrl-T).
- **Resume:** the replay maps `ConversationItem::Reasoning` to the same collapsed cell.
- **Tests:** snapshot tests for the collapsed, expanded and completed cells. The bridge test gets a scripted stream with reasoning.
- **`exec`:**
  - Human output prints `· thought for 12s` on stderr and no content.
  - `--json` carries the new event kinds.
  - `docs/cli.md` loses `reasoning_headline_updated` and gains the new kinds.

### 2.7 Hanafubuki (frontend and Tauri)

- **Pin:** bump the git revision. The integration supplies nothing new.
- **Tauri sink (`src-tauri/src/agent/events.rs`):** `ReasoningDelta` becomes a non-control event that merges with consecutive deltas of the same `reasoning_id`, like text deltas, up to 32 KiB. The headline rule and its tests go.
- **Types:** `src/types/agent/events.ts` and `conversation.ts` get the new event kinds and the `reasoning` conversation item, and lose `reasoningHeadline`.
- **Projection:**
  - `agentPresentationProjection.ts` keeps the live thinking blocks per `reasoning_id`: text, start time, duration.
  - `agentTranscript.ts` gets a `thinking` entry kind, built from durable items and from live blocks.
  - A live block is ordered before the live assistant entry of the same `sampling_step` (live entries are appended today, `:512-543`).
- **`AgentThinkingBlock.vue`** on `BaseCollapsible`, collapsed by default:
  - **Summary while live:** `Thinking 5s`, with `AgentShimmerText`.
  - **Summary when done:** `Thought for 12s`.
  - **Expanded:** `AgentMarkdown` with the caret while streaming; it follows the stream the way an assistant message does.
  - It uses the design system's role tokens and motion helpers (no font-weight transition), and i18n in en and zh.
- **The Working status:**
  - `workingLabel` drops the headline, so the order is the activity label, then the status.
  - Recommended (decision 2): remove `AgentWorkingStatus`'s separate "Thinking {duration}" timer, since the block now shows that time. The turn timer stays.
- **Tests:**
  - `AgentPanel.status.test.ts`, `agentPresentationProjection.test.ts` and the controller harness: the headline cases are replaced by thinking blocks.
  - The Tauri coalescer tests are updated.
  - A component test for the block covers its collapsed default, the live timer, expanding and the streamed text.
  - The integration goldens are regenerated only if a fixture streams reasoning.
- **Docs:** `docs/agent-integration.md` and the site's `agent-*` pages (the Agent in Hanafubuki) describe the block.
- **Before T3 starts:** Hanafubuki's tree holds the uncommitted Phase 7 edits, which touch the same manifest and `docs/agent-integration.md`. They are set aside first:
  - `git stash push -m phase7 -- docs/agent-integration.md package.json src-tauri/Cargo.toml src-tauri/Cargo.lock scripts/agent-local.ts scripts/agent-local.test.ts`, run after `git add -N` on the two new scripts so the stash includes them.
  - T3 then starts from the committed state, and the stash is reapplied after T3.

## 3. The documentation site (Agent repository)

### 3.1 Framework

Copy Hanafubuki's `site/` into the Agent repository as `site/`:
- Fumadocs core/ui 16.15.17 and fumadocs-mdx 15.4.5, on TanStack Start with a static prerender.
- React 19, Tailwind 4 and Vite 8, plus Orama static search.
- The theme toggle, the motion tokens and the tests.

New repository tooling, the same as Hanafubuki's:
- a root `package.json` (`packageManager: bun@1.4.2`, `workspaces: ["site"]`, scripts `site:dev|build|check|test|preview`, `format:site`, `format:check:site`), with prettier;
- `.prettierrc.json` and `.prettierignore`;
- `bun.lock`;
- `.gitignore` entries for `node_modules`, `site/.output`, `.nitro`, `.tanstack`, `.source`.

Drop the Hanafubuki-only parts:
- the home and download pages, with their components, styles, lib modules and tests (`download*`, `home-sections`, `supported-webuis`, `agent-demo`, `product-workspace-preview`);
- the `/download` entries in `vite.config.ts` `pages` and in `prepare-static-output.mjs`.

Replace the branding:
- title and description "hanafubuki-agent";
- nav links Docs and GitHub;
- the locale storage key `hanafubuki-agent.site.locale`;
- the favicon (from the Agent's logo, or a neutral one if there is none);
- `meta.json` titles and the `— hanafubuki-agent` page suffix.

A small home page: the name, one sentence, the install command, and links to Installation, Quick start and GitHub.

### 3.2 Giscus and Umami

- **Giscus:** `src/lib/giscus.ts` takes the given values exactly: `licyk/hanafubuki-agent`, `R_kgDOVnCscQ`, `Comment`, `DIC_kwDOVnCscc4DKsvU`, `mapping=pathname`, `strict=1`, reactions on, `emit-metadata=0`, `input-position=top` and `loading=lazy`.
  - Hanafubuki uses `specific` + `data-term`. With `pathname` the `term` helper and its test go.
  - Theme and language are covered by decision 7.
- **Umami:** `__root.tsx` gets the given script, with `data-website-id="11187ebe-09a5-4a52-a81b-7438a4e3f765"` and `async`, as Hanafubuki does.
- **Tests:** `giscus.test.ts` checks the attributes, the theme mapping and a Giscus language for every locale. A test checks that the Umami script has the given id.

### 3.3 Content: a complete rewrite

All 7 current files (4,724 lines, English, written for developers) are rewritten as MDX pages under `site/content/docs/`. Information architecture, with page slugs:

1. **Getting started**
   - `index`: what the Agent is, standalone versus embedded, feature tour.
   - `installation`: `cargo install hanafubuki-agent-cli`; the release archives with their checksums; building from source; platform notes (Linux Secret Service for the keyring, macOS Gatekeeper, Windows); upgrading.
   - `quick-start`: add a Provider, a first `exec`, the terminal interface.
2. **Using the Agent**
   - `terminal-interface`: layout, keys, slash commands, `@` mentions, the pager, resume, thinking.
   - `exec`: prompts, human output, `--json`, approvals, exit codes, scripting.
   - `sessions`: list, show, archive, delete, resume.
   - `providers-models`: protocols, endpoints, tokens, the model policy, reasoning effort, compaction.
   - `permissions`: modes, approvals, what each tool asks.
   - `tools`: commands and terminals, file edits and patches, web search and fetch.
   - `mcp`: adding servers (stdio and HTTP), credentials, approval and exposure policies, connecting.
   - `skills`, `memory`, `plan-mode`.
3. **Configuration**
   - `configuration`: directories, `HANAFUBUKI_AGENT_HOME`, the three files.
   - `config-layers`: defaults, user file, profiles, trusted projects, `-c`.
   - `config-reference`: every key, with its type, default and description.
   - `network-logging`: proxy and log level.
   - `recovery`: repairs, `.recovery/` and read-only files.
4. **Reference**
   - `cli-reference`: every command and flag.
   - `json-events`: the `exec --json` stream, every kind and its stability rule.
   - `environment`, `data-directory`, `exit-codes`.
5. **Embedding**
   - `embedding`: embedded mode, `AgentRuntime`, `AgentHostPorts`, the panic-hook bridge, shutdown.
   - `host-plugins`: writing plugins against `hanafubuki-agent-contracts`.
6. **Development**
   - `architecture`: crates and layers, the kernel, the plugin host, the turn flow, context, storage. From `architecture.md` and `spec.md`, rewritten.
   - `plugin-rules`: the plugin rules (from `rules/plugins.md`).
   - `building-testing`: toolchain, `cargo xtask` checks, the goldens and history corpus and their rules, CI.
   - `contributing`: the repository rules (pointing at AGENTS.md), commit style, twins.
   - `releasing`: from `release.md`, including rehearsing.
7. **Troubleshooting**
   - `troubleshooting`: `doctor`, common failures (keyring, data directory in use, Provider errors, MCP), logs and crash records, reporting a bug.

`extraction-plan.md` is finished history. It is not carried into the site. The two facts still in use, the twins list and the host interface, move to Development and Embedding. Git keeps the old file. (Decision 4.)

The Development pages remain the governing rules AGENTS.md points to: its document table, and the "read before changing" column, are re-pointed at the MDX pages.

### 3.4 Language (decision 3)

Hanafubuki's site is bilingual: zh-CN by default, with `x.mdx` plus `x.en.mdx`. Its `i18n-content.test.ts` requires every page in both. The given Giscus `data-lang` is zh-CN.
- **Recommended:** bilingual, zh-CN by default, with English. That is about 35 pages × 2.
- **Alternatives:**
  - English only: drop the i18n machinery and the content test.
  - Chinese only.

### 3.5 What moving `docs/` breaks, and the fixes

- **xtask `version` (`xtask/src/version.rs:286-301`):** it rewrites `hanafubuki-agent-* = "x.y"` snippets in README and `docs/*.md`. It is re-pointed at `README.md` + `site/content/docs/**/*.mdx`, and its test is updated.
- **Link updates:**
  - README (lines 17, 37-57);
  - AGENTS.md (the document table, and lines 83/92/102/164/170);
  - NOTICE line 25;
  - `architecture/notices-policy.toml`;
  - todo.md;
  - the comment in `release.yml`;
  - code comments citing `docs/…`;
  - `tui/.../paste_burst.rs:22`, which cites a nonexistent `docs/tui-chat-composer.md`.
- **Hanafubuki's links to these files:** D2 deletes them, and Hanafubuki links to them on GitHub:
  - its `AGENTS.md`: the Architecture table, the most-broken-contracts list and the content-logging pointer;
  - `docs/agent-integration.md`, which links `docs/rules/plugins.md`, `docs/architecture.md`, `docs/spec.md` and `AGENTS.md#agent-content-logging`.
  - D2b re-points them at the site's pages.
- **CI:**
  - `ci.yml` adds `site/**`, `package.json` and `bun.lock` to `paths-ignore`.
  - A new `site.yml` (push/PR touching `site/`) runs `bun install --frozen-lockfile`, `format:check:site`, `site:check`, `site:test` and `site:build`.

### 3.6 Deployment (decision 5)

The output is static (`site/.output/public`), and Hanafubuki has configs for Vercel, Netlify and Cloudflare.
- **Recommended:** GitHub Pages from `site.yml` on push to main (`VITE_BASE_PATH=/hanafubuki-agent/`; it needs Pages enabled in the repository). It runs in the same repository, needs no account, and Giscus `pathname` mapping works with the base path.
- **Alternative:** Netlify, like the Umami host, with a `netlify.toml` (`base = "site"`).

## 4. Phases (each validated, committed and pushed)

| Phase | Repo | Contents | Done when |
| --- | --- | --- | --- |
| T1 | Agent | model-api: thinking deltas on all three protocols, `reasoning.summary` on Responses. Kernel: the three events, `ConversationItem::Reasoning`, durations, headline removed. Facade re-exports. Leak tests rewritten; a new corpus fixture; request snapshots regenerated deliberately | 3-OS CI green; snapshot diff is only `"summary":"auto"` |
| T2 | Agent | TUI `ThinkingCell` (live collapsed/expanded, completed, replay, pager), the toggle key, Working without the headline; `exec` output and `docs/cli.md` | snapshots and bridge test; pty smoke |
| T3 | Hanafubuki | rev bump; Tauri coalescer; types; projection and transcript; `AgentThinkingBlock`; Working without the headline (and, if decision 2, without the thinking timer); i18n; tests; integration doc | full validation (frontend, Rust, goldens); visual check by you |
| D1 | Agent | the `site/` framework, tooling, branding, Giscus, Umami, `site.yml`, `ci.yml` paths, xtask `version` re-pointed | `site:check`, `site:test`, `site:build` green in CI |
| D2 | Agent | the content rewrite (§3.3, in the decision-3 languages); `docs/` removed; AGENTS.md, README, NOTICE, todo.md and comment links re-pointed | build green, every page prerendered, link check |
| D2b | Hanafubuki | its AGENTS.md and `docs/agent-integration.md` links re-pointed at the site | the links resolve |
| D3 | Agent | deployment (decision 5) | the site is live; a Giscus thread opens on a page; Umami records a view |

T1 → T2 → T3 is a strict order. D1 can run alongside T1/T2. D2 writes the thinking page last, so it describes what T2 built. The next Agent release (0.2.0) is yours, after T1–T2.

## 5. Decisions

1. Persist thinking in history, so it survives a reload or resume: **recommended**. The alternative is runtime only, where the block disappears on reload.
2. Remove Hanafubuki's separate "Thinking {duration}" timer from Working, since the block shows the time: **recommended**.
3. Documentation language: **bilingual, zh-CN by default** (recommended), English only, or Chinese only.
   - Bilingual is about 35 pages × 2 = about 70 MDX pages, by far the largest piece of work here.
   - The content test means no page can ship in one language only.
4. Drop `extraction-plan.md` from the site, keeping its live facts elsewhere: **recommended**.
5. Deployment: **GitHub Pages** (recommended) or Netlify.
6. Record one new history-corpus fixture with a reasoning item, using `HANAFUBUKI_AGENT_UPDATE_GOLDEN_HISTORY=1`, in both repositories: **recommended** (needed for decision 1).
7. Giscus theme and language: the pasted `data-theme="preferred_color_scheme"` and `data-lang="zh-CN"` are used as the starting values. Then, as on Hanafubuki's site, they follow the site's dark/light toggle and en/zh choice, sent by `postMessage`: **recommended**. Otherwise Giscus follows the OS theme and always shows Chinese.
