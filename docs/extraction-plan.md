# Agent extraction plan: `hanafubuki-agent` as a standalone project

Status: accepted (2026-11-08), every recommendation in §17 taken. Phases are
tracked in [todo.md](../todo.md) and Hanafubuki's `todo.md`. Written in
Hanafubuki's tree before the move (Phase 3), it names the crates by their
names there (§2.1 maps them to this repository's); it moved here with the
crates, and Hanafubuki keeps its integration document (§10.4).

## 0. Summary

The Agent moves to `/root/code_workspace/hanafubuki-agent`
(`github.com/licyk/hanafubuki-agent`, empty today), licensed GPL-3.0. It
ships as:

- **an embeddable library** (the `hanafubuki-agent` crate), which Hanafubuki
  consumes from crates.io as an ordinary Cargo dependency;
- **a standalone application** (`hanafubuki-agent` binary) with its own CLI,
  a TUI ported from Codex, a headless `exec` mode, its own configuration,
  storage, logging and data-directory lock.

Dependencies point one way: `hanafubuki → hanafubuki-agent`. The Agent
repository depends on no Hanafubuki crate. Hanafubuki keeps its domain
plugins (instances, tasks, UI context), its Tauri commands and Vue frontend,
and wires them to the Agent through the Agent's public host interface.

The work is done **in this tree first**: the Agent is decoupled from every
Hanafubuki domain crate while both still build and test together, with an
xtask check enforcing the boundary. Only then do the crates move, which is
then mostly `git mv` plus manifest rewrites. Tests, goldens and the history
corpus are regenerated in one tree, where a mistake is visible at once.

Most of the work is in four places:
- the host interface (§5);
- moving the Agent's settings out of Hanafubuki's config into the Agent's own
  (§6);
- the TUI port (§9);
- the shared foundation crates, where one process-wide fact (the panic hook)
  constrains the design (§4).

## 1. Current state (verified 2026-11-08)

### 1.1 What moves

The Agent's crates come to about 150K lines of Rust, including tests. They
fall into three groups.

**Agent crates, which move whole:**

| Crate | Lines |
| --- | ---: |
| `hanafubuki-agent` (kernel) | 44.6K |
| `hanafubuki-agent-application` | 30.6K |
| `hanafubuki-agent-history` | 8.2K |
| `hanafubuki-agent-memory` | 5.9K |
| `hanafubuki-agent-contracts` | 4.0K |
| `hanafubuki-agent-execution` | 3.8K |
| `hanafubuki-agent-plugin-{execution,mcp,web,workspace-fs}` | 4.0K together |

**Domain crates only the Agent uses, which move whole:**

| Crate | Lines |
| --- | ---: |
| `hanafubuki-mcp` | 11.8K |
| `hanafubuki-model-api` | 10.5K |
| `hanafubuki-workspace-fs` | 5.1K |
| `hanafubuki-skills` | 4.5K |
| `hanafubuki-web-search` | 3.3K |
| `hanafubuki-approval` | 1.3K |
| `hanafubuki-tool-contracts` | 0.8K |

No non-Agent Hanafubuki crate depends on these. The root depends directly on
two of the Agent's crates, `hanafubuki-mcp` and `hanafubuki-agent-memory`;
both uses are integration code.

**Shared infrastructure, used by both sides:**

| Crate | Lines | What the Agent uses |
| --- | ---: | --- |
| `hanafubuki-foundation` | 4.7K | `cancellation`, `blocking`, `background_worker`, `panic`, `paths`, `resolved`, `persistence`, `terminal_text`, `url_policy`, `environment`, `agent_settings` |
| `hanafubuki-process` | 6.9K | the executor, ownership receipts, process groups, PTY sizes |
| `hanafubuki-module-contracts` | 4.4K | `ModuleHost`, `Registry`, `PluginFaults`, `RestartPolicy` (the kardia host) |

None of the three holds Hanafubuki-specific production code: only test
fixtures mention ComfyUI paths. `foundation::agent_settings` exists only for
the Agent and Config.

### 1.2 Coupling that must be cut

Only `hanafubuki-agent-application` (AA) reaches Hanafubuki's domain crates:
`hanafubuki-config`, `hanafubuki-application`, `hanafubuki-management`,
`hanafubuki-webui`, `hanafubuki-api-server` and
`hanafubuki-instance-contracts`. By purpose:

- **Instances plugin.** It has 11 tools and uses `AgentInstanceMutationPorts`
  (instance mutation, provisioning, WebUI discovery, diagnostics),
  management request types, `WebUiInstallSelection`, `WebUiFamily` and
  `ApiServerStartOptions` (`plugins/instances/`, `mutation_ports.rs`).
- **Read sources.** `DesktopAgentReadSource` reads WebUI instances, instance
  management and task activity (`read_source.rs`, `management_tools/`,
  `marketplace.rs`).
- **Tasks plugin and Task Center.** These use `task_activity` types, and
  `AgentSlot` implements `hanafubuki_application::task_activity::AgentActivitySource`.
  Here a Hanafubuki-defined trait is implemented inside the Agent, so the
  dependency points the wrong way.
- **UI context.** `ui_context.rs` holds Hanafubuki page and section enums
  (Scan, `EmbeddedWebui`, the Mirrors and ApiServers settings sections).
- **Workspace roots.** They come from `managed_instances_dir` plus every
  instance's `install_dir` (`workspace_roots.rs`), narrowed to the selected
  instance's `core_path`.
- **Settings and paths.** `AppPaths`, `ConfigState`, `AppSettings`,
  `get_global_settings_inner`, `update_global_settings_patch_inner`,
  `mutate_agent_provider_registry_inner`, `safe_file` (for the start
  marker), the provider registry types, `ProxyMode` and
  `WebSearchProviderSetting`.
- **Runtime.** `hanafubuki_application::contained_runtime` provides
  `ContainedRuntime` (254 lines). It is generic, but lives in Hanafubuki.

Three leaks also reach below AA:
- `validate_instance_id` and `InvalidInstanceId` in `hanafubuki-agent-contracts`;
- `ContextSectionKind::APPLICATION_UI_CONTEXT`, which the kernel's
  `world_state.rs` knows;
- the "Application context" section of `prompts/base.md`, lines 132–156:
  instances, the API server, the Task Center. Line 1 also calls the Agent the
  "desktop application's" agent.

### 1.3 Settings and storage today

- **`<app_data>/config/agent.json`** holds `agentPermissionMode`,
  `agentMemory.*`, `agentDisabledPlugins`, `webSearchProvider`,
  `searxngBaseUrl` and `webFetchUseJinaReader`.
- **`<app_data>/config/agent-providers.json`** holds `agentProviders`: the
  revision, the default provider, the model policy and the providers. API
  tokens are stored verbatim, by design.
- **`network.json`** holds `proxyMode` and `proxyAddress`. These belong to
  the host; the Agent only reads them.
- **`<app_data>/data/agent/`** is the Agent's storage:
  - `history/` (JSONL rollouts and `index.sqlite3`);
  - `memory/`;
  - `mcp-settings.json`;
  - `skills/`;
  - `start-marker.json`, `crash-record.json` and `process-groups.json`;
  - the user `AGENTS.md`.
- **MCP secrets** are kept in the OS keyring through the root's
  `KeyringSecretStore` (`keyring` 4.2).

### 1.4 The root's integration surface

- **Commands and events:** 75 Agent Tauri commands (`src/agent/commands/`),
  the Agent's command guard, `TauriAgentEventSink` and
  `AgentPresentationDispatcher`.
- **Composition:**
  - `composition/application/agent.rs` (`prepare_agent_start`, which builds
    the `AgentLaunch`);
  - `instance_services.rs` (the mutation ports);
  - `domains.rs` (the slot and the start marker).
- **Lifecycle:** the `agent` module row (`modules/manifest.rs`), the shutdown
  stage, the Skill watcher (`platform/skill_watcher.rs`), and the Agent
  status and plugin event channels.
- **Diagnostics and settings:** the Agent's part of the diagnostics, the
  task-activity merge, and `apply_saved_settings` after a settings save.

### 1.5 Licensing facts

- **Hanafubuki:** it has no LICENSE file. It already links kardia 0.2
  (`cordis-rs`, GPL-3.0-only), so a GPL-3.0 Agent adds no new kind of
  obligation.
- **Codex:** Codex is Apache-2.0, with a NOTICE file and Ratatui-derived MIT
  files (`custom_terminal.rs`, `width.rs`, theme code). Apache-2.0 code may
  be included in a GPL-3.0 project, provided its LICENSE and NOTICE are kept,
  its MIT headers are kept, and modified files are marked.
- **crates.io names:** `hanafubuki-agent` and the other names proposed in §2
  are all unclaimed.

## 2. Target architecture

### 2.1 The `hanafubuki-agent` repository

```
hanafubuki-agent/
├── Cargo.toml                 workspace; version, license, repository in [workspace.package]
├── LICENSE                    GPL-3.0 text
├── NOTICE                     own notice + Codex (Apache-2.0) + Ratatui (MIT) attributions
├── AGENTS.md / CLAUDE.md      the repository's rules (moved from Hanafubuki's Agent rules)
├── rust-toolchain.toml        1.99.x, as Hanafubuki
├── .cargo/config.toml         alias xtask = "run --quiet --package xtask --"
├── .github/workflows/         ci.yml, release.yml (ported from cordis-rs, §12)
├── docs/                      architecture.md, rules/{plugins,lifecycle,architecture}.md, embedding.md, config.md
├── crates/
│   ├── foundation/            hanafubuki-agent-foundation   (copy of hanafubuki-foundation, trimmed)
│   ├── process/               hanafubuki-agent-process      (copy of hanafubuki-process)
│   ├── host/                  hanafubuki-agent-host         (copy of hanafubuki-module-contracts)
│   ├── runtime/               hanafubuki-agent-runtime      (ContainedRuntime, from hanafubuki-application)
│   ├── tool-contracts/        hanafubuki-agent-tool-contracts
│   ├── approval/              hanafubuki-agent-approval
│   ├── model-api/             hanafubuki-agent-model-api
│   ├── contracts/             hanafubuki-agent-contracts    (the plugin API; public for host plugins)
│   ├── kernel/                hanafubuki-agent-kernel       (today's hanafubuki-agent)
│   ├── history/               hanafubuki-agent-history
│   ├── memory/                hanafubuki-agent-memory
│   ├── execution/             hanafubuki-agent-execution
│   ├── skills/                hanafubuki-agent-skills
│   ├── mcp/                   hanafubuki-agent-mcp
│   ├── web-search/            hanafubuki-agent-web-search
│   ├── workspace-fs/          hanafubuki-agent-workspace-fs
│   ├── plugin-{execution,mcp,web,workspace-fs}/
│   ├── config/                hanafubuki-agent-config       (new: the Agent's own configuration, §6)
│   ├── agent/                 hanafubuki-agent              (the embedding facade: today's AA minus Hanafubuki)
│   ├── tui/                   hanafubuki-agent-tui          (new: ported from Codex, §9)
│   └── cli/                   hanafubuki-agent-cli          (new: the binary `hanafubuki-agent`, §8)
└── xtask/                     architecture checks + version/tag (§13)
```

**Naming.** Every published crate carries the `hanafubuki-agent-` prefix.
Two packages named `hanafubuki-foundation` in one dependency graph (a path
crate and a crates.io crate) would build, but every error message, `cargo
tree` and `cargo update -p` would be ambiguous. So the Agent's copies are
renamed, and Hanafubuki keeps its own names.

**Facade and kernel.** The kernel takes the name `hanafubuki-agent-kernel`,
so that `hanafubuki-agent`, the name Hanafubuki depends on, can be the
embedding facade.

**Edition and format.** The repository stays on edition 2021 and rustfmt's
defaults, so 150K lines are not reformatted on import. Moving to edition
2024 and cordis's 120-column width can come later, each as its own commit.

### 2.2 Layering inside the Agent repository

From bottom to top:

1. **`foundation`, `process`, `host`, `runtime`, `tool-contracts`:** neutral
   mechanisms.
2. **Domain crates:** `approval`, `model-api`, `skills`, `mcp`, `web-search`,
   `workspace-fs`, `execution`, `history`, `memory`.
3. **`contracts`:** the plugin API, which re-exports the host's registry and
   fault mechanisms.
4. **`kernel`.**
5. **Plugin crates.**
6. **`config`.**
7. **`hanafubuki-agent`:** composition, the generic plugins, the slot and
   the host interface.
8. **`tui` and `cli`:** the standalone front ends.

The rules carried over are today's `agent_boundaries` rules, under the new
names:
- the kernel depends on no plugin;
- a plugin depends on no plugin, on neither the kernel nor the facade, and
  never on another plugin;
- model-api names no context-section kind;
- `host` depends only on kardia and `foundation`.

### 2.3 The two modes

| | Standalone | Embedded (Hanafubuki) |
| --- | --- | --- |
| Entry | `hanafubuki-agent` binary: TUI, `exec`, subcommands | `hanafubuki_agent::AgentRuntime` built by the host |
| Configuration | `hanafubuki-agent-config`, from its config dir | the same crate, from a config dir the host names (§6) |
| Storage | its own data dir (`~/.local/share/hanafubuki-agent`, OS-specific) | the dir the host names (`<app_data>/data/agent`, unchanged) |
| Workspace | cwd and its Git root, or `--cd` | the host's workspace-roots port (instances) |
| Plugins | the generic set | the generic set plus host plugin rows (instances, tasks, UI context) |
| Approvals and questions | the TUI's overlays; `exec`'s policy flags | the host's UI, through the event sink and the resolve calls |
| Secrets | the Agent's keyring store | the host's secret store port |
| Logging | a file under the data dir; content rules as now | the host's `log` logger |
| Panic hook | installed by the binary | the host's, consulting the Agent (§4.2) |

## 3. Dependency boundaries and how they are enforced

**In the Agent repository**, `cargo xtask check` (§13) enforces:
- no dependency on any crate whose name starts with `hanafubuki-` unless it
  is one of the workspace's own;
- the layer rules of §2.2.

**In Hanafubuki:**
- **During Phase 2:** a new `agent_extraction` check refuses any dependency
  from an Agent crate (including the shared crates' Agent copies once they
  exist) on `hanafubuki-config`, `-application`, `-management`, `-webui`,
  `-api-server` or `-instance-contracts`. This makes the decoupling
  measurable before anything moves.
- **After the switch:** the existing `agent_boundaries` check is reduced to
  what still applies. No Hanafubuki crate other than the root and
  `hanafubuki-agent-integration` (§10) depends on a `hanafubuki-agent*`
  crate. `hanafubuki-agent-integration` may depend on
  `hanafubuki-agent-contracts` and `hanafubuki-agent`, never on the kernel or
  model-api.

## 4. The shared infrastructure

### 4.1 Recommendation: separate, renamed copies

The Agent repository carries its own copies of `foundation`, `process` and
`module-contracts`, trimmed to what the Agent uses. Hanafubuki keeps its own.
This is the duplication the request accepts, and it keeps Hanafubuki's core
mechanisms off the Agent's release cycle.

The alternative is for Hanafubuki to delete its copies and use the Agent's
published crates: one receipt registry, one `ResolvedPath`. That would make
every change to Hanafubuki's foundation, process or module host wait for an
Agent release, and these are the crates every Hanafubuki domain is built on.

What the separate copies change:

- **Two process-ownership domains.** The Agent owns its `ExecutorManager`.
  This covers `exec_command`, background terminals and MCP stdio children,
  and the Agent already records them in its own `process-groups.json`. The
  host no longer passes its executor in. Each domain keeps the "reserve,
  receipt, confirm" rules for its own processes. The Agent's shutdown,
  already a stage of Hanafubuki's quit, disposes the Agent's domain.
  `lifecycle.md` gains a sentence saying the Agent's processes are the
  Agent's, and the Agent repository carries the rules for its own domain.
- **Types do not cross the boundary.** `ResolvedPath`, `CancellationToken`
  and `ExecutorManager` are distinct types on the two sides. The host
  interface (§5) therefore takes plain values (`PathBuf`, strings, the
  Agent's own request types), never Hanafubuki's foundation types.
- **The copies diverge.** A fix to one is ported to the other by hand. Each
  repository's `todo.md` keeps a short "shared with the other repository"
  list of the modules that have a twin.

### 4.2 The one process-wide fact: the panic hook

Hanafubuki's `install_panic_hook` aborts the process on any panic its own
`foundation` does not see as contained (`panic/hook.rs`). Containment is
held in that copy's thread-locals: `CONTAINED_THREAD` and the guard depth.
With two copies, an Agent thread that the Agent's copy marked contained
looks uncontained to Hanafubuki's hook. Every Agent plugin panic would then
abort Hanafubuki, which undoes the fault isolation the Agent was built for.

The fix is small and points the right way:
- the facade exports `hanafubuki_agent::panic::is_panic_contained()`, which
  re-exports the Agent foundation's query;
- Hanafubuki's hook treats a panic as contained when either copy says so: a
  one-line change in `report_panic`.

The standalone binary installs the Agent foundation's own hook. The crash
records stay separate: the Agent's already lives at
`data/agent/crash-record.json`. A test in Hanafubuki pins the bridge: a
panic in an Agent plugin thread, with Hanafubuki's hook installed, unwinds
and does not abort. It runs in a child process, as Hanafubuki's existing
panic-hook tests do (`hanafubuki-foundation/tests/panic_hook.rs`).

**Other process-wide state is shared already:**
- `log` is one crate and the host's logger;
- kardia 0.2 unifies by semver;
- tokio runtimes are per owner.

There is nothing else of this kind.

## 5. The host interface (embedded mode)

### 5.1 Shape

The facade replaces today's Hanafubuki-shaped inputs (`AgentServiceInputs`,
`AgentLaunch`, `DesktopAgent`) with:

```rust
pub struct AgentHostConfig {
    pub config_dir: PathBuf,            // the Agent's own settings (§6)
    pub data_dir: PathBuf,              // the Agent's storage root (§7)
    pub bundled_skills: Option<PathBuf>,
    pub start: AgentStartOptions,       // safe mode, disabled-by-host, …
}

pub struct AgentHost {                  // what the host supplies; every field optional or defaulted
    pub plugins: Vec<AgentPluginRow>,   // host plugin rows, mounted after the Agent's
    pub workspace: Arc<dyn WorkspaceRootsPort>,
    pub network: Arc<dyn NetworkSettingsPort>,   // proxy; standalone reads its config
    pub secrets: Arc<dyn SecretStore>,
    pub events: Arc<dyn AgentEventSink>,
    pub status: Arc<dyn AgentStatusEventSink>,
    pub plugin_events: Arc<dyn AgentPluginEventSink>,
    pub mcp_events: Arc<dyn McpEventSink>,
}

let runtime = AgentRuntime::new(config, host)?;   // the slot: start, enable, turn_off, restart, status, shutdown
runtime.start();
let service = runtime.handle()?;                    // AgentService: sessions, turns, settings, MCP, Skills, …
```

`AgentRuntime` is today's `DesktopAgent` and `AgentSlot`, renamed and
unchanged in behaviour. That covers the supervised start, kernel-fault
restarts, the crash-loop stop and bounded shutdown. Today's
`AgentApplicationService` becomes the handle's API. All 75 Tauri commands
keep mapping onto it one to one; only their argument types change.

### 5.2 Host ports (each with a standalone default in the facade)

| Port | Replaces | Standalone default |
| --- | --- | --- |
| `WorkspaceRootsPort` (`roots()`, `selected(scope)`) | `managed_instances_dir` + instance configs + `AgentInstancePathPort` | cwd and its Git root; `--cd`/`--add-dir` |
| `extra_read_roots` (in `WorkspaceRootsPort`) | `app_data_dir`, `logs_dir` | none |
| `NetworkSettingsPort` | `network.json` through `ConfigState` | the Agent config's `[network]` |
| `SecretStore` | the root's `KeyringSecretStore` | the facade's keyring store (`keyring` 4) |
| `BlockingOperationPort` | the root's pool | the runtime's blocking pool |
| Event and status sinks | `Tauri*Sink` | the TUI/`exec` consumers; no-op |
| `HostContext` (world state) | `APPLICATION_UI_CONTEXT` in the kernel | none |

`HostContext` generalises the UI context. The kernel no longer knows
`APPLICATION_UI_CONTEXT`. Steering carries an opaque host context: a kind,
content and a version. A host plugin registers its renderer through the
existing renderer registry. So Hanafubuki's UI context becomes a
`hanafubuki-agent-integration` plugin like any other context section. The
kernel's world-state handling is unchanged, because it already versions
sections by kind.

### 5.3 What moves to Hanafubuki's side

- **Plugins:** `instances`, `tasks` and `ui-context`, written against the
  public `hanafubuki-agent-contracts` and registered as host plugin rows.
  Today extra rows are test-only (`new_with_test_plugins`); they become the
  public `AgentHost::plugins`.
- **The instance-id leak:** `validate_instance_id` and `InvalidInstanceId`
  move into the instances plugin.
- **Read source and mutations:** `DesktopAgentReadSource`, the mutation
  ports, `management_tools/` and `marketplace.rs`.
- **The application prompt:** the "Application context" part of `base.md`
  becomes an Instructions-slot section contributed by Hanafubuki's
  integration. `base.md` itself is made host-neutral: "a coding and
  operations agent".
- **Task Center:** `AgentActivitySource` is inverted. The facade exposes run
  projections (`AgentRunProjection`, already its type), and Hanafubuki's
  adapter implements Hanafubuki's `AgentActivitySource` over them.
- **File search:** its roots come from `WorkspaceRootsPort` instead of
  instance ids.

### 5.4 Contract stability

The plugin API (`hanafubuki-agent-contracts`) and the facade are now public
API across repositories. The Agent follows semver from 0.1. Before 1.0, a
minor bump may break, as Cargo treats `0.x`. Hanafubuki pins `0.N` and moves
with each minor release.

The registries, renderers and tool contracts are the stable surface. The
kernel's internals are not exported.

## 6. The configuration system

### 6.1 One system, owned by the Agent, in both modes

`hanafubuki-agent-config` owns every Agent setting:
- the permission mode;
- memory;
- disabled plugins;
- web search (provider, SearXNG URL, Jina);
- the provider registry (providers, default, model policy, tokens);
- MCP servers, moved into the same system from `mcp-settings.json`;
- TUI preferences;
- logging.

Hanafubuki's settings UI stops writing these through
`update_global_settings_patch`. It calls the Agent's settings API, through
thin Tauri commands that keep today's revision checks (`agent_settings_read`,
`agent_settings_write`, and the provider commands as they are). Hanafubuki
keeps only the host-owned values: `disabledModules` (whether the `agent`
module runs) and the network settings, which it supplies through
`NetworkSettingsPort`.

