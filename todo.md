# Follow-ups

## Plan: Codex's new terminal interface, its commands, configuration in the interface (2026-11-09)

The plan is [docs/plan-codex-tui-port.md](docs/plan-codex-tui-port.md).
Every §7 decision was accepted as recommended. The source is Codex `57a808a392` (no stable tag has the new
interface). Every phase is validated, documented in both languages,
committed and pushed. Each phase notes what Hanafubuki must change when it
moves its pin.

- [x] P0a (TUI): `[[derived]]` in the notices policy takes `markers`,
      and `crates/tui/src` accepts "Modified from OpenAI Codex 57a808a392
      (Apache-2.0)." beside rust-v0.93.0's; NOTICE names both. The commands
      are one static table (`commands::COMMANDS`, `CommandSpec`: name,
      aliases, description, `inline_args`, `during_task`, `needs_session`,
      `dispatch`); `/quit` is an alias of `/exit`; `/name rest` is split
      and the rest goes to a command that takes it (none yet). A command's
      dispatch and its views (`bottom_pane::module_view`) run under
      `catch_panic`: one that panics is closed, reported, turned off for
      the run and recorded in `ChatWidget.warnings` (for `/warnings`). The
      bridge starts without a session; the first message (or a
      `needs_session` command, through `Op::EnsureSession`) creates it,
      and a failure (no Provider) is an error cell, the interface goes on;
      `/new` returns to no session; `-m` applies to every session the
      interface creates. The terminal interface page, "Slash commands".
  - Limits: the approval and question overlays are chat core, not
    wrapped; a contained panic's report still goes to standard error over
    the interface (as before, todo 5a); a session whose `-m` failed is
    left with no turn.
- [x] P0b (facade): `check-plugin-panics` also covers
      `crates/agent/src/plugins/` (no violation there);
      `hanafubuki_agent::panic::{catch_panic, catch_panic_async}`
      re-exported (the embedding page, "The panic-hook bridge").
  - Moved, as AGENTS.md adds nothing before its first user: the
    feature-call helper (`ModuleHost::service`, `PluginFaults::guard_async`,
    `AgentError::FeatureUnavailable`) comes with the first feature plugin
    (P7's worktree or P8's recap, whichever lands first); the `[tui]`
    section comes with its first keys in P2, and grows with each command.
  - Hanafubuki, at its next pin: nothing to change (two re-exports).
- [ ] P1: the provider screen (`/providers`, first-run setup when there is
      no default provider or the model is empty, `test_provider_draft`,
      request headers) and the MCP manager and form (`/mcp`, `/mcp verbose`,
      `/mcp login`, secret rows as credentials, OAuth loopback
      `mcp_oauth_login`, CLI `mcp login`, MCP ids validated on read).
  - [x] The facade's and the CLI's half: `test_provider_draft` (the same
        probe as `list_provider_models`, nothing saved); `mcp_set_credential`
        and `mcp_remove_credential` (atomic under the MCP configuration
        gate, so no revision); `McpCredentialKey::for_server_secret`
        (`<server>-<NAME>`); `mcp_oauth_login` (a loopback on
        `127.0.0.1:0`, 5 minutes, cancellable; derived from Codex's
        `perform_oauth_login.rs`); CLI `mcp login ID`, `mcp add --oauth`,
        `providers add --header NAME=VALUE`; a server id is checked on read
        (a bad one is dropped with a repair) and on write. Fixed on the way:
        a removed credential stayed in `mcp.toml` (the writer merges; removed
        keys are now written as null). Still: a credential entry the
        read-time repair drops stays in the file.
  - [ ] The interface's half: the screens and first-run setup.
  - Hanafubuki, at its next pin: new calls `test_provider_draft`,
    `mcp_set_credential`, `mcp_remove_credential`, `mcp_oauth_login`;
    re-exports `McpCredentialKey`, `McpOAuthPhase`, `McpOAuthPolicy`,
    `AgentProviderHeaderSettings`; `mcp_write_server` now refuses an
    invalid id; `mcp_write_credentials` really removes a dropped key. The
    goldens and the corpus are unchanged.
