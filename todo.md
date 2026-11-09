# Follow-ups

## Plan: background compaction, hierarchical compression, a choice between them (2026-11-09)

The plan is [docs/plan-context-compaction.md](docs/plan-context-compaction.md).
Every §8 decision was accepted as recommended, except decision 12: the
previous plan stays and AGENTS.md allows one plan per piece of work. The
sources are Codex `57a808a392` and billion-context `4be21b08e` (acp-kernel
0.0.189). Each phase is validated, documented in both languages, committed
and pushed, and notes what Hanafubuki must change at its next pin.

- [x] P1: split `service/compaction/` along responsibilities (source and
      plan, a pure replacement build, install, status machine); no
      behavior change, goldens unchanged. Done: `source`, `replacement`,
      `status` beside `admission`, `prepare`, `execute`, `commit`;
      `commit_compaction` 75 lines. Nothing for Hanafubuki: kernel-internal,
      public API unchanged.
- [x] P2: the `[compaction]` setting (`strategy`, `background`,
      `hierarchical`), `AgentSessionConfig.compaction_strategy`, applied
      at turn admission, every transition; the `remoteCompaction` bug
      fixed; the Anthropic empty-`tools` case checked. Done: config
      `settings/compaction.rs` (per-field decoding, the summary-length
      pair checked once both are read), `/config` row, docs. The facade's
      `activate_provider_session` (every turn, `/compact`, plan
      implementation) applies the strategy (`set_session_compaction_strategy`)
      and `activate_model_session` takes a changed `remoteCompaction` into
      the session's `compaction_capability`; both only while idle, both
      persisted without a revision bump (no snapshot shows them, so a
      caller's revision read before the call stays current). Children copy
      the parent's, ephemeral ones run `standard`, `childSessions = false`
      gives a durable child `standard` (applied by the driver at creation
      and at each activation). Tests: the corpora decode as `standard`
      (`persistence_contract::session_config`), the bug's facade regression
      (`sessions::compaction`), the switch between turns. Goldens unchanged.
  - Open: the sub-tables reach no kernel code yet; P3/P7 carry what they
    need as a kernel type in `TurnExtensions` from `collect_turn_extensions`
    (as the sandbox reaches a turn). The `background.thresholdPercent`
    warning is a log line at creation and when the strategy changes.
    `agent_settings_fields()` is the web tools' form (rendered in
    Hanafubuki's Tools panel, flat patch keys), so `compaction` has no
    entry there: it is a nested table patched whole, like `memory`.
  - Open: Anthropic Messages with `LocalSummary` sends history `tool_use`
    / `tool_result` blocks with no `tools` key (`compaction_request` sets
    `tools: []`; `anthropic_messages/encode.rs` writes `tools` only when
    non-empty). The live API rejects such requests ("Requests which
    include `tool_use` or `tool_result` blocks must define tools"), from
    memory, not checked against the live API; the fake does not enforce
    it. Likely fix: send the tool specs with `tool_choice: none` on the
    local summary request (model-visible, regenerates the goldens), or
    have the Anthropic encoder declare the tools the history calls.
  - Hanafubuki, at its next pin: `AgentSettings.compaction`
    (`AgentCompactionSettings`: `strategy`, `background`, `hierarchical`)
    and `AgentSettingsPatch.compaction` (the table whole, strict); new
    re-exports `AgentCompactionSettings`, `AgentCompactionStrategy`,
    `AgentBackgroundCompactionSettings`,
    `AgentHierarchicalCompactionSettings`; a strategy select belongs with
    memory in the behavior panel, with `settings.fields.compaction*` keys
    of its own. `AgentSessionConfig.compaction_strategy`
    (`#[serde(default)]`, the corpus unchanged); kernel calls
    `set_session_compaction_strategy` and `session_compaction_strategy`.
    `activate_model_session` may now commit (persisting a changed remote
    compaction). Its own constructions of `AgentSessionConfig` add the
    field.
- [x] P3: background compaction in the kernel (slot, launch, adoption,
      `wait_for_pending`, the rebase install, cancellation, shutdown,
      usage, `AgentCompactionPhase::Background`). Done:
      `service/compaction/background/` (`mod`: the slot and the task
      registry; `launch`; `adopt`). The slot is `SessionRuntime.
      background_compaction`, never persisted and not an active operation;
      its run is shared by `Arc` (outcome behind its own lock) so a
      rolled-back commit cannot lose what the task wrote. The task is
      `tokio::spawn`ed and registered in `managed_background` (its own
      `Condvar`) before the record's lock is released; `stop_operations`
      cancels every slot and waits for the set, archive and delete cancel
      and wait for the session's tasks (5 s), a strategy change and a
      committed `remoteCompaction` change cancel. The threshold reaches a
      turn as `AgentBackgroundCompactionThreshold` in its extensions
      (`collect_turn_extensions`, one settings read with the sandbox).
      Launch: in `maybe_auto_compact` on the pre-turn step (with the step's
      estimate) and after each sampling request (with the provider's count
      only, no second estimate). Adoption: non-blocking before every
      automatic check, waiting (`wait_until`, woken by
      `request_turn_cancellation` advancing `terminal`) when a context-limit
      compaction is required and before `/compact`; one in-line pass when
      still over the threshold. `commit_compaction` takes `InstallMode::
      {Cas, Rebase}`. Usage is committed (no revision) when the run ends
      with a sample, so the install adds none and a discarded result is
      counted once. Tests: `compaction_contract/background/` (launch gates,
      a failed attempt, pre-turn and mid-turn adoption with work and world
      state after the snapshot plus a fork after it, the wait's success,
      failure, interrupt and still-over-threshold, `/compact` reuse and its
      manual fallback, a stale window refused at install, usage once and no
      turn budget, shutdown waiting, restart, archive, v1 and v2), and the
      facade golden `background_compacted_request` (HTTP fake, local
      summary, reopened from the JSONL history). `standard` goldens
      unchanged.
  - Deviations: the attempt key is (window, items before the current turn,
    model, implementation): a failed run is retried once another turn has
    completed, not only in a new window. The rebased world-state checkpoint
    is the state where the uncompacted part begins, not the live history's:
    the suffix's own updates (a removal included) must still apply, or
    validation refuses a stale removal. "Stale (model changed)" is covered
    by the window case and the strategy change: the kernel fakes cannot
    change a session's model without a real transport. No Codex code was
    copied, so no notices entry.
  - Open: the pre-turn in-line install has the stale-removal problem the
    rebase avoided: its checkpoint is the whole history's world state, so
    a current turn whose initial world-state update removes a section fails
    validation (`stale world-state removal`). Fixing it changes what a
    pre-turn install writes (a model-visible change, goldens to check).
  - Open: a turn admitted while `/compact` waits for a background run can
    take the result first; `/compact` then finds nothing and may fail with
    a stale revision (unguarded race).
  - Open: nothing observable tells a test that a turn is waiting on the
    background compaction, so the wait tests hold the compaction until the
    turn is admitted and assert the outcome (one compaction request, the
    background install, no in-line request after an interrupt) whichever
    checkpoint took it.
  - Hanafubuki, at its next pin: `AgentCompactionPhase::Background`
    (`phase: "background"` in `compaction_updated`, an appear);
    `AgentSessionRuntimeSnapshot.background_compaction`
    (`Option<AgentBackgroundCompactionState>`: `running`, `ready`;
    `#[serde(default)]`); new kernel types `AgentBackgroundCompactionThreshold`
    (a turn's extensions carry it from `collect_turn_extensions`; a host
    admitting turns through the kernel itself adds it) and
    `AgentBackgroundCompactionState`. Shutdown's stop step now also waits
    for background compactions. Golden to copy:
    `background_compacted_request.json`.
- [x] P4: background compaction in the interface and docs. Done: the
      terminal interface shows `Context compacted` from every
      `CompactionUpdated` that is `completed` (manual, pre-turn, mid-turn,
      new context, context error, background); `/compact`'s reply carries
      only a failure, so neither a manual compaction nor one `/compact` takes
      from the background shows twice; the cell waits behind a streaming
      message like tool updates. `/status` has a `Compaction` row from the
      snapshot it reads as it runs (`compacting in the background`, or ready
      and installed before the next request). Docs: json-events
      (`compaction_updated`'s `phase`, `reason`, `implementation`, `usage`;
      a background run's one `completed` event), terminal-interface (the
      cell, `/compact`, `/status`), internals (the bridge), architecture
      ("a third compaction path" dropped from "Not built": the three
      implementations and the background strategy are built; the Codex
      row). `with_input_tokens` moved into the facade's `test_support`.
      Tests: `app/tests/compaction.rs` (a background compaction installed
      in the turn that took it shows once, between the question and the
      answer, and once on resume; `/status` while it runs; `/compact` shows
      one cell, replayed once). Goldens unchanged.
  - Open: a compaction that fails during a turn shows nothing in the
    terminal interface (as before); only `/compact`'s failure is shown.
    The status line has no background item; `/status` is the only view.
  - Hanafubuki, at its next pin: the facade re-exports
    `AgentBackgroundCompactionState`; `test_support::with_input_tokens` is
    new. A host's compacted marker belongs on `compaction_updated` with
    `status: completed` (every phase, `background` included), and a status
    view reads `backgroundCompaction` from a fresh snapshot (it changes
    without a revision).
- [x] P5: `crates/compression` core (model, refs, boundaries, parse,
      project, protect, apply, prune, render) with the ported tests;
      notices and attribution. Pure (serde and serde_json only); 361
      ported tests; texts verbatim; tags never on tool-call arguments
      (`TextOnly`, `TextAndToolResults`, `None`); host predicates became
      data (`Message.protected`); `NOTICE`, the notices policy and the
      contributing page credit billion-context and acp-kernel.
  - Open (P5): recommend, nudge, the status line and `processTurn` are
    P6; `rebuild`, the decompress primitives and MEDIA REFS came with
    P9's crate half; absorb and rules with P10's crate half; timestamps
    are still open. The kernel's dependency row allows the crate
    in P7. No ledger `seq`, `base#part` ids or SKILL.md paths (the
    Agent has one item per message part).
  - Hanafubuki, at its next pin: a new published crate,
    `hanafubuki-agent-compression`; `NOTICE` gains billion-context.
- [x] P6: recommend, nudge, truncation, latches, the per-request
      pipeline, prompts (byte-exact golden), reports. Done: `recommend`,
      `nudge` (decision, low-water stamps, texts), `truncate` (latched
      emergency clips, terminal escape), `latch` (view latch), `turn`
      (`process_turn` in the kernel's step order), `prompts` (the five core
      tool schemas, `build_compress_system_prompt`, the rule texts and the
      opt-in language rule), `report` (`acp_status`, recap, status panel,
      `acp_cache` report and fold economics), `Config::validate`. 700 tests
      (315 new) with the kernel's `prompt-function-default.txt` byte-exact
      and eight more texts byte-exact against the kernel's own output.
  - Open (P6): timestamps are not in `process_turn` (relocation is not
    ported, plan §9). The content store, absorb, crush and filters came
    with P10's crate half, and the view latch's digest holds the settings
    of those that are on. Ported with P9's crate half: `open-objectives`' decompress
    assertion, the decompress primitives of `decompress-report`, the
    search and preview cases of `truncate-surrogates`. Not ported: host-set `dangerous`
    ranges, checkpoint carriers and media gaps, packs and section
    overrides, tokenizer injection, the text and hybrid protocols.
    Screening costs units × messages (release: 0.33 s at 10k messages,
    1.26 s at 20k, against the kernel's 0.57 s): measure with P8. Tool
    parameters are `serde_json::Value`, so their keys are sorted. The
    cache report shows times as UTC plus an offset the host passes; the
    kernel's fold-attribution offset quirk is kept; the per-tool listing
    sorts lowercase names instead of the locale's collation.
  - Hanafubuki, at its next pin: nothing to change; the compression crate
    gains public items only.
- [x] P7: `hierarchical` in the kernel (projection, the five core tools,
      the nudge, fallbacks, terminal escape, state), its goldens. Done:
      kernel `service/compression/` (`adapter`: history items to the
      crate's messages, ids `<source session>:<item>`, `CompactionSummary`,
      `ProviderCompaction` and a compacted prefix protected, the view back
      to items with tags, compacted carriers, clips and anchors as summary
      items; `tools`; `receipts`, ported from billion-context's host with
      the marker). `regular_model_request` projects while the strategy is
      `hierarchical` or the session has active blocks (`CompressionMode::
      {Off, Carry, Hierarchical}`); the static prompt after the Plan suffix,
      the nudge or status line as the last input and the five core tools
      only under `hierarchical`. Only `begin_sampling` keeps the state the
      projection returns (in memory, persisted by the next commit); the
      estimate and the post-install check build the same request and drop
      it. `PersistedAgentRuntimeState.compression` (`#[serde(default)]`,
      skipped when none, an unreadable one dropped with a warning); the
      runtime keeps it behind an `Arc`; fork at head copies it. Settings:
      the facade puts `AgentHierarchicalCompression` in each turn's
      extensions (P3's route); a turn without it, and `/compact`, use the
      defaults; no known window, no pressure. `compress`/`decompress` run on
      the record for their result and are applied again under the commit's
      lock (`commit_history_with`), so the state and its receipt are one
      commit and two calls of a step apply in order. Thresholds off under
      `hierarchical`; the context-error retry, `/compact`, `new_context` and
      the terminal escape (from the estimate's projection) run a standard
      compaction of the projected view; after it the blocks deactivate,
      and a session that switched away drops its state. A projection panic
      sends the full history and sets the turn's `compression_fallback`
      (standard for the rest of the turn); a tool panic is its error. The
      crate's state types derive `Eq` (the one change in
      `crates/compression`). Tests: `compaction_contract::hierarchical`
      (projection, tier 2, the read-only tools, refusals, the nudge, the
      threshold off, the context error, the terminal escape, panics,
      switching both ways, a restart), and the facade golden
      `hierarchical_requests` (nudge with prompt, tools and tags; after a
      fold; after a tier-2 distillation and a `decompress`; after the switch
      to `standard`). `standard` goldens unchanged; corpora decode.
  - Deviations: no warning event for a projection panic, only the log
    line (the event kinds come with P11). The receipts keep billion-context's
    texts but not its proxy's loop guards, breaker, recovery hints or
    degenerate-fold notes. `decompress` restores inline only (no `toFile`,
    range or image restore); `search_context` searches the block summaries
    (substring, as billion-context's host) without plan steering or the
    folded originals, with a fresh `DocCache` per call; `acp_cache` has one
    sample (the last response) and no cached tokens: the model API reports
    none. Budget pressure still compacts under `hierarchical`. A
    `[Compression FAILED`/refused receipt is an error output (`is_error`).
  - Open: the projection runs twice per step under `hierarchical` (the
    estimate and the request), and the state is persisted with every
    checkpoint: both measured in P8, see its deviations. Forks, children
    and an unreadable state: done in P8. The emergency voice nudges from
    `maxContextLimitPercent`; post-install `ContextBudgetExceeded` still
    applies to a terminal-escape compaction. The panic-containment test's
    debug-only overflows: replaced in P8 by a test-support marker. The
    architecture page's "Not built: a third compaction path" is P4's.
  - Hanafubuki, at its next pin: `PersistedAgentRuntimeState.compression`
    (its own constructions add `compression: None`); new kernel types
    `AgentCompressionState` (the crate's `State`) and
    `AgentHierarchicalCompression` (a turn admitted through the kernel itself
    adds it to the extensions for `[compaction.hierarchical]`);
    `AgentCoreTool` has five more variants, `ALL` is 8, `STANDARD` and
    `COMPRESSION` split it; `AgentToolCatalogContext.compression_tools`;
    `RESERVED_TOOL_NAMES` has 9 names (`compress`, `decompress`,
    `search_context`, `acp_status`, `acp_cache`). Golden to copy:
    `hierarchical_requests.json`. `NOTICE` names
    `crates/kernel/src/service/compression`.
- [x] P8: forks, resume, children, `rebuild`; state size; switching away.
      Done: kernel `service/compression/inherit.rs` (`inherited_state`). A
      fork before a turn (so `edit_from_turn`) and a child forked before
      its parent's running turn adopt the source's state through the
      crate's new `adopt_fork_state` (billion-context's
      `src/fork-adoption.ts`, `seedAllRefs` always on): a block whose
      covered messages are all present is kept active, with the blocks a
      tier block consumed as inactive records; expanded, consumed and
      straddling blocks are dropped; every present message keeps its ref
      and token snapshot; the parent's highest ref is reserved
      (`REF_FLOOR_ID`, a `by_raw` entry naming no message); the orphan,
      truncation, deferral and view latches carry over; the nudge, stats,
      terminal streak, absorbed records and rules start fresh. A child at
      its parent's head copies the state (as a fork at head does, so a
      `/side` runs `Carry`); an empty child has none. With no source
      state, and when a session is loaded without one (none written, or
      undecodable and dropped by `tolerant_compression_state`), the state
      is rebuilt from the window's `compress` calls (`rebuild_state`) with
      the source's last turn's tuning (the defaults at load) and the
      compress limits and recent zone relaxed; the crate's `rebuild_state`
      now skips a call whose receipt is `[Compression FAILED`/`refused`/
      `no-op` (a change to the ported function). A window holding a
      `CompactionSummary` or `ProviderCompaction` is never rebuilt; a
      window without `compress` calls gets no state (a `standard` fork
      stays `None`). The panic-containment test now panics through
      `test_support::COMPRESSION_PANIC_MARKER`: a view-latch digest the
      algorithm never writes, checked first inside the projection's and
      the tools' `catch_panic` closures under `cfg(any(test, feature =
      "test-support"))` (per session, no record field or global, nothing
      in production builds); the crate has no panic a malformed state
      reaches in release (its `u32` counters only wrap there). It passes
      in release. Tests: crate `fork` (7, ported from
      `fork-adoption-plan.test.ts`) and a `rebuild` case; kernel
      `compaction_contract::hierarchical::forks` (a fork before the second
      turn keeps `b1` and drops `b2`, keeps `m00001`/`m00002`, reserves the
      parent's highest ref and tags its own message above it, and shows
      the summary without its carrier; a fork before the fold turn keeps
      both; a lost parent state is rebuilt with the same blocks and refs,
      and a fork before the folds gets none; children at head and empty;
      a lost state rebuilt at restart, an undecodable one read as none);
      facade `tests::compression` (switching to `standard` end to end: the
      `compress` call stays as a carrier with tags and the summary, no
      prompt, tools or nudge, then a standard compaction ends the blocks;
      `edit_from_turn` after a fold keeps it). The kernel's hierarchical
      tests pass in release too. `standard` and `hierarchical`
      goldens unchanged (the golden's helpers moved to
      `tests/compression.rs`).
  - Measurements (release, a scratch harness in `crates/compression/tests`
    run with `cargo test --release -- --ignored --nocapture` and then
    removed: rounds of a short user message, three `read` calls with
    ~1.5 KB results and a ~1.6 KB answer, each round folded by a
    `compress` in the next): the state is about 89 B per message (the ref
    map both ways) plus about 560 B per block (ids and a 120-character
    summary): 42 KB at 500 messages and 49 blocks, 85 KB at 1 000/99,
    174 KB at 2 000/199, 353 KB at 4 000/399, 709 KB at 8 000/799.
    Counted in the kernel (a temporary trace in `conversation_commit`): a
    sampling step writes one to three full runtime checkpoints carrying
    the same state, and the state changes at every step (the request mints
    the new messages' refs); at a few blocks it is under a third of a record, at
    1 000 messages over 90 %. The rollout JSONL is append-only, so the
    state's share of a session's disk growth is O(steps × state).
    `process_turn` per call: folded 3.8 ms at 1 000 messages, 7.8 ms at
    2 000, 16 ms at 4 000, 40 ms at 8 000; unfolded 16 ms at 1 000,
    38 ms at 2 000, 175 ms at 5 000, 677 ms at 10 000, with or without
    the nudge's screening; cloning the state 0.2–2 ms. A step projects
    twice (its estimate and its request).
  - Deviations: the state is still written with every checkpoint. The
    measurement says it matters for long sessions, but writing it only in
    checkpoints after a change removes only the one-to-three duplicates
    per step (it changes every step) and leaves the O(steps × state)
    growth, while needing a tri-state field (absent = unchanged, none,
    a state), a rollback-safe marker in the kernel and the history store
    carrying the last state forward in `runtime_json`, materialization
    and lag catch-up: a persisted-format change across two crates, left
    for a decision (done in P8b, incrementally). The double projection is
    not fixed: at most ~80 ms a step at 8 000 folded messages, against the
    model's latency.
  - Open: the crate's per-call cost is
    superlinear in the window (677 ms at 10 000 unfolded messages, without
    the nudge too): profile `process_turn`. A child forked before its
    parent's running turn has no test of its own (it shares
    `inherited_state` with the fork before a turn). The rebuild at load
    uses the default tuning, so a session with `protectedTools` may number
    a replay differently; an approximation, as the crate's doc says.
  - Hanafubuki, at its next pin: nothing to change; persisted format and
    public kernel API unchanged. New public items: the compression
    crate's `adopt_fork_state` and `REF_FLOOR_ID`, and the kernel's
    `test_support::COMPRESSION_PANIC_MARKER`. Behavior: forks before a
    turn, `edit_from_turn` and children now carry the hierarchical state
    (a child at head, `/side` included, projects the parent's blocks), and
    a session whose state is missing or unreadable gets it back from its
    `compress` calls at load.
- [x] P8b: the hierarchical state persisted incrementally. Done: a chain
      of links (`kernel model/compression/chain.rs`), one in each runtime
      checkpoint that changed the state, as
      `PersistedAgentRuntimeState.compression_update`
      (`AgentCompressionUpdate`: `snapshot`, `delta`, `dropped`, and
      `unreadable` for one that cannot be read; `#[serde(default)]`,
      skipped when none, so `standard` records are unchanged). A delta
      (`AgentCompressionDelta`) carries its `seq` after the snapshot, the
      ref bindings minted or changed, the blocks made or changed other than
      in age (whole), the age of the blocks that only aged (`survived`),
      the token counts added and dropped, and the other fields whole when
      one changed. A snapshot writes the ref map one way (`by_raw`;
      `by_ref` derived on read, without `BLOCKED` and `REF_FLOOR_ID`): 15 %
      smaller. The kernel diffs against `SessionRuntime.compression_head`
      (`next_link` in `conversation_commit`): staged with the commit and
      restored with the runtime on rollback, so it is the last durable link
      once the commit settles (staged commits are serialized by
      `persisting_revision`). No head after a load, a fork or a child: the
      first link is a snapshot; so is a change a delta cannot carry (a
      binding or block gone, blocks reordered), and so is the link once
      the deltas since the snapshot would outweigh it. A checkpoint that
      changed nothing carries no link. The history store keeps the links
      as rows of `agent_compression_links` (a snapshot or `dropped`
      replaces the session's rows; deleted with the session), strips the
      state from `runtime_json`, and folds the rows
      (`AgentCompressionChain`) into `compression` when it loads a runtime
      (`load_runtime_state`, recovery); the index is schema 6, so an older
      index is rebuilt from the rollouts once, a record from before the
      chain reading as a snapshot. `EphemeralAgentHistoryStore` folds the
      same way. A chain that does not fold (no snapshot, a skipped `seq`,
      a delta naming a block or token count the state lacks, an unreadable
      link) loads as none, and P8's load-time rebuild replays the
      `compress` calls. Tests: kernel `model::compression::chain` (10: the
      first link a snapshot written one way, many steps folding back to the
      exact state with periodic snapshots, a delta carrying only the
      change, nothing for an unchanged state, snapshots for removals,
      `dropped`, broken chains repaired by the next snapshot, a delta for
      another state, the ref floor); `persistence_contract::
      compression_chain` (a failed commit rolls back the head: the next
      one is delta 1 and the store folds to the live state); facade
      `tests::compression` (a folded session over 14 turns: the rollout
      holds snapshots and more deltas, a restart reads back the exact
      state, so does a schema-5 index rebuilt from the rollouts, and with a
      delta row deleted the state is rebuilt from the `compress` calls; a
      fork from `edit_from_turn` reads back after a restart; a `standard`
      session writes no `compression` key and no row). Goldens unchanged.
  - Measurements (release, the P8 recipe as a temporary kernel test:
    rounds of ten messages, each folded by the next round's `compress`,
    `process_turn` per round, one step per round): the state P8 wrote
    whole into every checkpoint was 92 KB at 1 000 messages, 187 KB at
    2 000, 378 KB at 4 000, 759 KB at 8 000 (one-way refs: 78, 159, 321,
    645 KB); the link a step now writes once is 3.0 KB, 4.1 KB, 6.3 KB,
    10.7 KB, growing with the active blocks' ages (~13 B each; every block
    stays active in this recipe, the worst case). Over 500 steps the state
    took 117.7 MB of rollout per checkpoint copy before (×1–3 a step);
    now 4.9 MB in all, 23 snapshots and 477 deltas included (24× less
    before the per-step multiplier).
  - Open: the `survived` ages are O(active blocks) a step; a session that
    keeps hundreds of tier-1 blocks active pays ~13 B each per step.
  - Hanafubuki, at its next pin: `PersistedAgentRuntimeState.
    compression_update` (its constructions add `compression_update:
    None`); `compression` is now what a store loads (the folded chain)
    and what records written before the chain carry. New kernel types
    `AgentCompressionUpdate`, `AgentCompressionDelta`,
    `AgentCompressionChain` (a host store folds its links with it). The
    history index is schema 6 (`agent_compression_links`); an older index
    is rebuilt from the rollouts at the first open.
- [x] P9: decompress, search, media refs. Done (kernel half):
      `decompress` (kernel `service/compression/tools/decompress.rs`) has
      billion-context's host's four modes: the whole block inline (one tier
      up or `full`; an active block flagged restored inline, the re-fold
      hint), a range (`startId`/`endId`, the block's messages whose refs
      fall in it, read from the history, no state change), `toFile` (the
      text written to the session's export file, the output its path) and
      `imageRef` (`list`, an `mNNNNN` ref or a MEDIA REFS handle: the
      message's images attached to the output). `full` and range restores
      attach their messages' images too. The schema gains the host's
      `startId`/`endId`/`imageRef` and its image note
      (`core_tools/compression_spec.rs`, a derived file). Exports go through
      the new `AgentHistoryStore::write_session_export` (default: refused):
      the file store writes `history/attachments/<session>/exports/
      decompress-<block>[-full|-<start>-<end>].txt` (staged, published,
      replaced by name; the path rendered `native()`), so session deletion
      removes them with the attachments and the blob sweeps never see them;
      `execute_compression_tool` writes with the record's lock released,
      and the commit keeps that output (`CompressionToolRun.export`) while
      re-running the call only for its images and state. Restored images
      are the history's own `AgentImageAttachmentRef`s, written into the
      committed `ToolOutput.images` (`CompressionToolRun.images`), so a
      request loads them like any tool output's and a model without image
      input gets them stripped (an image restore is refused then).
      `search_context` (`tools/search.rs`) searches the summaries and then
      the folded originals (`folded_message_inputs` and `message_docs`),
      both `Substring` with minimum score 1e-9 as the host: one literal
      occurrence is a hit, none is no match (the hybrid's fuzzy n-grams make
      weak matches of garbage queries). Rendering is the host's: covered
      ref spans, the 【】-marked evidence snippet around the rarest term,
      the truncation note, the narrowing tip, the original-text section
      (one line per block no summary line listed, with its best message's
      ref). The `DocCache` is per session in `SessionRuntime.
      compression_search` (shared `Arc<parking_lot::Mutex>`, never
      persisted, 8 Mi units). MEDIA REFS: the adapter already passes a
      message's attachment ids as its image handles, so a fold lists them;
      a kernel test pins it and the re-access. `acp_cache` is unchanged: the
      model API's `ModelUsage` has `input_tokens` and `output_tokens` only.
      The crate's `Role::name` is public now. Tests: kernel
      `compaction_contract::hierarchical::tools` (range and its refusals;
      search over folded originals, summary hits first, no match; a folded
      image in MEDIA REFS, listed, restored by ref, by handle and by `full`,
      its pixels in the next request; `toFile` on an ephemeral store fails
      with the content inline), history `an_export_is_replaced_by_name_and_
      deleted_with_its_session`, facade `tests::compression::decompress_to_
      a_file_writes_under_the_session_and_goes_with_it` (real store: the
      path under the data directory in the output, the file's text, a
      `toFile` path ignored, removed by `delete_session_confirmed`). The
      kernel cannot see the data directory (the history crate depends on
      it), so the toFile contract is tested in the store and the facade.
      Golden `hierarchical_requests` regenerated on purpose: only
      `decompress`'s schema and description in `nudged_request` changed (its
      `b1` restore is unchanged); `standard` goldens unchanged.
  - Deviations from the host: a range is returned as the call's output
    (the host acks and injects the text into the next request) and needs no
    content store (the history holds the originals); `toFile` is a switch,
    not a path (writing where the model names would be an unapproved
    write), and the output says so; no automatic spill past 10 000
    characters (the request's tool-output cap bounds a large inline
    restore); images come back attached, not as files to `read`; no
    plan-aware steering (off by default in the host behind `search.
    planAware`, and the Agent has no such setting), no derived or foreign
    session search, no archived-block receipts.
  - Open: an export path under the data directory is readable by the
    `read` tool only where the data directory is not a protected root
    (standalone: none are). Images attached by a restore count toward the
    turn's tool-output byte budget; a `full` restore of a block with many
    images can exhaust it. The commit re-runs every `decompress` (only
    exports keep the first output), and a re-run's inline output bypasses
    `canonicalize_agent_tool_output` as in P7 (the request cap still
    applies).
  - For P10: `apply_retrieve`'s `Export { file_name, .. }` can go through
    `write_session_export` as `decompress`'s does (`ToolExport`); the
    content store's originals need files of their own (§4.6), which
    `attachments/<session>/` holds and deletion removes.
  - Crate half (before the kernel half): decompress, search, media,
    rebuild. `decompress` (`collect_block_content` one tier up or
    full, `mark_block_restored_inline`, `expand_block`, which also sets
    `expanded` since `sync_blocks` reactivates any other block, and the
    kernel's lineage helpers), `search` (`search_blocks` over `block_docs`
    and `message_docs`; hybrid, BM25, fuzzy, substring; a `DocCache` the
    caller keeps per session instead of process-wide state;
    `folded_message_inputs` gives the folded originals), `media` (MEDIA
    REFS appended to a block's summary at a fold and carried through
    merges and refolds; a message's handles are its `images`, inline
    `data:` and signed or expiring URLs excluded), `rebuild_state`
    (replays the `compress` calls; refs are minted with the view's
    protection, unlike the kernel, so a protected `compress` call takes no
    number and later refs stay where the model saw them). 88 new tests.
  - Unfaithful: no dictionary segmenter, so a CJK run of two or more
    characters is its bigrams and a one-character run itself (the
    kernel's ICU words and its all-OOV singles are not reproduced; the
    tests pinning ICU's words, `试验证明` against `验证`, and the `缓存`
    rescue's fuzzy-only cap are not ported); bigrams are of characters,
    not UTF-16 units; a system message weighs as a tool's. Not ported:
    the algorithm registry, semantic and asynchronous search, the V8
    160K-document case, media pinning (no message is unrecoverable: the
    host keeps the full history).
  - Hanafubuki, at its next pin: `AgentHistoryStore::write_session_export`
    (a default that refuses; a host store that keeps files implements it
    to make `toFile` work, and must remove the files with the session).
    If Hanafubuki's `protected_roots()` covers its app data directory (the
    Agent's test support models a host so), the model cannot `read` an
    export there: check at the pin, and allow the session's `exports/` for
    reads or expect `toFile` to be useless. The
    `decompress` schema changed (model-visible): golden to copy,
    `hierarchical_requests.json`. The compression crate's `Role::name` is
    public. `NOTICE` names `crates/kernel/src/core_tools/compression_spec.rs`.
- [ ] P10: the opt-in features (absorb, content store, crush, rules,
      filter), off by default.
  - Crate half done; the kernel half (tools, the content store's files,
    settings) lands after P7. `absorb` (the `[ACP absorb]` demand, the
    candidate rules, `apply_absorb`, `parse_absorb_input`, hiding the
    pair, the system-prompt section), `store` (`ContentStore`: plain data,
    SHA-256-keyed texts and per-ref entries, persisted by the host apart
    from the state; arrival-time placeholders, re-projected byte for byte
    when raw bytes come back; `store_covered_originals` for folds;
    `apply_retrieve` answers inline or `Export { file_name, .. }`, after
    which the host writes the file and builds the pointer with the path
    it chose), `crush` (json-fold, code-trim, log-select; the two-tier gate
    `evaluate_tool_result`), `rules` (`acp_rule` records with
    `execute_rule`, the host's one-call API, and the prompt section),
    `filter` (a `MessageFilter` trait the host passes per call; no
    built-ins), the three tool schemas verbatim (`acp_rule`'s is
    billion-context's host's). `process_turn_with(input, TurnExtras {
    content_store, filters })` returns `(TurnOutput, ContentStore)`;
    `process_turn` is unchanged. `NudgeState::absorb_aborted_turn_growth`
    books an interrupted turn (#5415's host side). `Config` gains
    `absorb`, `crush`, `content_store`, `rules`, `message_filters`;
    `State` gains `absorbed`, `rules`, `next_rule_id`; `Stats` gains
    `absorbed_tokens`, `stored_count`, `retrieval_count`. New
    dependencies `regex` and `sha2`, and serde_json's `float_roundtrip`
    (unified across the workspace): crush writes numbers as JavaScript
    does, which needs them parsed exactly. 130 new tests, one a golden
    recorded before the port that pins that defaults change no turn; crush was
    checked byte for byte against the kernel's own code over a generated
    corpus (783 inputs).
  - Unfaithful: tool names are fixed (`absorb`, `acp_retrieve`), not
    settings; a target an active block covers is refused as not visible
    (the host passes the full history); tokens are the crate's estimate
    throughout; crush keeps an object's keys in document order where
    JavaScript moves integer-like keys first. Kept from the kernel: its
    code scanners never leave a string once a quote opens one, and its
    log selection keeps lines in the order they were chosen. Not ported:
    the crush and filter registries (and their plugin-contract tests),
    `restampContentStore` and `ccr-restamp` (no ledger restamp), the
    legacy `acp_retrieved_` injections, the status report's store counters,
    absorb's stale-flag age (the host's).
  - For the kernel half: offer `absorb_tool`/`retrieve_tool`/`rule_tool`
    only while each feature is on, with `build_absorb_system_prompt`,
    `RULES_USAGE_PROMPT` and `format_rules_for_prompt` in the
    instructions; persist `ContentStore` beside the state; call
    `store_covered_originals` after a fold and `note_retrieval` after a
    hit.
  - Hanafubuki, at its next pin: nothing to change; the compression crate
    gains public items and fields (`Config`, `State` and `Stats` are built
    with `Config::new` and `Default`).
- [ ] P11: `/compression`, status line, `context_compressed`, docs,
      attribution; pty smoke.

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
- [x] P1: the provider screen (`/providers`, first-run setup when there is
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
  - [x] The interface's half: `/providers` (list with add, edit, delete,
        enable, default; a form with masked token and header rows; Test on
        the draft; after a save, the model picker writing the model policy)
        and first-run setup (no default Provider, or no model and no `-m`:
        the setup opens and a message waits for it); `/mcp` (manager: add,
        edit, delete, enable, connect, refresh, OAuth login and logout;
        stdio or Streamable HTTP; public or secret rows, a secret stored
        as a credential; Advanced fields), `/mcp verbose`, `/mcp login ID`.
        The form view (`bottom_pane/form/`, `textarea/single_line.rs`).
        Not ported: `SelectionSecondaryAction` (the screens handle their
        keys); no browser is opened for OAuth (the URL is shown).
  - Hanafubuki, at its next pin: re-exports `McpApprovalPolicy`,
    `McpExposurePolicy`, `AgentProviderSettings`.
  - Hanafubuki, at its next pin: new calls `test_provider_draft`,
    `mcp_set_credential`, `mcp_remove_credential`, `mcp_oauth_login`;
    re-exports `McpCredentialKey`, `McpOAuthPhase`, `McpOAuthPolicy`,
    `AgentProviderHeaderSettings`; `mcp_write_server` now refuses an
    invalid id; `mcp_write_credentials` really removes a dropped key. The
    goldens and the corpus are unchanged.
- [x] P2: the new interface (Codex's owned transcript, layers A–G):
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
  - [x] P2.2: fullscreen (owned) when `tui.fullscreenTranscript` and the
        alternate screen is allowed (`--no-alt-screen`,
        `tui.alternateScreen` auto|always|never, auto off in any SSH
        session: stricter than Codex); cells kept as `Arc`s and drawn by
        the view, `ChatWidget` stops drawing its live cells; older pages
        read on demand (`read_transcript_page`, spliced after the header);
        Ctrl+T toggles detail in place, or in scrollback mode opens
        `pager_overlay::TranscriptOverlay` (live tail, scrollback held
        until close); `/tui`; `/raw [on|off]`; `clipboard.rs` (arboard,
        OSC 52 fallback; `BSL-1.0` accepted in `about.toml` for arboard's
        Windows dependencies); `[tui]` typed (`AgentTuiPreferences`,
        facade `tui_preferences` / `write_tui_preferences`, no revision,
        like `write_settings`).
  - Open from P2: a tool call whose output was on the newer page shows
    without its output (5b); select mode has no default key (`/keymap`,
    P5); `app/mod.rs` is near the file limit.
  - Hanafubuki, at its next pin: `AgentStandaloneSettings` has a new
    public field `tui` (a struct literal needs it); re-exports
    `AgentTuiPreferences`, `AgentTuiPreferencesPatch`,
    `AgentAlternateScreen`.
- [x] P3: Esc to edit an earlier message (`app_backtrack`) over
      `fork_session_before_turn`; facade `edit_from_turn`.
  - Done: `edit_from_turn(session, turn)` forks before the turn and
    archives the original (refused while a turn runs, for an archived,
    child or ephemeral session); `app/backtrack.rs` (prime, browse, edit,
    in place in fullscreen and in the transcript overlay in scrollback);
    `agent_bridge/edit.rs` maps a prompt, counted from the latest, to its
    turn by matching the saved messages (a steering message is refused).
    Terminal interface, "Edit"; sessions, "Edit"; embedding,
    "edit_from_turn".
  - Open: browsing does not start from an open Ctrl+T view; with no
    prompt loaded in fullscreen, the second Esc does not load older
    pages; images are not restored to the composer (it has none).
  - Hanafubuki, at its next pin: the call `edit_from_turn`, the re-export
    `AgentEditFromTurn`.
- [x] P4: `/rename /archive /delete /resume [id|title] /fork /clear /export
      /copy /ps /stop /pwd /init`; `export_session_markdown`; `cwd` and
      `branch` in session summaries.
  - Done: one call family (`AgentCommand::Session`, `agent_bridge/
    sessions.rs`); `/rename` with the suggestion (Tab takes it); `/archive`
    and `/delete` confirmed, then no session; `/resume ID|TITLE`; the
    picker's Branch and Directory columns, Tab for the archived list,
    Ctrl+A to archive or restore; `/fork [name]`; `/clear [name]`;
    `/export [path]` (copy or save; never overwrites) over the facade's
    `export_session_markdown` (Codex's layout, thinking left out);
    `/copy` (the answer, code blocks, quotes; owned mode: select);
    `/ps`; `/stop` and `/clean`; `/pwd` and `/cwd`; `/init` (Codex's
    prompt); `/recap`. Terminal interface, "Session commands"; sessions,
    "Export".
  - Open: `/rename`, `/archive`, `/resume` are off during a turn (Codex
    allows them); `/resume TITLE` and the picker's archive run on the
    interface thread; the export is written in place (no temp file);
    the picker has no Ctrl+T preview.
  - Hanafubuki, at its next pin: the call `export_session_markdown`.
- [x] P5: `/theme` (syntect and two-face, also syntax highlighting),
      `/keymap`, `/title`, `/statusline`, `/presets`, `/config`,
      `/warnings`, `/memories`.
  - [x] P5a: highlighting of fenced code and diffs (syntect 5.3, two-face
        0.5 with Oniguruma; the grammars' and themes' licences in
        `crates/tui/syntax-licenses.txt`, appended to the release's
        `THIRD-PARTY-LICENSES.txt`); `/theme` (bundled and
        `<data>/themes/*.tmTheme`, live preview; default `ansi`);
        `/statusline` (model, reasoning, directory, branch, context,
        tokens, session id and title, task progress, permission mode,
        goal) and `/title` (OSC 0, sanitized, the title stack pushed and
        popped); `tui.theme`, `tui.statusLine`, `tui.statusLineColors`,
        `tui.terminalTitle`. Not ported: Codex's model-named themes, a
        background query for the default theme, a streaming highlighter.
  - [x] P5b: the keymap (`crates/tui/src/keymap/`: contexts global,
        chat, composer, editor, transcript, pager; context → global →
        defaults; chords; leader Ctrl-X; conflicts refused; `tui.keymap`)
        and `/keymap [debug]`; `/presets` (`tui.presets` slots 0–9, run
        through the slash parser, "from current settings" with
        `/permissions`, `/title`, `/statusline`); `/config`; `/warnings`
        (the configuration's layer warnings and recoveries, plugins, MCP,
        hooks, panics, the interface's own; "N warnings: /warnings" at
        start, which closes the D2 item on lost first-read warnings);
        `/memories`. The `TextArea::input` module-size exception is gone.
  - Open (P5b): popup, list, form, select-mode, search and browsing keys
    stay fixed as in Codex; `config set tui.keymap.<context>` is refused
    (only the whole table); a slot a profile or `-c` overrides is
    reported after the save, not refused.
  - Hanafubuki, at its next pin: `AgentTuiPreferences` has `keymap` and
    `presets` (struct literals); new `AgentTuiPreset`,
    `AgentConfig::layer_warnings`; facade `config_recoveries`,
    `config_layer_warnings`; re-exports `AgentMemorySettings`,
    `AgentMemoryStatus`, `AgentMemoryContributionMode`.
- [x] P6: per-session permission mode (`set_session_permission_mode`,
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
  - [x] The interface's half: `/permissions [mode]` (full access asks
        first; the session only; `/status` and the status line show the
        mode in effect); `/approve` (the session's latest ten denied
        reviews, searchable; the retry is still reviewed).
  - Hanafubuki, at its next pin: the re-export `PermissionReviewStatus`;
    new calls `set_session_permission_mode`,
    `session_permission_mode`, `approve_denied_action`; kernel variant
    `PermissionReviewEvidenceRole::UserApproval`; the review prompt gained
    one sentence (no golden covers review requests). Differences from
    Codex: refused during a turn; a compaction drops older denials.
- [x] P7: a mutable standalone workspace, `WorkspaceFsPolicy` rebuilt,
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
  - [x] The interface's half: `/cd [path]` (says the project
        configuration stays the one read at start); `/worktree` (continue
        this conversation, start a new one, browse: resume, copy, remove);
        `/fork` asks "here" or "into a new worktree" in a Git repository.
        `hanafubuki_agent_tui::run` takes `&Arc<AgentRuntime>`.
  - Open: the project configuration layer (trust level, the project's
    `config.toml`) does not follow `/cd`; it stays the project read at
    start (switching it needs a new instance). The worktree service type
    lives in the plugin crate, not in contracts (plugin-rules says so).
  - Hanafubuki, at its next pin: `exec_command` without `workdir` now runs
    in the first workspace root as read at the instance's start (it ran in
    the process's directory); new crates and a `worktree` row (Capability);
    `AgentStoragePaths::worktrees_root`; `AgentExecutionService::
    set_working_directory`; `WorkspaceFsService::replace_policy`.
- [x] P8: the shared structured request (from the titles plugin), the
      `recap` plugin, `/recap`, `suggest_session_title` in `/rename`.
  - [x] The facade's half: `plugins/structured_request.rs` (the titles
        plugin uses it, unchanged); the feature-call helper
        (`service/features.rs`: resolve the row's service per call, run it
        under `PluginFaults::guard_async`, `AgentError::FeatureUnavailable`
        when the row is down or the call panicked); the `recap` row
        (`RecapService`, the last 8 exchanges within 8192 estimated tokens,
        Codex's prompt) and the `titles` row's `TitleService`; the narrow
        port `AgentConversationReader`. Embedding, "Feature calls".
  - [x] The interface's half: `/recap`, the suggestion in `/rename` (P4).
  - Hanafubuki, at its next pin: `AgentError` is now exported and has
    `FeatureUnavailable { feature, reason }` (an exhaustive match needs an
    arm); new calls `recap_session`, `suggest_session_title`, type
    `AgentRecap`; a new plugin row `recap` (Capability, after `titles`;
    host rows now mount after it). Goldens and the history corpus are
    unchanged.
- [x] P9: child and ephemeral sessions (`parent`, `origin`,
      `AgentSessionDriver`, archive and delete cascade; a new history
      fixture in both repositories), `/side` and `/btw`, the workspace
      panes.
  - [x] The facade's half: lineage (`parent_session_id`, `origin` Side |
        Subagent | Review, `cwd`, `branch` from `.git/HEAD`) in the
        persisted runtime state (optional fields, format version unchanged)
        and in `AgentSessionSummary`; `AgentSessionListRequest.parent`
        (`TopLevel` by default, so every list now hides children, or
        `ChildrenOf(id)`), cursor version 2, index schema 5 (an older
        index is rebuilt from the rollouts once); ephemeral sessions in an
        in-memory store behind `SessionStores` (never listed or
        recovered); kernel `create_child_session`,
        `close_ephemeral_session`, `ephemeral_children`;
        `AgentSessionDriver` (`session_driver.rs`; `run_turn` and
        `last_answer` wait for P10); `open_side_conversation` and
        `close_side_conversation` (Codex's side policy and boundary as
        `ContinuationContext` items); archive and delete cascade over
        persisted descendants (a refused deletion deletes nothing). New
        corpus `golden/history-lineage/` with
        `recorded_lineage_history_transcript.json`, and the request golden
        `side_conversation_request.json`.
  - [x] The interface's half: `/side [message|close]` and `/btw` (a
        second conversation: docked on the right in fullscreen at 100
        columns or more, with a divider, zoom and the leader's pane keys;
        a switch with a banner in scrollback; Codex's command scope; Esc
        returns); the resume picker's cwd and branch came in P4. No tab
        strip (one side conversation per session).
  - Open: a compacting parent refuses `/side`; no `/side` before a first
    turn; restore does not cascade; a child does not inherit the
    session's permission mode (P10 passes it).
  - Hanafubuki, at its next pin: copy `history-lineage/`,
    `recorded_lineage_history_transcript.json` and
    `side_conversation_request.json` byte for byte, with the twin of
    `recorded_lineage_history_reopens_and_continues`; every session list
    without `parent` now hides children; the index reindexes once (schema
    5); kernel signatures changed: `create_model_session` and
    `create_unconfigured_session` take an `AgentSessionWorkspace`,
    `prepare_session_deletion` takes `descendants`,
    `PreparedAgentSessionDeletion` holds a list and is not `Copy`; new
    facade calls `open_side_conversation`, `close_side_conversation`;
    re-exports `AgentSessionOrigin`, `AgentSessionParentFilter`.
- [x] P10: the `subagents` plugin (Codex's v2 tools, orchestrator prompt:
      goldens), `/subagents`, `/agents`.
  - [x] The facade's half: the `subagents` row (after `goal`): tools
        `spawn_agent`, `send_message`, `followup_task`, `wait_agent`,
        `list_agents`, `interrupt_agent` over `AgentSessionDriver` (Plan
        mode: `wait_agent`, `list_agents`); children are durable sessions
        of origin `Subagent` with the parent's effective permission mode;
        depth 1, at most 3 running per parent, waits up to 5 minutes; the
        `subagent_guidance` prompt fragment (the tail of Codex's
        orchestrator.md) for top-level Default-mode sessions; facade
        `subagents(parent)`, `interrupt_subagent`. Ten request goldens
        regenerated (Default 30 tools, Plan 15); corpora unchanged.
  - [x] The interface's half: `/subagents` (status cell, a live picker,
        showing a child with its journal followed and its turn adopted);
        a child's pending approval shown in the parent ("Subagent NAME:")
        and answered on the child; `/agents` (sessions with their children
        nested, filters, open, new, fork, archive or restore, delete,
        interrupt).
  - Fixed after P14: an interrupt that lands while the tool catalog is
    built ends the child's turn Cancelled (it was Failed).
  - Open: `wait_agent` pollutes memory
    contribution, so a coordinating session never contributes; archiving
    a parent while a child runs is untested.
  - Hanafubuki, at its next pin: re-exports `AgentSubagentStatus`,
    `AgentSubagentState`; calls `subagents`, `interrupt_subagent`; a
    `subagents` row; every request golden changes again.
- [x] P11: the `review` plugin (rubric, read-only child session, JSON
      findings), `/review`.
  - [x] The facade's half: the `review` row (after `subagents`); a review
        is a durable child of origin `Review` (empty, Default mode) whose
        first message is Codex's request for the target (uncommitted
        changes, base branch with its merge base, commit, custom); the
        rubric is a prompt fragment for review sessions only; the
        `review.read-only` approval rule denies every action that needs
        approval in a review session except `git --no-pager
        status|diff|show|log|blame|rev-parse|merge-base|ls-files` with
        plain arguments (contracts: `ApprovalRule::evaluate` takes the
        session's origin; `AgentSessionOrigin` moved to contracts); the
        review's turn runs in FullAccess; the answer is read tolerantly;
        the parent gets Codex's exit message as context. Facade
        `start_review`, `cancel_review`, `review_targets` (git queries in
        `crates/worktree/src/queries.rs`); `AgentReviewTarget` is a strict
        IPC request. New golden `review_request.json`; no other changed.
  - [x] The interface's half: `/review [instructions|cancel]` (a picker
        of targets; refused before a first message; a "Reviewing" status
        and Esc to cancel; the review cell with the verdict and findings;
        messages sent meanwhile wait for the findings).
  - Open: read-only tools, allowed MCP tools and the Agent's own tools
    still run in a review; `request_user_input` could stall one; no
    `review_model`; a failed review is reported as interrupted; with the
    plugin off, an earlier review session is no longer kept read-only.
  - Hanafubuki, at its next pin: every `ApprovalRule` implementor takes
    `origin`, and so does `ApprovalRuleSnapshot::verdict`; new calls
    `start_review`, `cancel_review`, `review_targets` and their types; a
    `review` row; kernel `add_continuation_context`; copy
    `review_request.json` byte for byte.
- [x] P12: the `goal` plugin (tools, context, the first continuation hook,
      goal templates), `/goal` and its status-line item.
  - [x] The facade's half: `AgentGoal` (objective ≤ 4,000 characters,
        status active | paused | blocked | budget_limited | complete,
        origin, token and time budgets, usage) as an optional field `goal`
        of the persisted runtime state (a fork copies it; children and
        ephemeral sessions never have one); kernel `session_goal`,
        `update_session_goal` (compare-and-swap); the `goal` row: tools
        `get_goal`, `create_goal`, `update_goal` (Plan mode: `get_goal`
        only), a `goal` world-state section, and the continuation hook
        `goal/continue` (Codex's continuation, budget-limit and
        objective-updated templates, within the same turn); facade `goal`,
        `set_goal`, `clear_goal`, `pause_goal`, `resume_goal`; event
        `session_goal_updated`. Ten request goldens regenerated (the three
        tools); new corpus `golden/history-goal/` with
        `recorded_goal_history_transcript.json`. `activity.rs`'s ceiling
        644. Plugin rules' tool counts corrected (they were already off:
        Default 24, Plan 13).
  - [x] The interface's half: `/goal [objective [--tokens N]|clear|pause|
        resume|edit]` (a menu with usage and budgets; "Replace goal?"),
        the status line's goal item; setting or resuming starts no turn.
  - Open: continuation is within the turn (Codex starts a new turn), so a
    goal without a budget is bounded by a turn's limits; `resume_goal`
    starts no turn; `/goal edit` (keeping usage) needs an edit path;
    tokens count cached input.
  - Hanafubuki, at its next pin: copy `history-goal/`,
    `recorded_goal_history_transcript.json` and the ten regenerated
    request goldens byte for byte, with the twin of
    `recorded_goal_history_reopens_and_continues`; `PersistedAgentRuntime
    State` struct literals need `goal`; exhaustive event matches need
    `SessionGoalUpdated`; a `goal` row after `recap`.
- [x] P13: `hanafubuki-agent-hooks` and `plugin-hooks`, the
      `ToolLifecycleHookRegistry` extension point, hook events, `/hooks`.
  - [x] The facade's half: `crates/hooks` (Codex's `[hooks]` and
        `hooks.json` from the home and a trusted project; tolerant;
        matchers, with `Bash`/`Write`/`Edit` aliases; command handlers
        through the executor; trust by sha256 of the definition and
        enablement in `<data>/hooks-state.json`); `crates/plugin-hooks`;
        `ToolLifecycleHookRegistry` in contracts (deny-only before a call,
        context after the step's outputs, fail-open); Stop as the first
        continuation hook; SessionStart and UserPromptSubmit as a
        `hook_context` section; `AgentEventKind::Hook{Started,Completed}`;
        facade `hooks_list`, `set_hook_enabled`, `trust_hooks`. `[hooks]`
        is no longer reported as an unknown configuration key. No golden
        changed; `kernel/src/activity.rs`'s ceiling rose from 641 to 643
        (its exhaustive event match).
  - [x] The interface's half: `/hooks` (events, hooks with trust and
        on/off, toggle, trust, trust all; the configuration's skipped
        items); at start, "N hooks need review" and a warning.
  - Open: dropped events (model request, permission request, compaction,
    session end, subagent start/stop, interrupt) and handlers (`mcp_tool`,
    `prompt`, `agent`, async) are diagnostics; a project with only
    `hooks.json` is not seen until the configuration exposes the project
    root; hooks of one event run in turn; UserPromptSubmit cannot block;
    `trust_hooks` hashes the definition on disk at call time.
  - Hanafubuki, at its next pin: `ContextRequest` has a new field
    `user_message` (a struct literal needs it); new events `hook_started`
    and `hook_completed`; `AgentContinuationHook::applies` (default true);
    new crates and a `hooks` row; facade `hooks_list`, `set_hook_enabled`,
    `trust_hooks`; `AgentStoragePaths.hooks_state_file`.
- [ ] P14 (decision 4): `hanafubuki-agent-sandbox`, sandbox presets in
      `/permissions`, `/setup-default-sandbox` on Windows.
  - [x] P14a, Linux: `crates/sandbox` (`SandboxPolicy` read-only |
        workspace-write | full-access); the model's commands run through
        the binary as a helper (`--hanafubuki-agent-sandbox`: no_new_privs,
        Landlock writes only to the writable roots, `/dev/null`,
        `/dev/tty`; seccomp blocks the network unless allowed), Codex's
        Landlock + seccomp path rather than bubblewrap; fail closed (a
        mode the machine cannot enforce refuses the command, before
        approval). `sandboxMode` (unset: workspace-write on Linux,
        full-access elsewhere until P14b/c), `[sandboxWorkspaceWrite]`
        `writableRoots`, `networkAccess` (default false); per-session
        `set_session_sandbox_mode` (a subagent inherits it); `doctor`
        shows the sandbox. `crates/process` unchanged.
  - [x] P14b, macOS: Seatbelt through `/usr/bin/sandbox-exec -p <policy>
        -DWRITABLE_ROOT_N=…` (Codex's three `.sbpl` files, unchanged but
        for a header and `/dev/tty`); writes only under the writable roots,
        an existing `.git`, `.agents` and `.hanafubuki-agent` read-only;
        network only when allowed; available when `sandbox-exec` exists,
        else fail closed; workspace-write is now the macOS default.
        Verified on Linux (policy and command-line tests) and by a macOS
        target check of the sandbox crate only; `tests/macos.rs` first
        runs on the macOS CI job.
  - Open (P14b): on macOS in workspace-write, `git commit` and anything
    writing `.git` fail (as in Codex) until P14d's escalation.
  - [x] P14c, Windows, unelevated: Codex's restricted-token sandbox
        (`crates/sandbox/src/windows/`) through `helper_dispatch`: a
        write-restricted token whose restricting SIDs are one capability
        SID per writable root (SHA-256 of its `PathKey`, no state file),
        write ACEs on the roots, deny-write ACEs on an existing `.git`,
        `.agents`, `.hanafubuki-agent`; a private desktop; a kill-on-close
        job; TEMP a writable root on every platform. Opt-in by
        `[windows] sandbox = "unelevated"`; unset or `"elevated"` refuses
        sandboxed modes; the Windows default stays full-access.
        `/setup-default-sandbox` (Windows only) explains. Compiled and
        linted for Windows only; `tests/windows.rs` and the CLI test first
        run on the Windows CI job.
  - [ ] P14e, Windows, elevated: sandbox users from a UAC setup, network
        blocked by firewall rules, the command runner, readiness; a real
        `/setup-default-sandbox` writing `windows.sandbox = "elevated"`;
        then the Windows default. Left open by decision (2026-11-09):
        port it only once asked.
  - Open (P14c): unelevated does not block the network (Codex's offline
    environment variables only; the `sandbox` section then adds
    `"networkEnforced": false`); a background process ends with its
    command. `sandbox_status()` now applies `windows.sandbox` and returns
    a `Result` (Hanafubuki: the signature changed).
  - Hanafubuki, at its next pin (P14c): `AgentSettings::windows`
    (`AgentWindowsSandbox`); a settings table whose values are all unset
    is no longer written.
  - [x] P14d: `exec_command`'s `sandbox_permissions` (`use_default` |
        `require_escalated`) and `justification`; an escalated command is
        approved like any action (Manual asks, AutoReview's reviewer sees
        `sandboxEscalation`, FullAccess refuses it, as Codex's `never`),
        its digest differs from the sandboxed one, and an approved one
        runs unwrapped; a session allowance for it covers that command
        only (the overlay says so). A `sandbox` world-state section tells
        the model the mode, writable roots, network and how to escalate.
        `/permissions` presets: Read only, Default, Auto-review, Full
        access (both modes for the session). Eleven request goldens
        regenerated; corpora unchanged.
  - Open (P14d): `/permissions manual|auto-review|full-access` set the
    mode only (the preset names collide); the status line does not show
    the sandbox; Codex's `prefix_rule` and `with_additional_permissions`
    are not ported.
  - Hanafubuki, at its next pin (P14d): `ExecCommandRequest.
    sandbox_escalation`, `ApprovalAction.sandbox_escalation` and
    `SandboxEscalation` (re-exported); `ContextSectionKind::SANDBOX` (six
    known kinds), `MAXIMUM_AGENT_WORLD_STATE_SECTIONS` 12; approval
    payloads may carry `sandboxEscalation` (show the justification); copy
    the eleven request goldens.
  - Open: on Linux the network is off by default for the model's
    commands (P14d's escalation lets one command out, approved); test support's
    `configure_test_provider` also writes `sandboxMode = "full-access"`
    (test binaries cannot be the helper); `.git` under a writable root
    stays writable (Landlock cannot carve sub-paths); review sessions are
    not backed by a read-only sandbox yet.
  - Hanafubuki, at its next pin: call `hanafubuki_agent::sandbox::
    helper_dispatch()` first in `main` (else every command fails on Linux
    by default, or set `sandboxMode = "full-access"`); `AgentSettings`
    has `sandbox_mode` and `sandbox_workspace_write`;
    `ExecCommandRequest.sandbox`; `set_working_directory` is now
    `set_workspace_roots`; a new crate.
- [x] CI on macOS and Windows (red since P2.1, fixed after P14): platform
      assumptions in tests (line endings, absolute paths, the alternate
      screen, the Windows SID, PowerShell's wording); a narrow description
      column stacks; a stop that overtakes a background terminal's wait
      finds it gone; a confirmed cancellation whose commit outlasts the
      grace is no longer aborted (its sink heard nothing); a subagent
      shown after it asked for an approval shows it. Tests wait on
      outcomes (`TEST_WAIT`, 60 s) and give shutdowns that are not their
      subject `TEST_SHUTDOWN_DEADLINE` (10 s); tests whose time limit is
      the point measure it relative to a far larger one. Reproduced and
      cleared under CPU and disk load on Linux.
  - Open: `workspace_commands::a_denied_action_is_approved_between_turns_
    and_refused_during_one` uses a 10 s model delay as its window and is
    `#[ignore]`d until it holds the turn open itself (a TODO at the test;
    `cargo test -p hanafubuki-agent-tui -- --ignored` runs it);
    a turn thread blocked on the record's lock at the moment the abort is
    decided can still be aborted while staging its end (a narrow window,
    left as is); `mcp_oauth`'s refused-port wait rests on an unproven
    cause (another test's child holding the listener until it execs).

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
      that do not start the Agent (the interface keeps them since P5b).
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