### 6.2 Format and layering (standalone)

Configuration is TOML, as in Codex, so that it can be edited by hand.

**Locations:**
- the user config is `<config_dir>/config.toml`, with `config_dir` following
  the OS convention, overridable with `HANAFUBUKI_AGENT_HOME`;
- an optional project config, `<git root>/.hanafubuki-agent/config.toml`, is
  read only for a trusted directory;
- named profiles live in `[profiles.<name>]` and are selected with `-p`;
- `-c key=value` overrides the merged result, with dotted paths and values
  parsed as TOML. Codex's `config_override.rs` is small and is ported.

**Precedence, lowest first:** defaults < user < profile < project <
`-c`/flags.

**Embedded mode** reads the user layer only, from the `config_dir` the host
names: `<app_data>/config/agent/`. There are no profiles and no project
layer.

**Reading and writing:**
- reading is tolerant, by the decoding rule: unknown keys are kept and
  warned about, and a file that cannot be used is quarantined, never deleted;
- writing goes through `toml_edit`, which keeps comments, with an atomic
  replace and a revision check for concurrent writers.

### 6.3 Moving today's settings (needs a decision)

Users' provider registries (including tokens), permission mode, memory
policy and plugin choices sit in `agent.json` and `agent-providers.json`.
MCP servers sit in `mcp-settings.json`. After the switch Hanafubuki no
longer reads the Agent keys. Without an import, every user loses their
provider setup on update. The data is not deleted, but it is no longer read.

