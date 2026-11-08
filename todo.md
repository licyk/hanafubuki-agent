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
- [x] Phase 4b: the CLI (`hanafubuki-agent-cli`, binary `hanafubuki-agent`,
      `clap`): the global flags, `exec` (text and JSON lines, `--approve`,
      questions on a terminal, exit codes, `-o`), `config path|get|set|edit`
      (`AgentConfig::value`, `set_user_value`, `layer_sources`), `providers`,
      `mcp`, `sessions`, `skills`, `memory`, `doctor`; the proxy in the
      Agent's processes' environment; the host-supplied identity
      (`AgentHostInstructions::identity`, `application`)
      ([docs/cli.md](docs/cli.md), architecture.md §3.6). Phase 4 is done:
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
  - `docs/agent-integration.md`: the host instructions now carry the
    identity and the application name.
  - Nothing else: the facade's other changes are additions (re-exports of
    MCP, approval and user-input types, `probe_keyring`, the
    configuration's key API), and the CLI is a crate Hanafubuki does not
    depend on.
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