- [ ] P2: the new interface (Codex's owned transcript, layers A–G):
      terminal extensions, the `HistoryCell` extension, `TranscriptView`,
      the owned shell, the clipboard, keymap core; owned mode by default;
      Ctrl+T in both modes (live tail); `/tui`; `/raw`.
  - [x] P2.1, layers A–C: `tui/alternate_screen.rs` (enter and leave with
        the keyboard flags, optional mouse capture), mouse events as
        `AppEvent::Mouse`; `HistoryCell` gains `raw_lines` (every cell),
        presentations (compact, expanded, Rich/Raw), activity ids and
        disclosure, `copy_source`, animation ticks; `ChatWidget`'s live
        tail; `transcript_view/` (anchored positions, layout cache, live
        tail, follow-latest, scrolling, mouse and keyboard selection, F3
        search, F4 disclosure, footer). The Ctrl+T pager already shows a
        tool call's whole output. Not ported: hyperlinks and wrap
        provenance, copy mode (needs layer E), bookmarks, images.
  - [ ] P2.2: the owned shell (entries by `Arc`; per frame: live cells,
        render, follow control, footer; suppress `ChatWidget`'s own live
        cells), owned mode by default, Ctrl+T in both modes, `/tui`,
        `/raw`, the clipboard (`Copy` actions, `/copy` later), the first
        `[tui]` keys; remove P2.1's temporary `allow(dead_code)`s; the
        crate doc names both Codex revisions.
- [ ] P3: Esc to edit an earlier message (`app_backtrack`) over
      `fork_session_before_turn`; facade `edit_from_turn`.
- [ ] P4: `/rename /archive /delete /resume [id|title] /fork /clear /export
      /copy /ps /stop /pwd /init`; `export_session_markdown`; `cwd` and
      `branch` in session summaries.
- [ ] P5: `/theme` (syntect and two-face, also syntax highlighting),
      `/keymap`, `/title`, `/statusline`, `/presets`, `/config`,
      `/warnings`, `/memories`.
- [ ] P6: per-session permission mode (`set_session_permission_mode`,
      `turns.rs` no longer overrides it), `/permissions`, `/approve`
      (`approve_denied_action`, a model-visible note: goldens).
  - [x] The facade's half: `set_session_permission_mode` and
        `session_permission_mode` (runtime state beside the allowances; not
        persisted, forks and recoveries start with none; a running turn
        keeps the mode it started with); `approve_denied_action(session,
        review_id)` for one of the session's 10 latest denied reviews,
        between turns only: a `ContinuationContext` item (provider `kernel`,
        hook `approved-denied-action`) with Codex's approved-action wording,
        which the model and the next auto-review see (`user approval`
        evidence). No golden or corpus change; the note's text is pinned in
        `kernel/tests/loop_contract/approved_action.rs`. Permissions,
        "Session mode" and "Approving a denied action".
  - [ ] The interface's half: `/permissions`, `/approve` (the review ids of
        `PermissionReviewCompleted { Denied }`); `/status` shows the mode in
        effect (the session's, else `settings_view().settings.permission_mode`).
  - Hanafubuki, at its next pin: new calls `set_session_permission_mode`,
    `session_permission_mode`, `approve_denied_action`; kernel variant
    `PermissionReviewEvidenceRole::UserApproval`; the review prompt gained
    one sentence (no golden covers review requests). Differences from
    Codex: refused during a turn; a compaction drops older denials.
- [ ] P7: a mutable standalone workspace, `WorkspaceFsPolicy` rebuilt,
      `exec_command`'s default workdir (the `-C` bug), `/cd`;
      `hanafubuki-agent-worktree` and `plugin-worktree`, `/worktree`, the
      worktree choice in `/fork`.
  - [x] The facade's half: `AgentRuntime::set_working_directory` (standalone
        only; refused while a turn, a compaction or a background terminal
        runs; the filesystem policy is replaced in place and the commands'
        directory moves; live sessions stay); `exec_command` without
        `workdir` runs in the first workspace root, a relative `workdir`
        against it (the `-C` bug); no context note is needed (the
        `workspace_roots` section is rebuilt every turn).
        `hanafubuki-agent-worktree` (`<data>/worktrees/<repo>-<8 hex>`,
        git through the instance's `ExecutorManager`, the owner in the
        worktree's git metadata) and the `worktree` row; facade calls
        `worktree_create`, `worktree_list`, `worktree_bind`,
        `worktree_remove` (feature calls).
  - [ ] The interface's half: `/cd`, `/worktree`, the worktree choice in
        `/fork`.
  - Open: the project configuration layer (trust level, the project's
    `config.toml`) does not follow `/cd`; it stays the project read at
    start (switching it needs a new instance). The worktree service type
    lives in the plugin crate, not in contracts (plugin-rules says so).
  - Hanafubuki, at its next pin: `exec_command` without `workdir` now runs
    in the first workspace root as read at the instance's start (it ran in
    the process's directory); new crates and a `worktree` row (Capability);
    `AgentStoragePaths::worktrees_root`; `AgentExecutionService::
    set_working_directory`; `WorkspaceFsService::replace_policy`.
- [ ] P8: the shared structured request (from the titles plugin), the
      `recap` plugin, `/recap`, `suggest_session_title` in `/rename`.
  - [x] The facade's half: `plugins/structured_request.rs` (the titles
        plugin uses it, unchanged); the feature-call helper
        (`service/features.rs`: resolve the row's service per call, run it
        under `PluginFaults::guard_async`, `AgentError::FeatureUnavailable`
        when the row is down or the call panicked); the `recap` row
        (`RecapService`, the last 8 exchanges within 8192 estimated tokens,
        Codex's prompt) and the `titles` row's `TitleService`; the narrow
        port `AgentConversationReader`. Embedding, "Feature calls".
  - [ ] The interface's half: `/recap`, the suggestion in `/rename`.
  - Hanafubuki, at its next pin: `AgentError` is now exported and has
    `FeatureUnavailable { feature, reason }` (an exhaustive match needs an
    arm); new calls `recap_session`, `suggest_session_title`, type
    `AgentRecap`; a new plugin row `recap` (Capability, after `titles`;
    host rows now mount after it). Goldens and the history corpus are
    unchanged.
- [ ] P9: child and ephemeral sessions (`parent`, `origin`,
      `AgentSessionDriver`, archive and delete cascade; a new history
      fixture in both repositories), `/side` and `/btw`, the workspace
      panes.
- [ ] P10: the `subagents` plugin (Codex's v2 tools, orchestrator prompt:
      goldens), `/subagents`, `/agents`.
- [ ] P11: the `review` plugin (rubric, read-only child session, JSON
      findings), `/review`.
- [ ] P12: the `goal` plugin (tools, context, the first continuation hook,
      goal templates), `/goal` and its status-line item.
- [ ] P13: `hanafubuki-agent-hooks` and `plugin-hooks`, the
      `ToolLifecycleHookRegistry` extension point, hook events, `/hooks`.
- [ ] P14 (decision 4): `hanafubuki-agent-sandbox`, sandbox presets in
      `/permissions`, `/setup-default-sandbox` on Windows.

## Plan: visible thinking and the documentation site (2026-11-09)

The plan was `docs/plan-thinking-and-site.md` (removed once its
repository work was done; git keeps it); every recommendation in its §5 was accepted (store thinking in history; drop
Hanafubuki's separate thinking timer; a bilingual site, zh-CN by default;
`extraction-plan.md` not carried over; GitHub Pages; one new history-corpus
fixture recorded deliberately; Giscus follows the site's theme and locale).
The next release with it is 0.2.0 (the headline event is removed).

- [x] T1: thinking in the model API (three protocols, `reasoning.summary`)
      and the kernel (three events, `ConversationItem::Reasoning`, its own
      budget), the headline removed (the site's architecture page,
      "Thinking", and its exec --json events page, "Thinking"). Each item is cut
      at `AgentHardLimits::maximum_reasoning_text_bytes` (256 KiB) and no
      longer spends `maximum_model_output_bytes`; a response that stops
      early completes its open items. No request golden sets an effort, so
      none changed; the new corpus is `golden/history-reasoning/` with
      `recorded_reasoning_history_transcript.json`, and `golden/history/` is
      unchanged.
  - What T2 and T3 build on (serialized as the facade serializes them):
    - `AgentEventKind::ReasoningStarted { reasoning_id, sampling_step }` →
      `{"type":"reasoning_started","reasoningId":3,"samplingStep":1}`;
      `ReasoningDelta { reasoning_id, delta }` →
      `{"type":"reasoning_delta","reasoningId":3,"delta":"…"}` (Markdown,
      concatenated as is: a new summary part's delta starts with its blank
      line); `ReasoningCompleted { reasoning_id, duration_millis }` →
      `{"type":"reasoning_completed","reasoningId":3,"durationMillis":1200}`.
      Started → deltas → completed, all before the step's
      `history_committed`; an item may have no delta (a provider that sent
      no text). `AgentReasoningId` (a `u64`, unique in the session, drawn
      from the message counter) is re-exported by the facade. A turn that
      fails mid-stream sends no completion: end live blocks at the turn's
      terminal event.
    - `ConversationItem::Reasoning { text, duration_millis, sampling_step,
      truncated }` →
      `{"type":"reasoning","text":"…","durationMillis":1200,"samplingStep":1,"truncated":false}`,
      committed with the step's other items in the model's order (normally
      ahead of the step's tool calls and answer). It carries no reasoning id: a live
      block is replaced by the items of the `history_committed` that follows
      its completion. Exhaustive matches on `ConversationItem` need an arm.
    - Removed: `AgentEventKind::ReasoningHeadlineUpdated`
      (`reasoning_headline_updated`) and `AgentTurnSnapshot::reasoning_headline`
      (`reasoningHeadline`). `ReasoningUpdated` and `AgentReasoningSnapshot`
      (the turn's thinking time) are unchanged.
    - Model API: `ModelEvent::ReasoningSummaryDelta` is now
      `ReasoningDelta { item_id, kind: ModelReasoningTextKind, index, delta }`
      and `ModelOutputItem::Reasoning` has `content: Vec<String>` beside
      `summary`; a Responses request with an effort sends
      `"reasoning":{"effort":…,"summary":"auto"}`, which changes a host's
      request goldens only where they set an effort.
    - `AgentHardLimits` has `maximum_reasoning_text_bytes`: a struct
      literal without `..FIRST_BATCH_HARD_LIMITS` needs it.
      `history_committed`'s `itemCount` counts the step's reasoning items.
    - Test support: `reasoning_tool_events` and `reasoning_answer_events`
      (Responses streams whose reasoning summary streams).
  - Open: reasoning items count toward the history bounds
    (`maximum_history_bytes`, 4 MiB, and `maximum_history_items`, 1024, one
    more item per thinking step) but not toward the token estimate that
    triggers compaction. Compaction drops them, so a session that compacts is
    unaffected; a long session that thinks a lot and uses little context
    can now reach `LimitExceeded { "history bytes" }` sooner. Decide whether
    compaction also triggers on the history bounds, or whether a reasoning
    item's bytes leave `retained_bytes`.
- [x] T2: the TUI thinking cell (`history_cell/thinking.rs`), Ctrl-O,
      Working without the headline; `exec`'s `· thought for 12s` on
      standard error and `sessions show`'s `thought for 12s` line
      (the site's terminal interface page, "Thinking", and architecture
      page, "The terminal interface and the command line").
  - What T3 mirrors: collapsed by default; live `Thinking 5s` counting up
    from the cell's own start, completed `Thought for 12s` from the
    kernel's `durationMillis` (seconds, rounded down; `(truncated)` when
    the item says so); an empty text shows the header only; the turn's
    terminal event completes a live block with the time so far; one
    toggle for the live block and every later one; a reasoning update
    that arrives while a message streams is held until that message
    completes, so the transcript keeps the model's order. The live text
    is the streamed text; for an item that streamed both a summary and
    raw text the committed item (the summary) is what a reload shows.
  - Open (found in T2, not changed): `sessions show` prints its items
    latest first, in the page's order, where a person reads a transcript
    oldest first.
- [ ] T3 (Hanafubuki): the thinking block, Working without the headline or
      the thinking timer, the Task Center showing the configuration's
      recoveries.
  - Copy `crates/agent/src/tests/golden/history-reasoning/` (one session,
    `active/2026/11/09/session-2026-11-09T04-11-30-1.jsonl`) and
    `recorded_reasoning_history_transcript.json` byte for byte into the
    integration's `src/tests/golden/`, with the twin of
    `recorded_reasoning_history_reopens_and_continues` (`golden.rs`): it
    opens with both reasoning items' text and the next request carries
    none of it. The recorded prompt is "List the Skills." and its tool
    `skills_list`.
- [x] D1: the `site/` framework, tooling, Giscus, Umami, `site.yml`.
      Hanafubuki's site at its versions, without its home and download
      pages; a small home page; Giscus starts from the given attributes
      and follows the site's theme and language once its frame has loaded
      (its first message); `cargo xtask version` also scans
      `site/content/docs/**/*.mdx`. Under the Pages base, TanStack Start's
      static server functions fetched from the origin root, which broke
      client-side navigation: `patches/` prefixes Vite's base
      (AGENTS.md, "Site"). The router links with a trailing slash, so
      giscus's `pathname` term is the same for a page reached by URL or by
      in-site navigation.
- [x] D2: the content rewrite, bilingual; `docs/` removed; links re-pointed.
      Thirty-two pages in zh-CN and English, every heading with an
      explicit id shared by both languages; the Development section's
      `internals` page keeps the as-built detail of the old spec. AGENTS.md
      cites the pages' English sources, the README the live site, code
      comments and NOTICE `site/content/docs/<slug>.en.mdx`; `cargo xtask
      version` scans the README and the site only. A link to a heading on
      another page passes its fragment to the router apart, so the
      trailing slash goes before the `#`. Only the active plan is left in
      `docs/`.
  - Found while writing the docs (documented as current behaviour, likely
    bugs):
    - `-C` does not change the process's working directory, so
      `exec_command` without `workdir` runs where the binary started
      (`crates/execution/src/shell.rs:68`, `Path::new(".")`).
    - `mcp.toml` server ids are not validated on read.
    - `.git/` may show in `@` mention results.
    - Configuration warnings from the first read are lost for commands
      that do not start the Agent.
    - Every start of the terminal interface creates a session that
      `sessions list` shows (see Phase 5b below).
    - `skills/src/service/mutations.rs`: `source_folder_name`'s comment
      prefers the frontmatter name, the code the source folder's name.
    - Provider requests ignore `[network] proxy`: they read the process's
      proxy variables.
    - An unusable `config.toml` rewritten with defaults drops profiles,
      projects, `logLevel` and `[network]` from the live file (they are
      kept only in `.recovery/`).
    - The golden update variables trigger on any value
      (`var_os(..).is_some()`), `0` and the empty string included.
    - `check-plugin-panics` covers only `crates/plugin-*` (fixed in P0b).
    - `sessions show` lists its items latest first (see T2 above).
- [ ] D2b (Hanafubuki): its links into this repository's docs re-pointed.
- [x] D3: GitHub Pages. Pages is enabled (source: GitHub Actions) and
      `site.yml`'s `deploy` job publishes every push to main
      (`actions/upload-pages-artifact` 5.0.0, `actions/deploy-pages`
      5.0.1); the site is https://licyk.github.io/hanafubuki-agent/. On
      the live site a page reached by URL and by in-site navigation gives
      the same Giscus term (`hanafubuki-agent/docs/<slug>/`, one thread),
      and every page carries the Umami script with its website id.
- [x] 0.2.0 released (tag `v0.2.0` at `feba8a1`, release run
      42576971888): all 24 crates on crates.io, the four archives and their
      sums on the GitHub release.
- [ ] Yours: the visual checks of the thinking display in Hanafubuki and
      in the terminal interface on macOS and Windows terminals.

## Extraction from Hanafubuki

The phases were in the extraction plan (§14), finished and removed in D2
(git keeps it); Hanafubuki's `todo.md` tracks them with their done notes. Phases 1–3 are
done: the workspace, the decoupling inside Hanafubuki's tree, and the move.

- [x] Phase 4a: what the facade needs to run without a host: config
      layering (`AgentConfig::layered`, `ConfigLayers`, `-c` from Codex,
      `[network]`, `logLevel`, `AgentDirs`/`HANAFUBUKI_AGENT_HOME`), the
      data-dir lock (both modes), the standalone log file, the keyring
      store, the cwd workspace and `AgentRuntime::standalone`
      (the site's Configuration pages; architecture page, "The terminal
      interface and the command line").
- [x] Phase 4b: the CLI (`hanafubuki-agent-cli`, binary `hanafubuki-agent`,
      `clap`): the global flags, `exec` (text and JSON lines, `--approve`,
      questions on a terminal, exit codes, `-o`), `config path|get|set|edit`
      (`AgentConfig::value`, `set_user_value`, `layer_sources`), `providers`,
      `mcp`, `sessions`, `skills`, `memory`, `doctor`; the proxy in the
      Agent's processes' environment; the host-supplied identity
      (`AgentHostInstructions::identity`, `application`)
      (the site's command line reference page). Phase 4 is done:
      an `exec` turn against the scripted model runs end to end in CI
      (`crates/cli/tests/exec.rs`).
  - Deferred: `mcp add` sets no credentials (`credentialEnvironment`,
    `credentialHeaders`, OAuth login are edited in `mcp.toml`); `providers
    add` takes no request headers; `exec` resumes no session and attaches
    no images; `doctor` cannot say which process holds the data directory
    (the lock file names none); a management command that starts the Agent
    also starts its MCP servers in the background, as every start does;
    Ctrl-C during a command other than `exec` ends it without a shutdown
    (the lock goes with the process, the next start reaps its processes).
  - AGENTS.md discovery reads the working directory's and the Git root's
    own `AGENTS.md` (each a root), not the directories between them, since
    the runtime selects no target; Codex reads every level.
- [ ] Hanafubuki, when it moves its pin past Phase 4a: nothing to change for
      the lock (it is taken by the start marker); `AgentProxy` is now
      `hanafubuki_agent_config::AgentProxy`, re-exported where it was.
- [ ] Hanafubuki, when it moves its pin past Phase 4b:
  - `AgentHostInstructions` has two new fields (its struct literal in
    `hanafubuki-agent-integration/src/host.rs`, `application_instructions`,
    stops compiling): pass `identity: "You are the desktop application's
    coding and operations agent.".to_string()` and `application:
    "Hanafubuki".to_string()`, the strings the Agent used to have, and the
    integration goldens stay byte-identical (checked here by building the
    facade's goldens with those values); the history corpus is unchanged.
  - Hanafubuki's `docs/agent-integration.md`: the host instructions now
    carry the identity and the application name.
  - Nothing else: the facade's other changes are additions (re-exports of
    MCP, approval and user-input types, `probe_keyring`, the
    configuration's key API), and the CLI is a crate Hanafubuki does not
    depend on.
- [x] Phase 5a: the terminal interface's core (`hanafubuki-agent-tui`;
      the site's terminal interface and architecture pages): upstream ratatui
      0.30 and crossterm 0.29 (Codex's git forks dropped, the palette
      query too), the inline viewport and scrollback, the app loop, the
      chat widget and its history cells, markdown and streaming, the diff
      of a `PatchPreview`, the composer, the status line and key hints, the
      approval and question overlays, `agent_bridge.rs`; bare
      `hanafubuki-agent [PROMPT]` runs it on a terminal. NOTICE carries
      Codex's NOTICE and license and Ratatui's.
- [x] Phase 5b: the rest of the terminal interface (the site's terminal
      interface page, "Slash commands"): the slash commands with
      the composer's popup (`/model` from the Providers' model catalogs
      and `set_session_model`, `/mode`, `/skills`, `/new`, `/resume`,
      `/compact`, `/diff`, `/mention`, `/status`, `/mcp`, `/quit`,
      `/exit`), the transcript pager (Ctrl-T) and `/diff`'s on the
      alternate screen, the resume picker over the history store's session
      pages with the transcript replayed, `hanafubuki-agent resume
      [ID|--last] [PROMPT]`, `@` file mentions from Codex's `file-search`
      (in the TUI crate, on `nucleo` 0.5), and `cargo xtask check-notices`
      (`architecture/notices-policy.toml`). The facade gained
      `workspace_roots()` and re-exports `AgentCompactionStatus`. Phase 5
      is done: a pty run covers the popup,
      `/status`, a mention, a turn, Ctrl-T, `/diff`, `/model`, `resume`
      and `resume --last` against a fake Responses server.
  - Deferred from 5b: the permission mode changed from the interface (it
    is the configuration's; a per-session one is not in the facade);
    plan mode's "implement this plan?" prompt (switch back with `/mode`
    and ask); `/model`'s reasoning effort; the transcript pager's live
    tail of the running call and Codex's backtracking (Esc to edit an
    earlier message); a replayed tool call shows its name without the
    subject the live event carried (the history keeps the arguments,
    not the presentation); `/skills` lists the catalog, it does not
    toggle or insert a Skill; the sessions in the resume picker show no
    working directory or branch (the summaries have none); `exec`
    resumes no session; syntax highlighting (no `.tmTheme` is shipped).
  - Every start of the interface created a session (fixed in P0a: the
    first message creates it).
  - Still deferred from 5a: images in the composer (paste, attach); a
    session's earlier messages in the composer's history (only this
    run's); desktop notifications; Ctrl-Z suspend; the external editor;
    the time-worked separator counts from the status line's
    timer; a contained Agent panic writes its report to standard error,
    over the interface, as the Agent's hook does everywhere.
- [ ] Hanafubuki, when it moves its pin past Phase 5a: nothing to change.
      The facade gained re-exports only (`AgentTaskPlanSnapshot`,
      `AgentTaskPlanStep`, `AgentTaskPlanStepStatus`, `AgentToolPresentation`,
      `AgentUserInputOption`, `AgentUserInputQuestion`, `ApprovalAction`,
      `ApprovalActionKind`, `ToolOutput`, `ToolOutputContent`), and the TUI is
      a crate Hanafubuki does not depend on.
- [ ] Hanafubuki, when it moves its pin past Phase 5b: nothing to change.
      The facade gained one call, `AgentApplicationService::workspace_roots`
      (an addition over the host's `WorkspaceRootsPort`), and re-exports
      `AgentCompactionStatus`; the TUI and the
      CLI are crates Hanafubuki does not depend on, and nothing the model
      sees changed (the goldens and the history corpus are untouched).
- [x] Phase 6, the repository's part: clippy with `-D warnings` in CI
      (an `#[allow]` names its reason); `release.yml` (`plan`,
      `ci-result`, `checks`, `publish`, `licenses`, `binaries` for four
      targets, `github-release`), resumable by a manual run on the tag;
      the archives carry LICENSE, NOTICE and `THIRD-PARTY-LICENSES.txt`
      from `cargo-about` 0.9.2 (`about.toml`, `about.hbs`), which covers
      `nucleo`'s and the other MPL-2.0 crates; every crate but `xtask`
      packages and verifies (`cargo publish --workspace --dry-run`); the
      checklist is the site's releasing page.
- [ ] Phase 6, the user's step: `cargo xtask tag --push` on `main` for
      `v0.1.0` and watching the release run (about three and a half hours
      under crates.io's new-crate limit; resume with `gh workflow run
      release.yml --ref v0.1.0`). Done when the crates are on crates.io
      and the archives on the GitHub release.
- [ ] Hanafubuki, when it moves its pin past Phase 6: nothing to change.
      The clippy fixes boxed variants of internal enums only (the public
      `AgentModelContextBoundary` keeps its shape under an allow), and
      removed `ProcessOwnershipRegistry::len` and `ReservationSet::len`
      from `hanafubuki-agent-process`'s test support, which nothing
      called (its twin, `hanafubuki-process`, may keep them).
- [x] The configuration's recovery (Hanafubuki's open item from 2b): a
      repair whose original cannot be kept in `.recovery/` (or whose
      repaired file cannot be written) leaves the values in memory and the
      file read-only until restart, instead of failing the read; each
      repair is an `AgentConfigRecovery` (`AgentConfig::recoveries`), which
      `doctor` prints; a new file lists its keys in declared order. A hand
      edit is read at the next write or restart, by design (as Hanafubuki's
      Config and Codex).
- [ ] Hanafubuki, at its first pin past this (T3 does it): the recoveries in
      the Task Center's configuration recovery notice (the integration maps
      `AgentConfigRecovery` onto a `ConfigRecoveryReport` for the Agent's
      files); nothing else changes.

## Shared with the other repository

Twins: a fix to one is ported to the other by hand. Hanafubuki's `todo.md`
keeps the same list.

- `hanafubuki-agent-foundation` ↔ Hanafubuki's `hanafubuki-foundation`:
  `background_worker`, `blocking`, `cancellation`, `environment`,
  `json_normalization`, `module_faults`, `panic` (the host's hook takes a
  second containment check, the Agent's does not; the crash record's JSON is
  shared across the bridge: Hanafubuki's hook writes
  `data/agent/crash-record.json`, the Agent's start marker reads it),
  `paths`, `persistence`, `terminal_text`, `url_policy`, `user_agent`.
  Agent-only: `agent_settings`, `secrets`. Hanafubuki-only:
  `bounded_work_queue`, `concurrency`, `readiness`, `resolved`.
- `hanafubuki-agent-process` ↔ `hanafubuki-process`: `da_filter`,
  `executor`, `ownership`, `process_groups`, `terminal_size`,
  `test_support`, `tree`. Hanafubuki-only: `environment`.
- `hanafubuki-agent-host` ↔ `hanafubuki-module-contracts`: `error`,
  `fault`, `host` (without `RootServices::serve_bridge`), `logging`,
  `registry`, `restart`, `status`, `test_support`, `worker`.
  Hanafubuki-only: `bridge`.
- The integration goldens and history corpus (Hanafubuki's
  `hanafubuki-agent-integration/src/tests/golden/`) ↔ the facade's
  (`crates/agent/src/tests/golden/`): the history corpus is the same bytes in
  both; a history-format change re-records both.