**Recommendation:** a one-time import, at the integration's first start,
when the Agent config does not exist yet:
- the agent keys are read from Hanafubuki's two files;
- the Agent's config is written;
- the old keys are left in place for one release, then dropped.

AGENTS.md discourages migrations unless asked for; this is one, so it is
listed in §17.

The import is about 150 lines in `hanafubuki-agent-integration` plus a
test against the settings corpus.

### 6.4 Secrets

- **Provider tokens** stay where they are by design: in the provider
  registry file, plaintext. The file is owner-only (`0600` on Unix).
  Standalone also accepts `api_key_env = "NAME"` so that a token can stay out
  of the file.
- **MCP secrets** go through the `SecretStore` port. Standalone mode uses the
  keyring (`keyring` 4, as Hanafubuki); embedded mode uses the host's. The
  entry names stay as they are, so embedded secrets need no migration.

## 7. Storage and persistence

- **Data directory:**
  - embedded mode keeps `<app_data>/data/agent` unchanged, so history,
    memory and Skills carry over with no migration;
  - standalone mode uses the OS data dir (`dirs::data_dir()/hanafubuki-agent`),
    overridable with `HANAFUBUKI_AGENT_HOME`;
  - the layout is today's `AgentStoragePaths`.
- **Lock:** the facade takes an advisory lock (`<data_dir>/.lock`, via
  `fs4`) for the runtime's life. The history store's SQLite index and
  per-session JSONL writers assume one writer process, and two processes on
  one data dir would corrupt them. A standalone TUI started on Hanafubuki's
  data dir while Hanafubuki runs therefore refuses with a clear message
  instead of sharing it. Separate directories by default make that the
  exception. The lock is a file the OS releases when the process dies, so a
  crash leaves nothing to clean up.
