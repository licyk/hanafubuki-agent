# Follow-ups

## Extraction from Hanafubuki

The phases are in [docs/extraction-plan.md](docs/extraction-plan.md) §14;
Hanafubuki's `todo.md` tracks them with their done notes. Phases 1–3 are
done: the workspace, the decoupling inside Hanafubuki's tree, and the move.

- [x] Phase 4a: what the facade needs to run without a host: config
      layering (`AgentConfig::layered`, `ConfigLayers`, `-c` from Codex,
      `[network]`, `logLevel`, `AgentDirs`/`HANAFUBUKI_AGENT_HOME`), the
      data-dir lock (both modes), the standalone log file, the keyring
      store, the cwd workspace and `AgentRuntime::standalone`
      ([docs/config.md](docs/config.md), architecture.md §3.6, §10).
- [ ] Phase 4b: the CLI (`hanafubuki-agent-cli`, `clap`), `exec` (human +
      JSONL), `config`/`providers`/`mcp`/`sessions` subcommands, `doctor`.
      What it builds on and still needs:
  - parse `-c` with `ConfigOverrides::parse`, `-p`, `--cd`, `--add-dir`,
    `--data-dir` (overrides `AgentDirs::data_dir`), `--log-level` (a
    `logLevel` override) into `AgentRuntimeOptions`;
  - at start: `panic::configure_crash_record` +
    `panic::install_panic_hook(logging::logging_blocked)`, then
    `logging::install_file_logger(storage_paths().logs_dir, level)` with
    `config().standalone_settings()?.log_level`, then `start()` and
    `wait_until_started`;
  - `exec` passes its own `AgentEventSink` to `submit_turn_with_sink` and
    replaces the status/plugin/MCP sinks in `AgentRuntimeHost`; approvals
    and user-input questions need a policy (`--approve`);
  - `config set` writes keys the settings API does not patch (`network`,
    `logLevel`, profiles, `projects` trust) by editing `config.toml` with
    `toml_edit` itself, or the configuration gains a generic path write;
  - `doctor`: `DataDirLock::acquire` (and drop) for the lock, the keyring
    under `KEYRING_SERVICE`;
  - `[network] proxy` reaches the HTTP clients but not the child processes'
    environment (they inherit this process's, where a user's proxy
    variables already are); add an overlay if that is wanted;
  - AGENTS.md discovery reads the working directory's and the Git root's
    own `AGENTS.md` (each a root), not the directories between them, since
    the runtime selects no target; Codex reads every level.
- [ ] Hanafubuki, when it moves its pin past Phase 4a: nothing to change for
      the lock (it is taken by the start marker); `AgentProxy` is now
      `hanafubuki_agent_config::AgentProxy`, re-exported where it was.
- [ ] Phase 5: the TUI from Codex `rust-v0.93.0`, with `check-notices`.
- [ ] Phase 6: `release.yml` with binaries, clippy `-D warnings`, `v0.1.0`
      on crates.io.

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