- **Logs:**
  - standalone writes `<data_dir>/logs/hanafubuki-agent.log`, rotated, at the
    configured level;
  - the content rules move with the Agent: content only at `trace`, bounded
    by `bounded_trace_content`, never credentials;
  - embedded mode keeps using the host's logger.
- **Crash record and start marker:** both are unchanged, under the data dir.
- **Persisted formats:** unchanged by the move. The history corpus
  (`formatVersion` 5) moves to the Agent repository with its goldens, and
  must keep decoding there.

## 8. Standalone capabilities

### 8.1 The CLI (`hanafubuki-agent-cli`, binary `hanafubuki-agent`)

The CLI uses `clap`, mirroring Codex's shape where it fits:

```
hanafubuki-agent [PROMPT]                 # the TUI (default)
hanafubuki-agent exec [PROMPT|-]          # headless: one turn or a script; --json for JSONL events
hanafubuki-agent resume [--last|ID]       # reopen a session in the TUI
hanafubuki-agent sessions list|show|archive|delete
hanafubuki-agent config path|get|set|edit
hanafubuki-agent providers list|add|remove|default
hanafubuki-agent mcp list|add|remove|connect
hanafubuki-agent skills list|install|remove
hanafubuki-agent memory status|reset
hanafubuki-agent doctor                   # config, providers, data-dir lock, keyring, MCP reachability

global: -c key=value, -p PROFILE, -m MODEL, --cd DIR, --add-dir DIR,
        --permission-mode default|plan|full-access, --log-level, --data-dir
```

### 8.2 `exec`

`exec` runs the kernel with no TUI:
- events go to stdout, as human-readable text or as JSONL;
- approvals follow `--permission-mode` and an `--approve` policy (`deny`,
  `ask` on a TTY, or `allow` for named tools);
- user-input questions fail unless they can be answered on a TTY;
- the exit code reflects the turn's terminal reason.

Codex's `exec` human and JSONL event processors are the model to follow.

### 8.3 What standalone needs that the embedded build gets from Hanafubuki

- A workspace roots provider (§5.2).
- A keyring secret store.
- The Skill watcher. It moves from Hanafubuki's root into the facade, so both
  modes have it.
- The bundled system Skills, `install_bundled_system_skills`. The binary
  embeds them; the host passes its path.
- Panic hook installation, logging setup and terminal restore on panic.
- `doctor`.

## 9. The TUI port

- **Base:** the Codex TUI at `rust-v0.93.0`. It is 116 files and about 40K
  lines, and talks to the core directly through `Op` and `EventMsg`.
  - The Codex TUI at HEAD is not usable. It is about 300K lines, behind a
    JSON-RPC app-server protocol of about 230 methods.
  - `rust-v0.48.0` is smaller but lacks rendering work done since.
- **Forked dependencies:** at `rust-v0.93.0`, Codex patches `crossterm` with a
  git fork (`nornagon/color-query`) and `ratatui` with
  `nornagon-v0.29.0-patch` (as it already did at `rust-v0.48.0`). crates.io refuses git dependencies, so the port
  targets upstream `ratatui` 0.30 and `crossterm` 0.29.
  - ratatui 0.30 has the scrolling-regions feature upstream.
  - The color query (the terminal palette) is dropped, or replaced by a
    fixed theme with an override.
  - This is the first spike of Phase 5.
- **Adapter:** one module, `tui/src/agent_bridge.rs`, maps `AgentEventKind`
  onto the TUI's internal event enum, and maps submit, steer, interrupt,
  approve, answer, compact and set-mode onto the handle's API. This replaces
  Codex's `ConversationManager` and `CodexConversation`.
  - Session list and resume come from the history store.
  - The `/model` list comes from the provider registry and `ModelCatalog`.
  - `@` file mentions use the workspace roots (the Codex `file-search` crate,
    about 1.7K lines, built on `ignore` and `nucleo`).
- **Scope:** the survey's include and skip lists.
  - **Include:** the terminal setup, the inline viewport
    (`custom_terminal`, `insert_history`), the app loop, the chat widget,
    history cells, streaming and markdown, diffs, and the composer with its
    popups.
  - **Also include:** the approval and request-user-input overlays, the
    pager, a status line, key hints, and these slash commands: new, resume,
    compact, diff, model, mode, mcp, skills, status, mention and quit.
  - **Skip:** login, ChatGPT accounts, feedback upload, update checks, rate
    limits, analytics, cloud and daemon, voice, review mode and
    multi-agent.
  - Plan mode and the task checklist get history cells; their events exist
    in the kernel already.
- **Licence checklist:**
  - keep Codex's LICENSE and NOTICE in `NOTICE`;
  - keep the MIT headers on the Ratatui-derived files;
  - add a "Modified from OpenAI Codex `rust-v0.93.0` (Apache-2.0)" line to
    every ported file;
  - check the `.tmTheme` assets' licences before shipping, or drop them.
- **Tests:** keep the snapshot tests that come with the ported widgets, and
  add an adapter test that drives a scripted model, as the
  kernel's `FakeServer` does, through the bridge into the chat widget's
  history cells.

## 10. Hanafubuki after the extraction

### 10.1 What stays

- **`hanafubuki-agent-integration` (new crate, about 6K lines):**
  - the instances, tasks and UI-context plugins;
  - the read source and the mutation ports;
  - the Task Center adapter;
  - the application-context instructions section;
  - the settings import;
  - the workspace-roots and network ports;
  - the keyring secret-store adapter.

  It depends on `hanafubuki-agent`, `hanafubuki-agent-contracts` and
  Hanafubuki's domain crates.
- **The root:** the 75 thin Tauri commands, the guard, the event and status
  sinks, the `agent` module row, the shutdown stage, the panic-hook bridge,
  and the composition (`AgentRuntime::new(config, host)`).
- **Vue:** the Agent panel, settings and transcript, unchanged. The settings
  panels switch from the generic settings patch to the Agent settings
  commands.

### 10.2 What leaves

The 17 Agent crates and the Skill watcher leave. So do the Agent parts of
Hanafubuki's foundation (`agent_settings`) and Config:
- the `agent*` settings structs, schema entries, validation and persisted
  documents;
- `agent-providers.json` handling, which stays only as the import's reader
  (§6.3).

### 10.3 Checks and goldens

- **Goldens split:**
  - the Agent repository keeps the kernel and application goldens over the
    generic plugin set, and the history corpus with
    `HANAFUBUKI_AGENT_UPDATE_GOLDEN*`;
  - Hanafubuki keeps integration goldens: today's request goldens with the
    instance and task tools and the application section. Every current
    request golden contains `control_instance`.
- **xtask:**
  - `agent_boundaries`, `model_api_context` and the Agent parts of the
    strict-decoding policy (37 of 114 entries) and the module-size policy
    move to the Agent repository;
  - Hanafubuki keeps `agent_faults` (its commands end in the guard) and the
    sync-command entries for its Agent commands.

### 10.4 Docs

These move to the Agent repository:
- `docs/agent-architecture.md`;
- `docs/rules/agent-plugins.md`;
- `docs/backend-spec.md` §11;
- the Agent content-logging section of AGENTS.md.

Hanafubuki gains `docs/agent-integration.md`, covering the host plugins and
ports, the settings path, the panic bridge and the quit stage. AGENTS.md
links to it.

## 11. Development workflow

- **Phase 2:** everything is in this tree. Hanafubuki's normal validation
  applies.
- **Phases 3 to 6 (before the first release):** Hanafubuki depends on the
  Agent through a pinned git revision:
  `hanafubuki-agent = { git = "https://github.com/licyk/hanafubuki-agent", rev = "…" }`.
  `main` keeps building, and each Agent change Hanafubuki needs is a rev
  bump.
- **Local co-development:** an uncommitted
  `[patch."https://github.com/licyk/hanafubuki-agent"]` (later
  `[patch.crates-io]`) section points `hanafubuki-agent` at
  `../hanafubuki-agent/crates/agent`. A Hanafubuki script, `bun run
  agent:local`, toggles it.
- **After the first crates.io release:** Hanafubuki depends on
  `hanafubuki-agent = "0.1"`, and Cargo.lock pins the exact version. The
  Agent's CI runs its own checks on 3 OSes. Hanafubuki's CI builds against
  the pinned version.

## 12. Release workflow (ported from cordis-rs)

- **`ci.yml`:**
  - on push to `main`, on PRs and on manual dispatch, run on ubuntu, windows
    and macos;
  - steps: `cargo fmt --check`, `cargo check --workspace --all-features
    --tests` with warnings as errors, `cargo xtask check`, and `cargo test
    --workspace`;
  - Clippy is added with `-D warnings` once the workspace is clean of
    warnings, as its own commit;
  - path filters skip docs-only changes, as cordis's do.
- **`release.yml`:**
  - triggered by a `v*` tag, or by a manual run that resumes a stopped
    release;
  - jobs `plan`, `ci-result` and `checks`, as in cordis: the tag must name
    the workspace version, and CI's run of the commit is reused;
  - `publish` runs `cargo publish --workspace`, in dependency order, skipping
    versions already on crates.io and waiting out crates.io's new-crate rate
    limit. About 25 crates are published on the first release, which takes
    about an hour under that limit.
  - **New:** `binaries`, a matrix over x86_64 Linux (gnu), x86_64 and
    aarch64 macOS, and x86_64 Windows (msvc). It builds `hanafubuki-agent`
    with `--release`, packages `tar.gz` or `zip` archives with LICENSE and
    NOTICE, and writes SHA-256 sums.
  - `github-release` creates the release with generated notes from the
    previous tag, and uploads the archives and sums.
- **Secrets:** `CARGO_REGISTRY_TOKEN` (the crates.io token you configured in
  the repository), and `GITHUB_TOKEN` for the release.
- **xtask release commands, ported from cordis's `version.rs` and
  `tag.rs`:**
  - `cargo xtask version <patch|minor|major> [--dry-run]` bumps
    `[workspace.package].version` and every internal requirement, then
    refreshes Cargo.lock;
  - `cargo xtask tag [--push] [--dry-run]` makes an annotated `v<version>`
    tag on a clean HEAD.

  The npm and site workflows are not ported.

## 13. xtask in the Agent repository

Ported from Hanafubuki's xtask, the Agent-specific parts, plus cordis's
`arch`:

| Command | Checks |
| --- | --- |
| `check` | runs every check below |
| `check-dependencies` | §2.2 layers, no foreign `hanafubuki-*` crate, `host` only on kardia + foundation |
| `check-source-layout` | Hanafubuki's layout rules; `lib.rs` only declares and re-exports (cordis's `arch` also covers `mod.rs`, which 473 items in the moving crates would fail, so it is not adopted) |
| `check-module-size` | file/function limits with the policy file (moved entries) |
| `check-strict-decoding` | the Agent's IPC/integrity types, with its own policy file (the 37 entries) |
| `check-model-api-context` | model-api names no section kind |
| `check-notices` (new) | every file derived from Codex carries its attribution header (§9) |
| `version`, `tag` | release (§12) |

## 14. Phases

Each phase ends validated, committed and pushed; the repository it lands in
is named.

| Phase | Where | Contents | Done when |
| --- | --- | --- | --- |
| 0 | — | Decisions (§17) | answered |
| 1 | agent repo | Bootstrap: workspace skeleton, LICENSE/NOTICE, toolchain, AGENTS.md, xtask with `check-dependencies`/`check-source-layout`/`version`/`tag`, `ci.yml` | CI green on an empty workspace on 3 OSes |
| 2a | Hanafubuki | Generic seams in the Agent: `WorkspaceRootsPort`, `NetworkSettingsPort`, `SecretStore`, `HostContext` (the kernel loses `APPLICATION_UI_CONTEXT`), public host plugin rows, `AgentActivitySource` inverted, the instance-id leak and `base.md`'s application section moved out | the instances/tasks/UI-context plugins live in a new `hanafubuki-agent-integration`; goldens unchanged (the application section is contributed, same text) |
| 2b | Hanafubuki | The Agent's own config crate inside the tree (`hanafubuki-agent-config`), the Agent's settings off `hanafubuki-config`, the import (§6.3), the settings commands; `ContainedRuntime` into the Agent side | `agent_extraction` check passes: no Agent crate depends on config/application/management/webui/api-server/instance-contracts |
| 2c | Hanafubuki | Agent copies of foundation/process/module-contracts (`hanafubuki-agent-foundation`, …), the Agent's own executor, the panic-hook bridge and its test, the Skill watcher into the Agent | the Agent crates depend on no Hanafubuki crate at all; full validation; the bridge test proves a plugin panic does not abort |
| 3 | both | Move: the Agent crates to the agent repo (renamed per §2.1), its goldens and corpus, its docs; Hanafubuki depends on a pinned git rev | both repos' full validation; Hanafubuki builds and its integration goldens match |
| 4 | agent repo | Standalone base: `hanafubuki-agent-config` layering, data-dir lock, logging, keyring store, workspace roots from cwd, the CLI skeleton, `exec` (human + JSONL), `config`/`providers`/`mcp`/`sessions` subcommands, `doctor` | an `exec` turn against a scripted model, end to end, in CI |
| 5 | agent repo | The TUI: dependency spike (upstream ratatui/crossterm), port, bridge, slash commands, resume picker, approvals/questions, notices | snapshot tests and a bridge test; manual run on Linux/macOS/Windows terminals |
| 6 | agent repo | `release.yml`, binaries, first release `v0.1.0` to crates.io | release run green; crates on crates.io; archives on the GitHub release |
| 7 | Hanafubuki | Switch to `hanafubuki-agent = "0.1"`, remove the git pin, `agent:local` script, integration doc, todo.md | full validation; the Tauri app runs the Agent (visual check by you) |

Phase 2 is most of the risk and most of the work. 2a, 2b and 2c are each
several commits. Phase 3 should then be almost mechanical.

## 15. Testing and validation

- **Phase 2:** Hanafubuki's full validation every commit, plus the new
  `agent_extraction` check.
  - The integration's plugins get tests through the public host plugin rows,
    not test-only constructors.
  - The goldens must stay byte-identical through 2a and 2b, because the
    application section keeps its text; only its owner changes.
- **Agent repository:** the kernel contract suites, the application tests
  over the generic plugins, the goldens and the history corpus, and the TUI
  snapshots and bridge test.
  - `exec` end-to-end tests run against the scripted model (`FakeServer`
    moves with it).
  - CI runs on 3 OSes, which also closes Hanafubuki's open off-Linux items
    for the Agent's code: the watcher backends, the stdio exit reason and
    the memory install rename.
- **Hanafubuki after the switch:** the integration goldens, the panic-bridge
  test, the settings import test, and the existing Tauri-side Agent tests.

## 16. Risks

| Risk | Mitigation |
| --- | --- |
| The panic hook aborts on Agent panics (two foundation copies) | The bridge (§4.2), with a test, in Phase 2c before the move |
| Users lose provider setup on update | The import (§6.3) |
| Two processes on one data dir | The data-dir lock (§7) |
| The shared copies drift | The twin lists in both `todo.md`s; fixes ported deliberately |
| crates.io refuses git dependencies (TUI forks) | Upstream ratatui/crossterm only; spike first in Phase 5 |
| crates.io new-crate rate limit on the first release | The ported publish loop waits and resumes |
| The public plugin API churns while Hanafubuki depends on it | Pre-1.0 minor bumps; Hanafubuki moves deliberately with each |
| Codex licence obligations missed | `check-notices` in xtask; NOTICE review before `v0.1.0` |
| The move loses history | Decision 6 (§17) |

## 17. Decisions (accepted 2026-11-08: the recommended option of each)

1. **Shared crates:** separate renamed copies in the Agent repository, plus
   the panic-hook bridge (recommended, §4). The alternative is for Hanafubuki
   to use the Agent's published foundation, process and host crates and
   delete its own.
2. **Settings import:** a one-time import of `agent.json`,
   `agent-providers.json` and `mcp-settings.json` into the Agent's config on
   first start, with the old keys left for one release (recommended, §6.3).
   The alternative is no import, so users re-enter their providers.
3. **Registry:** crates.io through `CARGO_REGISTRY_TOKEN`, as cordis does
   (assumed from "GitHub Cargo registry credentials"). The alternative is a
   git dependency only, with nothing published. Please confirm which secret
   you set and for which registry.
4. **Licence:** `GPL-3.0-only` (as cordis/kardia; recommended) or
   `GPL-3.0-or-later`.
5. **TUI base:** Codex `rust-v0.93.0` (recommended) or `rust-v0.48.0`.
6. **History:** a fresh import with a provenance commit naming the
   Hanafubuki-dev commit it came from (recommended: simple, and the history
   stays readable in Hanafubuki). The alternative is `git filter-repo`, not
   installed here but available with pip, to carry the crates' history into
   the new repository.
7. **Clippy:** add `-D warnings` after the move, as its own phase-6 commit
   (recommended), or from Phase 1.
8. **Standalone defaults:** the data and config dirs at the OS convention
   (`~/.local/share/hanafubuki-agent`, `~/.config/hanafubuki-agent`)
   (recommended), or a single `~/.hanafubuki-agent` as Codex's `~/.codex`.
