# Agent Plugins

The Agent is composed of plugins on [kardia](https://github.com/licyk/kardia),
the Rust implementation of Cordis. This document holds the rules of that
composition. The workspace rules around it are in
[AGENTS.md](../../AGENTS.md); the composition as built (plugin table, tool
count, flows) is in [spec.md](../spec.md), and how its parts fit together,
with the reasons for the less obvious choices, in
[architecture.md](../architecture.md). A host application (Hanafubuki is
one) embeds the facade and adds its own plugins through
[the host interface](#the-host-interface).

## Crates and dependencies

| Crate | Role | May depend on | Must not depend on |
| --- | --- | --- | --- |
| `hanafubuki-agent-foundation`, `hanafubuki-agent-process`, `hanafubuki-agent-host` | The Agent's own copies of the application's `hanafubuki-foundation`, `hanafubuki-process` and `hanafubuki-module-contracts`, trimmed to what the Agent's crates use (the host: the module host ([modules.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/modules.md#the-module-host)) and the mechanisms its rows share, the registry mechanism (`Registry`, `Register`, leases), `PluginFaults`, `spawn_scoped_worker`, `LogExporter`, `PluginError`; no window bridge). Their types are distinct from the application's ([The host interface](#the-host-interface)) | the process copy and the host on `hanafubuki-agent-foundation`; the host on `kardia-core`, `kardia-macros` | every other project crate, `tauri` |
| `hanafubuki-agent-contracts` | Extension-point contracts: the registry services, the extension points (`ExtensionPoint`), the tool, tool-source, context, turn-admission, prompt, approval-rule, continuation-hook and observer contracts, the `SelectableSkills` view, host services; re-exports the registry, fault and worker mechanisms of `hanafubuki-agent-host` and the foundation's `CancellationToken`, so a plugin depends on this crate alone | `kardia-core`, `kardia-macros`, `hanafubuki-agent-host`, `hanafubuki-agent-foundation`, `hanafubuki-agent-approval`, `hanafubuki-agent-tool-contracts` | `tauri`, `hanafubuki-agent-kernel`, `hanafubuki-agent`, `hanafubuki-application`, any domain service crate, any plugin |
| `hanafubuki-agent-kernel` | Kernel: sessions, turns, the tool pipeline and core tools, model request assembly, world state, Skill selection, history commits, compaction, approval flows; creates the registries it reads (`hanafubuki-agent-skills` only for `SkillId`, the identity persisted in Skill references and Skill context) | contracts, `kardia-core`, `hanafubuki-agent-model-api`, `hanafubuki-agent-skills`, `hanafubuki-agent-approval`, `hanafubuki-agent-tool-contracts`, `hanafubuki-agent-foundation` | any plugin, `tauri`, `hanafubuki-agent`, `hanafubuki-agent-process`, `hanafubuki-agent-web-search`, every Hanafubuki crate outside the Agent |
| `hanafubuki-agent-plugin-<name>` | One capability: binds one domain crate to extension points | contracts, `kardia-core`/`kardia-macros`, `hanafubuki-agent-approval`, `hanafubuki-agent-tool-contracts`, `hanafubuki-agent-foundation`, its own domain crate(s) | `hanafubuki-agent-kernel`, `hanafubuki-agent`, `hanafubuki-application`, `tauri`, other plugins |
| `hanafubuki-agent` | The Agent's composition: the plugin host and manifest, the Agent's own plugins, the slot, its Executor and Skill watcher, the IPC facade, and the host interface ([The host interface](#the-host-interface)) | kernel, contracts, plugin crates, `hanafubuki-agent-config`, `hanafubuki-agent-runtime`, the Agent's foundation, process and host copies | `hanafubuki-agent-model-api`, `hanafubuki-agent-web-search`, every Hanafubuki crate outside the Agent |
| `hanafubuki-agent-config` | The Agent's own configuration (`AgentConfig`): its settings, the Provider registry and the MCP servers, as TOML files in the directory the host names, or standalone in the OS directory with the settings layered ([architecture.md](../architecture.md#10-persistence-and-compatibility), [config.md](../config.md)) | `hanafubuki-agent-mcp` (the MCP settings and their repair), `hanafubuki-agent-foundation`, `toml`, `toml_edit`, `dirs` | the kernel, contracts, plugins, `hanafubuki-agent`, `hanafubuki-config`, `hanafubuki-application`, `tauri` |
| `hanafubuki-agent-runtime` | The Agent's copy of the contained runtime (`ContainedRuntime`, `AGENT_RUNTIME_THREAD_NAME`) | `hanafubuki-agent-foundation` | every other project crate, `tauri` |
| A host's integration (Hanafubuki: `hanafubuki-agent-integration`) | Outside this repository: the host's plugin rows, written against the contracts, and its ports | the facade, contracts, `hanafubuki-agent-approval`, `hanafubuki-agent-tool-contracts`, `hanafubuki-agent-config` (a host's settings import) | the kernel, `hanafubuki-agent-model-api` and every other crate here |

`cargo xtask check-dependencies` checks the Cargo side of this table: no
dependency on a Hanafubuki crate, from any source; every member's internal
dependencies within its row of the layer table (`LAYERS`); kardia only in
`hanafubuki-agent-host`, contracts, the kernel, plugin crates (any member
named `hanafubuki-agent-plugin-*`) and `hanafubuki-agent`, and never the
`kardia` facade (it pulls in WASM/dylib hot reload); `hanafubuki-agent-host`
on no member but `hanafubuki-agent-foundation`; no member but the facade on
`hanafubuki-agent-config`; every plugin on contracts and on neither the
kernel, the facade nor another plugin; and every plugin crate a dependency of
`hanafubuki-agent`, which mounts it. `check-plugin-panics` holds the panic
rule below in the plugin crates (a host checks its own plugins).
`hanafubuki-agent-model-api` may not depend on contracts, and
`check-model-api-context` checks that its production sources name no context
section kind ([Context renderers](#context-renderers)).
The rest is review.

## Composition rules

1. **The kernel depends on nothing a plugin can replace.** kardia restarts a
   plugin when an injected service is replaced, and a kernel restart would
   drop every live session. The kernel plugin injects nothing; it publishes
   registries created before the runtime starts, and the kernel reads them
   as snapshots per turn or per sampling step.
2. **Registrations are effects of the caller.** Contribute through
   `hanafubuki_agent_contracts::Register` on an injected registry service. The entry
   belongs to the calling plugin (`Svc::caller`) and is removed when that
   plugin stops; a plugin never keeps a handle to undo it. A refused
   registration (`RegistrationError`, e.g. a duplicate id) fails the plugin.
3. **Running calls keep their entry's plugin alive, bounded.** A call takes an
   `EntryLease` from a snapshot entry and observes its cancellation. A tool a
   source planned leases its source's entry, and so does the source while it
   contributes. A
   stopping plugin first hides its entries from new snapshots, waits
   `DRAIN_GRACE` for leased calls, cancels them, waits `RETIRE_CANCEL_BOUND`,
   then continues its teardown.
4. **The kernel reaches plugins only through the contracts registries.** A
   hook that changes an outcome is a kernel-owned registry with documented
   ordering and combination (deny-only, tighten-only, first match, ordered by
   priority and id), never a kardia event: the kernel holds no kardia context.
   The result never depends on registration order, because plugins start
   concurrently. A new extension point is added with its first real user.
5. **No long async effect bodies.** kardia never cancels `apply` or an async
   effect, so a loop there blocks its plugin and every ancestor from stopping;
   the host only bounds the damage ([Bounds](#bounds): the row fails after
   `PLUGIN_START_TIMEOUT`, and the fiber stays stuck until its Agent
   instance's runtime stops). `apply` validates config, registers and spawns; long-lived work runs
   through `hanafubuki_agent_contracts::spawn_scoped_worker` (the plugin injects
   `PluginFaults::NAME`, which the kernel plugin provides), or in a worker
   group the plugin stops from a `ctx.defer` disposer (the `execution`
   plugin's activity monitors).
6. **Unchecked root access stays in the host.** Only the module host that
   owns the kardia root (`hanafubuki-agent-host`, configured by
   `hanafubuki-agent`'s `AgentHost`) and the Agent facade read
   plugin state from it. Plugins declare what they use
   (`PluginDef::inject`); domains never see a kardia context.
7. **Plugins return errors, never panic.** Production plugin sources contain
   no `.unwrap()` or `.expect(`; a failing `apply` leaves the plugin `Failed`,
   which the host reports and a configuration update or `retry_plugin`
   restarts. A panic is a bug, but it stays in its plugin (which the host
   restarts after a backoff): kardia catches it
   in `apply`, effect bodies, disposers and event listeners, and the kernel
   guards every call it makes into plugin code
   ([Panics in plugin code](#panics-in-plugin-code)). That holds in release
   builds too: they unwind, and the panic hook lets a panic on an Agent
   thread unwind to its catch ([Where Agent code runs](#where-agent-code-runs)).
   Locks are `parking_lot`, never std ([architecture.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/architecture.md)).
8. **Every kardia mutation runs on the Agent runtime.** kardia spawns on the
   ambient runtime and panics without one; host methods that mount or update
   plugins are async, and their callers send them to the Agent runtime.

## Where Agent code runs

Everything the Agent runs is on a thread where a panic is contained
([architecture.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/architecture.md)): the panic hook lets it unwind to its
catch there, and aborts the process anywhere else.

- **The Agent runtime** (a `ContainedRuntime` of `hanafubuki-agent-runtime`,
  the Agent's copy of the application's mechanism; threads `agent-rt`), one per Agent
  instance: the slot's start builds it and the slot keeps it until that
  instance's stop (shutdown, a restart, a turn off) stops it last
  (`AgentSlot::take_runtime`). It runs every Agent command's
  work (the guard in `src/agent/commands/guard.rs` spawns it on
  `AgentHandle::runtime` and awaits the task), the whole start
  (`hanafubuki-agent`'s `startup`: construction and recovery on its blocking pool, the
  plugins' start, the MCP servers' background start), the async shutdown
  stages (its `shutdown`), the global settings command's
  `AgentSlot::apply_saved_settings`, and what those spawn: turns and compactions (`Handle::try_current` at admission),
  kardia's effects and disposals, scoped workers, the host's fault,
  retire, remount and reconfigure tasks, MCP and model clients. Its blocking
  pool runs `BlockingService` and every `BlockingOperationPort` call of the
  Agent (`ContainedRuntime::blocking_port`), the blocking Agent commands, and the
  ambient `spawn_blocking` of Agent code; on_thread_start marks every thread
  of it contained.
- **Contained by the Agent's own foundation.** These marks and the guards
  below are thread-locals of `hanafubuki-agent-foundation`, which the
  application's foundation cannot see. The root's panic hook
  (`platform::panic_hook`) therefore installs
  `hanafubuki_foundation::panic::install_panic_hook` with the Agent's
  `hanafubuki_agent::panic::is_panic_contained` as its second
  containment check: a panic is contained when either foundation says so.
  Without it every Agent panic would abort the application; the root's
  `platform::panic_hook` tests pin both outcomes in a child process.
- **The Agent's worker threads**, all in `BackgroundWorkerGroup::contained`
  groups: history writers and MCP stdio readers (`AgentApplicationService`'s
  workers), the memory supervisor (whose own current-thread runtime marks its
  blocking pool too), the `execution` plugin's activity monitors, and the
  Skill catalog watcher; and the slot's supervisor thread (`agent-supervisor`,
  marked contained), which runs every restart and turn off.
- **Guards on other threads**: Task Center and diagnostics read the Agent's
  runs, history flush and plugin status through the slot
  (`AgentSlot::list_run_projections` and `get_run_projection`, which the
  integration's `AgentTaskActivities` adapts to Task Activity's
  `AgentActivitySource`; `flush_for_diagnostics`,
  `plugin_status_for_diagnostics`), under `catch_panic`.

Not Agent threads, so their panics still abort: the presentation dispatcher
(application code that holds the Tauri emitter; the Agent's sink only marks a
session dirty from the Agent's thread), and the threads of the services the
Agent calls (an executor's reader and waiter threads, a discovery or
maintenance worker, the launch backend). A new spawn in Agent code lands on
the ambient runtime or in a contained group, never on a plain thread or
`tauri::async_runtime`.

## Host, configuration and status

- The Agent is optional to the application: the feature module `agent`,
  whose service is its slot ([modules.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/modules.md#the-agent-module)). It
  is built after the interface
  opens, on the runtime its start builds for it, from the launch composition
  captured (`AgentLaunch`: a reusable `AgentConstructor`, which receives that
  runtime for its blocking port and the instance's `AgentInstanceInputs` —
  start options, kernel fault sink, and the Agent's own Executor scoped to
  the instance's process owner group — and builds `AgentServiceInputs`, the
  host's ports among them ([The host interface](#the-host-interface)); and
  the path of the Agent's process-group ledger). `DesktopAgent::start` adds
  the Agent's own `ExecutorManager` (over the child-process environment
  `DesktopAgent` keeps) and its Skill watcher, which restarts on each running
  instance's roots and stops first at shutdown; the launch is held in the `AgentSlot`
  (`Disabled { setting | crashLoop }`, `Starting`, `Running`, `Faulted`,
  `Stopping`, `Failed { stage, reason }`, `Stopped`). The slot keeps the
  launch, so every later instance (a restart, an automatic restart, a turn
  on) starts the same way, each with the next `generation`. The start reaps
  the process groups an earlier run left
  ([lifecycle.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/lifecycle.md#process-ownership)), then reaches `Running`
  once its plugins started; the MCP servers connect afterwards in the background,
  concurrently (`MCP_CONNECT_CONCURRENCY`, 4), each bounded by its own
  connect and initialize timeouts, and each server's status says how it went.
  Shutdown and a turn off cancel a start under way (`AgentSlot::close`,
  `AgentSlot::disable`): construction and recovery block and cannot be
  cancelled, so the start checks between stages; the plugins' start is
  cancelled where it stands; then the start retires what it built within
  `RETIRE_BOUND` (5 s), less than the wait shutdown gives it, so nothing it
  started outlives its runtime. A construction, recovery or start-up failure leaves the slot
  `Failed`; every Agent command then returns "the Agent is unavailable:
  <reason>" through its guard. `agent_status` and `agent://status-changed`
  report the slot, with the generation, whether the instance runs in safe
  mode, whether it is degraded (a plugin `failed`, its teardown stuck, or
  an active one with a `warning`, derived from the plugin status, whose changes `agent://plugins-changed`
  announces) and the quarantined sessions. A built Agent that will not run (its recovery or start failed, or
  shutdown overtook it) is stopped in shutdown order before the slot says
  so; a construction that fails part-way stops the worker groups it started.
- The slot supervises the Agent (`slot/supervisor.rs`). A fault is either a
  plugin's or the kernel's. A panic in plugin code is the plugin's
  ([Panics in plugin code](#panics-in-plugin-code)): the plugin is stopped,
  `failed`, and started again by the plugin host under the restart policy
  below, and the Agent keeps running. A panic
  that reaches a kernel-level catch is the kernel's: a turn's task
  (`execute_admitted_turn` and the managed turn's task, reported through the
  `AgentKernelFaultSink` the kernel is built with), an Agent command's guard
  (its task's `JoinError::is_panic`, a synchronous body's `catch_panic`), a
  member of the Agent's worker groups (history writers, the memory
  supervisor, MCP stdio: `BackgroundWorkerGroup::contained_with_panic_handler`),
  and the host's and service's own tasks (the fault follower, a remount, the
  settings follow-up, the MCP start: `spawn_kernel_task`). Kernel code that
  panicked may have left in-memory session state half-changed — the locks
  do not poison, so nothing rolls back — so the slot rebuilds the Agent from
  disk: every report goes to the one sink of the instance's start
  (`AgentSlot::fault_sink(generation)`; a report of an instance that no
  longer runs is ignored), the slot marks the Agent `Faulted { reason, at,
  restartsInWindow, nextRestartAt }` at once and stops it, and starts a new
  one after 2 s, 10 s, 30 s for the first, second and third automatic
  restart in a 10-minute window (`AgentRestartPolicy`, the module host's
  `RestartPolicy`); a fourth fault in the
  window leaves it `Faulted` with no `nextRestartAt` until the user restarts
  it, which also empties the window. A kernel fault reported while an
  instance is still starting fails that start instead. An automatic restart
  keeps the instance's safe mode.
- The repair commands act on the slot, need no running Agent and return once
  the transition is scheduled (`agent_slot_command`): `agent_restart`,
  `agent_restart_safe_mode` (this instance starts with every row outside the
  core off; `disabledPlugins` is not changed, and enabling a plugin is
  refused until a normal restart), `agent_rebuild_history_index` (the history
  index is discarded and rebuilt from the rollouts before recovery,
  `FileAgentHistoryStore::open_rebuilding_index`). The Agent is turned off
  and on as the `agent` application module (`disabledModules`,
  `app_set_module_enabled`; [modules.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/modules.md#the-agent-module)): the
  module's stop turns it off in order (`AgentSlot::turn_off`,
  `Disabled { setting }`), a mount turns it on (`AgentSlot::enable`), which
  leaves a crash-loop stop alone. A restart is refused while the Agent
  starts, while its module is off and while the application shuts down. A
  user's restart or turn on resets the crash-loop marker
  (`data/agent/start-marker.json`), so the way out of a crash loop is a
  restart or turning the module off and on. `agent_reset_memory` stays the
  one user-triggered way to discard memory.
- A restart stops the old instance with shutdown's sequence and budget
  (`stop_instance`), sweeps its process owner group, stops its runtime, then
  starts the next instance. A call in flight on the old instance has its task
  dropped with that runtime, and its guard returns "the Agent restarted;
  <command> did not finish" (`AgentSlot::interrupted`; "the Agent was turned
  off", or "stopped after an internal error" when no restart follows), as does
  a call whose Agent returned an error once it no longer runs. Holders
  resolve the slot per call (Task Center, diagnostics, the settings command,
  the Agent's Skill watcher), so no old Agent stays reachable.
- A capability that cannot come up fails its plugin, not the Agent: a Skill
  catalog that did not open fails the `skills` plugin (and every Skill call
  returns why), a required MCP server that did not connect is its MCP status,
  a `web` configuration that does not derive is that row's error. A session
  recovery rejects, or one the history store could not admit as it opened
  (an unreadable rollout, item or attachment, a misplaced file), is
  quarantined (logged, listed in the status, files kept); the store opens
  and the Agent runs.
- `AgentHost` is the Agent's configuration of the generic module host
  (`AgentPlugins`, its `HostProfile`): the mount order, bounds, enablement,
  reconfiguration, retry, safe mode, fault handling, status and shutdown are
  the host's, defined once in [modules.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/modules.md#the-module-host).
  What is the Agent's: its rows, its configuration from its own settings
  and the host's proxy (`AgentPluginSettings`), its
  enablement from `disabledPlugins`, its groups, the contributions in
  its status, its `AgentPluginEventSink`, and the kernel fault sink a panic
  in a host task reaches (which rebuilds the Agent, below). Its options
  (`ModuleHostOptions`): `module_faults` off, since its rows are plugins,
  not application modules, and their faults reach it through its
  `PluginFaults` alone; `automatic_restarts` the application modules' policy
  (`AgentHostOptions::restart_policy`, `RestartPolicy::default()`: 2 s,
  10 s, 30 s, three in ten minutes), so a plugin a fault stopped starts
  again by itself, and after those stays `failed` until `retry_plugin`.
- `AgentHost` mounts, in order, `agent-host-services` (the immortal
  `BlockingService`, the sanctioned blocking boundary), `agent-kernel` (the
  `AgentKernelRegistries`) and every enabled row of `agent_plugin_manifest`,
  the host's rows among them (in safe mode, `AgentHostOptions::safe_mode`, no row outside the core is
  enabled, at start-up or by a later settings change),
  each after the previous one settled or timed out ([Bounds](#bounds)),
  keeping one fiber per row. The Agent's start starts it, on the Agent
  runtime, before the MCP servers; a model operation starts it if nothing
  did.
- A row is `AgentPluginRow::unconfigured` or `AgentPluginRow::from_settings`
  (a `ModuleRow` of the Agent's profile) with a function deriving its
  configuration from `AppSettings`, and declares
  its `AgentPluginGroup`: `Capability` (tools, tool sources and their rules)
  or `Context` (world-state sections). `Core` is the two fixed rows'.
  Configuration that is not derived from settings does not exist; a service a
  plugin adapts is captured in its `PluginDef` constructor (through
  `AgentPluginInputs`), not passed as configuration.
- A settings-derived plugin follows the settings while the application runs.
  The settings command schedules `AgentApplicationService::apply_settings_to_plugins`
  on the running Agent's runtime after it persists a patch
  (`AgentSlot::apply_saved_settings`) and returns; the host reconfigures its
  rows from the latest published settings
  ([modules.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/modules.md#the-module-host), Settings). `update` also clears
  a `Failed` state. Nothing waits for the restart (its outcome is
  logged) or for the MCP proxy follow-up, which reads the latest settings
  under the MCP configuration gate and reconnects the Streamable HTTP
  servers concurrently in the background. Since each step reads the latest
  settings under its own lock, two quick changes apply in order whatever
  order their work runs in.
- Enablement is a setting: `disabledPlugins` (the Agent's `config.toml`) holds the names the user
  turned off (unknown names are kept and ignored; a core row ignores it);
  it is the Agent profile's `disables`, which the host's `reconfigure`
  follows: a re-enabled row mounts only once its previous fiber stopped, so
  its registrations cannot collide with the old ones. A disabled row is not
  mounted at start-up. `AgentApplicationService::set_plugin_enabled`
  is the one writer: it refuses an unknown name and disabling a core row,
  writes the set through `AgentConfig::set_plugin_disabled` and applies the
  settings.
  Disabling a plugin is not a way to deny a tool: approval stays with the
  rules and the permission mode.
- A plugin that absorbs a change without restarting registers a kardia update
  hook (`Context::on_update`); `Fiber::update` offers the new configuration
  to it first. Otherwise the plugin restarts, which drains its running calls.
- `AgentApplicationService::plugin_status` returns every plugin's
  `AgentPluginStatus`: the host's `ModuleStatus` (name, group,
  `pending`/`active`/`failed`/`disabled`/`disposed`, error text, the
  `warning` of an active plugin part of which the kernel left out of turns
  (its `PluginFaults` issues, see [Context](#context)), enabled, lock, and
  `restart`: a pending automatic restart, or the ones the window counts,
  which the plugin manager shows as the Modules page does) and
  its current contributions, read from the registry snapshots by entry
  owner before the host's row lock; a warning is read with the status) and `retry_plugin` restarts a
  `Failed` plugin with its current configuration, bounded like every start. Both are cheap in-memory
  reads or kardia calls, exposed to Vue as `agent_list_plugins`,
  `agent_set_plugin_enabled` and `agent_retry_plugin`; the host tells its
  `AgentPluginEventSink` (`agent://plugins-changed`) whenever a row changes,
  a change settles, a disabled row's teardown ends or a plugin's issues
  change (`PluginFaults::set_issues_listener`: only a new, changed or
  cleared issue, so a contributor failing the same way every turn announces
  nothing more), and Vue re-reads. App diagnostic bundles carry the
  status as `app/agent-plugins.json`. After start-up the host logs every
  enabled plugin that is not `Active`; the Agent runs without its
  contributions.
- Shutdown disposes the kardia root once, bounded by the caller's deadline
  (the plugins' stage of the Agent's shutdown budget,
  [lifecycle.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/lifecycle.md#runtime-readiness-and-shutdown)); a plugin
  still stopping then is reported as an `agent-plugins` shutdown diagnostic
  and dropped with the instance's runtime. MCP connections, memory workers,
  history and executions are owned by the application and stop in its own
  shutdown steps, never in a plugin's teardown.

### Bounds

The host bounds every wait it makes on kardia: a start within
`PLUGIN_START_TIMEOUT` (20 s), a disposal within `PLUGIN_STOP_BOUND` (10 s),
a remount within the disposal's bound; a core row that does not start in
time stays mounted. The rules and messages are the module host's
([modules.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/modules.md#the-module-host), Bounds); rule 5 relies on them. A
fiber stuck for good is dropped with its Agent instance's runtime at
shutdown (or a restart's hard stop).

## The host interface

An embedding application (the host) adds to the Agent only through
`AgentHostPorts`, one field of `AgentServiceInputs`, built for every Agent
instance (its rows are mounted once). Hanafubuki's side is
`hanafubuki-agent-integration` (`agent_host_ports`), described in its
[agent-integration.md](https://github.com/licyk/Hanafubuki/blob/main/docs/agent-integration.md);
the Agent depends on none of its types. The interface takes plain values (`PathBuf`, strings,
maps) or the Agent's own types, never the application's foundation,
process or module host types: they are distinct types on the two sides, and
the host adapts at the edge (the Agent's shutdown
diagnostics, for one, are mapped to the host's at the call). Beside the ports the host names the Agent's two
directories: `AgentServiceInputs::config` (an `AgentConfig` on the
configuration directory; Hanafubuki's is `<app_data>/config/agent/`) and
`storage_paths` (`AgentStoragePaths` under the data directory; Hanafubuki's is
`<app_data>/data/agent`).

- **Host plugin rows** (`plugins: Vec<AgentPluginRow>`, built with
  `AgentPluginRow::unconfigured` and a group). They are plugins like any
  other: written against contracts, every rule of this document applies,
  `disabledPlugins` turns them off, safe mode leaves them unmounted. They
  mount after the Agent's capability plugins and before its context plugins,
  so a host's world-state sections come before the workspace roots,
  environment and permission mode (only the kernel's
  `tool_catalog_omissions` comes first). In Hanafubuki: `tasks`, `instances`,
  `ui-context`.
- **`WorkspaceRootsPort`** (`workspace`): the roots (`roots()`, read again
  for every turn and every Skill catalog read; the first is the base of
  relative requests), the directories the filesystem tools may also read
  (`extra_read_roots()`), the directory a turn's host input selects for
  instruction discovery (`selected(&TurnExtensions)`) and the labelled
  directories file search walks (`search_roots(&TurnExtensions)`). In Hanafubuki
  (`InstanceWorkspaceRoots`): the managed instance root and every instance's
  install directory, the application's data and logs, and the instance the
  UI context names.
- **`HostContextPort`** (`steering_context`): the host context a steering
  message carries, an `AgentHostContext` (a `ContextSectionKind` and its
  content, at most `MAXIMUM_HOST_CONTEXT_BYTES`), rendered on the Agent's
  blocking pool from the steering's host input. The kernel writes it to the
  world state as a section of its kind when it changed and handles it like
  any other section; the host's plugin registers that kind's renderer. In Hanafubuki
  (`UiSteeringContext`): the UI context, kind `application_ui_context`.
- **`NetworkSettingsPort`** (`network`): the proxy the Agent's web tools,
  MCP servers and OAuth use (`AgentProxy`: system, disabled or a URL), read
  at start and whenever the host calls `AgentSlot::apply_saved_settings`;
  it answers from memory. In Hanafubuki (`ApplicationNetworkSettings`): the proxy of
  the application's network settings (`ConfigState`'s snapshot).
- **`SecretStore`** (`secrets`, the Agent foundation's, re-exported by
  `hanafubuki-agent` with `BlockingOperationPort` and
  `run_blocking_operation`): where MCP OAuth tokens are kept, under the same
  entry names as before. In Hanafubuki: the root's `KeyringSecretStore` (the OS
  keyring), on the Agent runtime's blocking pool.
- **The child-process environment** (`DesktopAgent::new`'s
  `inherited_environment`, and `DesktopAgent::replace_child_environment`):
  what the Agent's processes inherit in place of this process's environment
  and the overlay applied last, as plain maps. In Hanafubuki: the application's
  inherited overlay (`ChildProcessEnvironment::inherited_snapshot`), and the
  proxy overlay, which the root's proxy target replaces in both
  environments at once (`runtime::proxy`).
- **`AgentHostInstructions`** (`instructions`): who the Agent is and what
  the host adds to the base instructions. `identity` is the sentence that
  opens them (`{identity}` in `prompts/base.md`) and `application` the name
  the environment section gives (`"application"`); `sections` go after the
  tool guidance and `ownership` closes "Approval and ownership", each joined
  one blank line from its neighbours. The default is the standalone Agent's:
  "You are a coding and operations agent.", `hanafubuki-agent`, no
  additions. In Hanafubuki: "You are the desktop application's coding and
  operations agent.", `Hanafubuki`, the application context section and the
  ownership paragraph (`src/prompts/` of the integration).
- **Host input**: `AgentTurnRequest`, `AgentSteeringRequest` and
  `AgentFileSearch` carry what the host submitted beside a message or a
  search as `TurnExtensions` (`host`), which the Agent passes on unread: to
  the context contributors (`ContextRequest::turn`), to `HostContextPort`
  and to `WorkspaceRootsPort`. The IPC inputs that carry the UI context are
  the host's (Hanafubuki's `DesktopAgentTurnInput`, `DesktopAgentSteeringInput`,
  `AgentFileSearchRequest`), converted with `From` in the thin command.
- **The data directory's lock** (`<data_dir>/.lock`): held from the
  Agent's first start until its shutdown, in both modes. A host does
  nothing for it; its Agent fails to start (and the host keeps running)
  while another process holds the directory.
- **The Agent's runs** for a host's activity view: `AgentSlot::list_run_projections`
  and `get_run_projection` (`AgentRunProjection`: activity id, turn number,
  `AgentTurnPhase`, times, error), guarded. The host maps them to its own
  view (Hanafubuki's `AgentTaskActivities`, Task Activity's `AgentActivitySource`).

Without a host, `AgentRuntime::standalone` supplies every port itself
([architecture.md §3.6](../architecture.md#36-standalone)): the working
directory and its Git root as the workspace, no plugin rows, context or
instructions, the configuration's `[network]` proxy, the OS keyring. A
host's composition keeps `DesktopAgent`, `AgentLaunch` and
`AgentHostPorts`, which the standalone runtime composes too.

## Extension points

| Registry (contracts unless noted) | A plugin registers | Unique by | The kernel reads it | Combination |
| --- | --- | --- | --- | --- |
| `ToolRegistry` | `AgentToolHandler` | flattened tool name, none reserved ([Tool names](#tool-names)) | per sampling step, with the tool plan | the kernel pipeline |
| `ToolSourceRegistry` | `AgentToolSource` | source id | per sampling step | planned after core and registered tools; a failing source by its trust |
| `ContextRegistry` | `ContextContributor` | contributor id, and one contributor per kind | at turn admission | collected by kind name; shown by renderer slot |
| `ContextRendererRegistry` | `ContextRenderer` (data, no plugin code) | section kind | snapshot per model request | slot, then registration order ([Context renderers](#context-renderers)) |
| `TurnAdmissionRegistry` | `TurnAdmissionContributor` | contributor id | at turn admission | merged into the turn's `TurnExtensions` by rank and id; a contributor that fails or returns a type already returned is left out |
| `PromptContributorRegistry` | `PromptContributor` | contributor id | snapshot per sampling step, prepared once per step, asked per model request | prepared values merged by rank and id (a repeated type is left out); fragments join the sections the conversation opens with, by renderer slot |
| `ApprovalRuleRegistry` | `ApprovalRule` | rule id | frozen with each step's tool plan | deny > require user > the session's allowance > the mode |
| `ContinuationHookRegistry` | `AgentContinuationHookRegistration` | (provider id, hook id); at most `MAXIMUM_CONTINUATION_HOOKS` | per sampling step | by priority, provider id, hook id |
| `ObserverRegistry` | `AgentObserver` | observer id | after each durable commit | all, in commit order |


### Panics in plugin code

The kernel calls plugin code only through a guard: `RegistryEntry::guard`
or `guard_async` (the entry knows its owner), or `PluginFaults::guard` and
`guard_async` where no entry is at hand. A panic there is logged at error,
reported to `PluginFaults` as a `PluginFault` of the entry's owner (plugin,
`ExtensionPoint`, bounded panic message), and becomes `PluginPanicked`,
which the caller turns into that call's own error path:

| Call | A panic means |
| --- | --- |
| tool `availability` (registered or sourced) | the tool is `Omitted` from the step |
| tool `permission`, `invoke` | the tool error "the tool failed with an internal error", as any failing tool |
| tool source `contribute` | the source failed: its trust decides (an `Optional` one is omitted) |
| context and turn admission `contribute` | that contributor is left out of the admission (as for an error, below) |
| prompt `prepare` | the contributor's values are left out with a warning |
| prompt `contribute` | the contributor renders no fragment |
| approval rule `evaluate` | a denial naming the plugin: a rule fails closed, never "no opinion" |
| continuation hook | the hook failed: its failure policy decides |
| observer `observe` | that observer is skipped; the others are told and the committing thread never unwinds |
| a `spawn_scoped_worker` body | the worker ends, logged with its label |

The panic message reaches the log and the plugin's status, never the model.
The `mcp` plugin contains one panic itself: `McpTool::invoke` runs its
server's call under `catch_panic_async`, so a panic there is that call's
"the tool failed with an internal error" and that server's fault
(`McpManager::fail_after_panic`: `Failed` with `the MCP client panicked
during a tool call: <message>`, then reconnected as below), never the
plugin's; the other servers and the plugin keep running. A panic anywhere
else in the plugin is still the plugin's fault. A child fiber per server was
rejected: a named child faults under a name the host's fault handler does
not know, and an unnamed one faults as `mcp`
([architecture.md](../architecture.md#12-design-choices-and-alternatives)).
The kernel never holds a lock across a guarded call, so the guards change no
lock ordering. The identity accessors (`id`, `descriptor`, `kinds`, `rank`,
`trust`) are data and are not guarded.

One `PluginFaults` is shared by the kernel's registries (`AgentKernelRegistries::faults`,
copied into every entry) and published by the kernel plugin; it is the one
the Agent's host follows. An `ExtensionPoint` converts into the
`FaultPoint` a fault carries. What follows a fault is the module host's
([modules.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/modules.md#the-module-host), Faults, Automatic restarts): the
plugin is stopped and reports `failed` with `panicked in <extension point>:
<message>`, and the host mounts it again by itself after the restart
policy's next delay while its window allows; past that, until
`retry_plugin`, a new configuration, or turning it off and on again mounts it
anew, once its teardown has ended. A plugin whose start failed is not
restarted by itself. A fault for a core row or an unknown name is only
logged.

### Tools

- A tool's `ToolDescriptor` fixes the model-visible spec, exposure, execution
  class, effect (`ReadOnly` tools, registered or sourced, are the only ones in
  Plan mode), activity action, memory effect and timeout for the lifetime of
  the registration (for a sourced tool, of the step).
  `availability` may offer it as a freeform tool or omit it for a step
  (`ToolPlanContext`).
- The kernel owns the pipeline and plugins cannot reorder it: mode policy,
  `permission`, approval when review is required (the approval rules, then
  the session's allowance, then the permission mode's flow), then `invoke`
  with the one-shot approval. `invoke` re-checks that the approval matches
  the call it runs.
- The pipeline runs under the step's gate: an in-flight permit, then read
  access for a `ConcurrentRead` tool or exclusive access, admitted first
  come, first served, so `permission` sees what the step's earlier calls
  did. An approval that waits (the user, or the reviewer model) releases the
  gate and takes it again once approved; the step's other calls run
  meanwhile, and the user is asked one prompt at a time per step. An
  approved action can be stale once other calls ran in the gap; the
  re-check in `invoke` (`verify_tool_approval`, a domain's own check) turns
  that into the call's error, never a run of something unapproved. A call
  admitted after its turn was cancelled does not start.
- `invoke` runs within `ToolDescriptor::timeout`, or the turn's
  `tool_idle_timeout` (10 min) when it is `None`; neither `permission` nor
  an approval wait counts. A call past it has its cancellation cancelled and
  its future dropped, and the model reads "the tool did not finish within
  N s and was stopped"; the turn continues. Work the call left on the
  `BlockingService` observes the cancellation and ends itself. A tool whose
  own bound pauses while it waits for the user sets the timeout above that
  bound: an MCP tool's is its server's call timeout plus one elicitation
  wait (`MAXIMUM_MCP_ELICITATION_WAIT`).
- Outputs commit in call order, each as soon as every earlier call's has
  (`tool_execution/scheduler.rs`). A call that ends in an error (the turn's
  cancellation among them) commits nothing; the step's other calls still
  run to their end and commit, since their effects happened, then the step
  returns the first error. On a cancel only the unfinished calls get the
  synthetic interrupted output (`validation/tool_pairs.rs`), which the model
  reads right after its call.
- A turn samples at most `AgentHardLimits::maximum_sampling_steps` (32)
  times: the next step fails it with `LimitExceeded { "sampling steps" }`
  (terminal reason `budget_exceeded`), with every earlier output committed.
  At `turn_deadline` (30 min) the kernel cancels the turn as a user's cancel
  would (`request_turn_cancellation`: pending approvals and questions
  aborted, finished calls committed) and ends it `Failed` with
  `TurnDeadlineExceeded` (`deadline_exceeded`), whose message the user
  reads. Neither limit is shown to the model.
- Arguments arrive as the model's JSON, after one kernel pass guided by the
  tool's own input schema (`tool_catalog/arguments.rs`, before mode policy,
  `permission` and approval, which all see the result; history keeps what the
  model sent): `1.0` and a numeric string become numbers where the schema
  says `integer` or `number` (and not `string`), a `null` for a property the
  schema neither requires nor declares nullable is removed, and a key it does
  not declare is removed where it says `additionalProperties: false`. The
  handler validates them once (`decode_arguments`, `object_arguments`, the
  `tools::arguments` helpers), returning `ToolCallValidationError`, whose
  texts the model reads back. See [Model tool arguments](#model-tool-arguments). Blocking work
  runs on the injected `BlockingService`; a call observes
  `ToolInvocation::cancellation`. `ToolInvocation::progress` only queues
  (`report` never waits), from the call or from a monitor thread that
  outlives it; the kernel commits the session's progress in order.
- A tool that exists only while something outside the Agent offers it (an
  MCP server's catalog, the memory settings) comes from an `AgentToolSource`.
  The kernel asks each source once per sampling step with a `ToolSourceStep`
  (session, turn, generation, session revision, memory contribution,
  cancellation). It returns `SourcedTool`s: an `AgentToolHandler` built for
  that step plus the identity the plan records (provider id, tool id,
  generation, namespace description, activity subject) and its
  `ToolSourceTrust`. Keep the identity stable while the tool does not change,
  and change it when the tool does: a deferred-tool selection (the
  `ToolSelection` history item) names `(provider id, tool id, generation)`,
  so a selection whose tool is gone or changed matches nothing and is
  ignored, and the kernel validates a
  tool's schema and provider spec once per identity (`AgentToolSpecCache`),
  not on every step.
- A sourced tool is planned and run like a registered one, with two
  differences: the kernel checks the model's arguments against its schema
  before `permission`, and its trust is external: an `Optional` tool that
  cannot be planned is omitted (reported in `tool_catalog_omissions`), a
  `Required` one fails the plan. A name collision is neither (see
  [Tool names](#tool-names)). A source
  whose plugin stops while it is asked (turned off, or shutdown) has left the
  configuration and is omitted whatever its trust, so turning a plugin off
  never fails a step. A source's own
  `trust` decides the same for a failing `contribute`. Over the direct
  catalog budget only sourced tools are deferred.
- A tool source may return a `ToolPlanLease` with its contribution; the plan
  keeps every lease and each call receives them as
  `ToolInvocation::plan_leases`, so a source's tools run against the state
  they were planned from. Find yours with `ToolInvocation::plan_lease::<T>()`,
  never by downcasting the `Arc`.
- Per-turn data reaches a call as `ToolInvocation::turn` (`TurnExtensions`,
  keyed by type): what the turn admission contributors froze for the turn
  (the `skills` plugin's `hanafubuki_agent_skills::SkillTurnCatalog`). Read yours with
  `turn.get::<T>()` and treat a missing value as a tool error. Contracts name
  only the values the kernel itself reads (`SelectableSkills`).
- A tool adds instructions to the conversation by returning a
  `ToolContextFragment` with its `ToolResult`. The kernel commits it right
  after the output as the context of the Skill `source` names, once per turn
  and Skill, and turns a source that is not a Skill id into a tool error.
  That context is the one delivery of a Skill's `SKILL.md`: `skills_read`
  without a `path` returns it and an output that only says where the
  instructions are, and a Skill a `$mention` already selected in the turn
  is not committed again. Of the history it compacts, a compaction keeps
  neither tool outputs nor Skill context (only user messages, the summary
  and the re-rendered sections), so the choice between them did not depend
  on it.
- The kernel's own tools (`AgentCoreTool`: `new_context`, `update_plan`,
  `request_user_input`) act on the turn and are not plugins; neither is
  `tool_search`, the plan's search over deferred tools. Their names are
  `RESERVED_TOOL_NAMES`.
- An activity action is an open identifier (`AgentActivityAction`); Vue labels
  the names it knows and shows the generic tool label otherwise, so a new tool
  needs no frontend change.

### Tool names

- A tool's name is its `ToolName` (namespace segments and a local name). The
  registry and the planner key it by its flattened form
  (`namespace__name`); that string is never parsed back.
- `ToolRegistry::admit` refuses a registration whose flattened name is
  reserved (`RESERVED_TOOL_NAMES`), already registered, or longer than a
  tool id (128 bytes). The `RegistrationError` fails the registering plugin
  at its start, with nothing registered; the other plugins and the turns are
  unaffected.
- The wire name is per request (`tool_catalog/provider_table.rs`):
  `assign_provider_names` writes it into each planned tool's provider spec
  when the plan is built for the step's protocol. Where the protocol takes
  namespaces (Responses) a namespaced function keeps its namespace and local
  name; elsewhere, and for a namespace another provider already uses, the
  name is flattened. Every name is fitted to
  `ToolProviderCapabilities::maximum_tool_name_bytes` (Responses 128,
  counted over `namespace__name`; Anthropic Messages 128; Chat Completions
  64, OpenAI's function-name limit).
- Names are unique over their flattened form, with `tool_search` held for
  the plan's search. In planning order (trust, then name, provider id, tool
  id) the first tool keeps its name; a later one that collides, or one too
  long, is cut and ends with `_` and 12 hex digits of a hash of its
  `(provider id, tool id)`, so it is the same on every step. A collision
  (only a sourced tool can collide, since registration refuses the others)
  is reported in `tool_catalog_omissions` with the reason "renamed to
  `<name>`"; the tool stays planned. Nothing about a name fails a step.
- A returned call is resolved by lookup of its `(namespace, name)` in the
  plan (`AgentToolPlan::resolve_call`); a name the request did not carry is
  an unknown tool (the error below). The decoders return the provider's
  namespace and name apart (`ModelToolCall::namespace`), and history records
  both, so Responses replays a call with its namespace; a protocol without
  namespaces replays one as `namespace__name`
  (`flattened_tool_call_name`, which the permission-review and memory
  evidence use too).
- Schemas are lowered by the same per-protocol rule: a protocol whose
  `strict_json_schema` capability is set (Responses) gets each tool's schema
  whole, constraints included; one without it gets the portable keywords
  (`portable_model_schema`): Anthropic Messages, and Chat Completions, the
  protocol of every OpenAI-compatible endpoint, some of which refuse the
  other keywords. Tools are
  never sent `strict`. The kernel validates arguments against the whole
  schema either way.

### Model tool arguments

The model's arguments are read tolerantly; a mistake is a field-named error
the model reads back as the tool result, never a failed turn.

- No argument type uses `deny_unknown_fields`, and no helper rejects a key
  it does not know: an unknown key is ignored. Ignoring it never widens what
  runs, because capabilities (approvals, receipts, resolved paths, executor
  ids) are never arguments, and the call still goes through `permission`
  and approval. The one exception is a value meant for another action of
  the same tool: `provision_instance` rejects a non-null property its
  selected action does not take (`reject_other_action_arguments`), since
  its actions create and delete instances.
- A missing or `null` optional field is absent (`nullable_*` and
  `optional_*` alike: strict-mode schemas list every property as
  required-and-nullable, but omitting one means the same). A field that is
  semantically required (`cmd`, `path`, `recursive` of `delete_path`) stays
  required.
- A limit out of range is clamped into its bounds with a note
  (`clamped_u64`/`clamped_usize`, `ArgumentNotes`, appended to the result by
  `ToolResult::with_argument_notes`); a limit that guards safety (a byte or
  depth cap) clamps to the cap and never past it, and an omitted one takes
  its default. Containment is not a limit: a path is passed as given and the
  domain resolves and contains it.
- `decode_arguments` names the field an error is about
  (`invalid tool argument plan[1].status: ...`, via `serde_path_to_error`).
- A sourced tool's arguments are checked against its schema
  (`validate_dynamic_tool_arguments`), which the plan already validated, so
  a call does not validate the schema again; a `type` the check does not
  know constrains nothing, and an integral number is an `integer`.
- A call the step cannot run is answered, never a protocol failure, and its
  output keeps the call/output pair valid for every provider protocol:
  - a name the step does not offer (unknown, `Hidden`, mode-hidden,
    provider-incompatible) gets one identical error, `no tool named `<name>`
    is available in this step`, so a hidden tool stays indistinguishable
    from an absent one, and never runs; history records it under the
    `unavailable` provider and tool id;
  - a Deferred tool not yet admitted gets the discovery error with the
    `tool_search` recovery;
  - arguments a provider decoder could not parse
    (`ModelToolCall::argument_error`, with `{}` as the arguments) or over
    `maximum_tool_argument_bytes` get "the tool arguments could not be
    used" with a 512-byte preview of what was sent;
  - a malformed `tool_search` gets a field-named error, and an out-of-range
    `limit` is clamped with a note.
- A `tool_search` commits its output, then a `ToolSelection` history item
  with the identities it selected (no item when it matched nothing). The
  planner admits the selections of the history since the last compaction,
  which keeps no `ToolSelection`, so a selection lasts across turns until
  then and the model searches again after it. The item is not model input:
  a provider without native search sees the selected tools as Direct in the
  plan, one with it replays the search output and may call them.

### Context

- A `ContextContributor` declares the kinds it produces (`ContextSectionKind`,
  an open identifier with the persisted snake_case names) and returns at most
  one `ContextSection` per declared kind. Two contributors declaring one
  kind, or a contributor declaring `tool_catalog_omissions` (the kernel writes
  it per sampling step), fail the registering plugin.
- At turn admission the application collects the sections from one registry
  snapshot (`hanafubuki_agent_kernel::collect_world_state`; contributors run one after
  another under an entry lease). A contributor that fails (an error, a
  panic, its plugin stopping) is left out of that turn; so is a section of a
  kind it did not declare, a kind already produced, or a plugin kind past
  `MAXIMUM_AGENT_PLUGIN_WORLD_STATE_SECTIONS` (7, a host's kinds among them;
  the five known kinds have their own slots and never use it up). Each is recorded as an issue of its plugin
  (`PluginFaults::set_issue`), which the plugin's status shows as its
  `warning` (and the Agent as degraded) until that contributor succeeds
  again or the plugin is mounted anew; the turn is never refused over it. A kind no contributor returns is
  removed from the world state on that turn, so a contributor plugin that
  failed to start leaves its sections absent rather than failing every turn.
- The submitted UI context (the integration's `application_ui_context`)
  never refuses the message or steering it came with: one this build cannot
  use (another schema version, an unusable instance id) is dropped with a
  warning, and one over `MAXIMUM_HOST_CONTEXT_BYTES` (4 KiB, the kernel's
  bound for a host context) is cut on a UTF-8 boundary.
- The result is collected in kind name order, so history records it the same
  way every time; the model is shown it in its renderers' order
  ([Context renderers](#context-renderers)). `ContextSectionKind::KNOWN`
  names the Agent's own kinds only for the section bound.
- A `ContextRequest` carries the session id, the session mode, the turn's
  permission mode and `TurnExtensions` for the host's inputs (the
  integration's `DesktopAgentUiContextSnapshot`), which the Agent passes on
  unread. Treat a missing value as "not supplied for this turn". Long work
  runs on the `BlockingService`.
- Text rendered from JSON goes through `ContextSection::from_json`, which
  sorts object keys so the version digest is stable.
- The `workspace-context` plugin's `workspace_instructions` section holds
  the user's own `AGENTS.md` (or `AGENTS.override.md`) from the Agent data
  directory (`AgentStoragePaths::root`, `data/agent/`), then each workspace
  root's from the root down to the selected instance. The user's file is one
  more group under the same 32 KiB budget (a fair share each, at most 16 KiB
  per group), cut and warned about in `environment` like the rest; a missing
  file or directory adds nothing.

### Context renderers

- Everything model-visible that is neither history nor a tool is a context
  section of a `ContextSectionKind`: a world-state section (persisted, from
  a `ContextContributor`) or a prompt fragment (per request, from a
  `PromptContributor`). The model is shown a section by the
  `ContextRenderer` of its kind, which the plugin that owns the kind
  registers in the `ContextRendererRegistry` (one per kind; an invalid or
  repeated kind fails the registering plugin). A renderer is data: the
  `ContextSlot`, the `ContextRole`, a `preamble` (what the section is and
  what authority it lacks), an `epilogue`, and `versioned`, which puts the
  content in an `<agent_world_state kind=".." version="..">` element. The
  wrapper text of a kind lives in the plugin that owns it, never in
  `hanafubuki-agent-model-api`, which sends a fragment's role and rendered text as
  they are and matches on no section kind. Sections without a framing of
  their own use `WORLD_STATE_PREAMBLE`. The kernel frames the context items
  that are history, not sections: a compacted window's initial context, the
  task checklist and a selected Skill's instructions (`SkillContext`).
- The kernel registers the renderer of the kind it writes itself
  (`tool_catalog_omissions`) when its registries are built; the
  `workspace-context`, `permission-mode`, `skills` (`skill_catalog`) and
  `memory` (`agent_memory`) plugins, and the host's `ui-context`
  (`application_ui_context`), register theirs with their contributors. The
  kernel knows no host kind: a host context steering carries is written as a
  section of its own kind and shown by that kind's renderer, so with the
  `ui-context` plugin off it stays in history unshown.
- The kernel takes a renderer snapshot for every model request. A section
  whose kind has no renderer (its plugin stopped) is left out of that
  request; a world-state section stays in history, and the next admission
  removes it if no contributor produces it.
- Order: every run of adjacent sections in a request is ordered by slot
  (`Instructions`, `WorldState`, `Capabilities`, `Memory`), then by renderer
  registration order (the kernel's, then the manifest's mount order, the
  host's rows after the Agent's capability plugins and before its context
  plugins; a remounted plugin's renderers move to the end of their slot);
  every other item keeps its
  place. The prompt fragments join the run the conversation opens with, so
  the stable slots come first and the memory summary, which may change on
  any step, comes last. A world-state section that changes later is shown
  where it changed (its earlier version is no longer shown), which keeps the
  prefix before it cached. No "replaces previously provided" notice exists:
  only the latest version of a kind is ever shown.
- There is no section-changed event: the persisted `WorldStateUpdate` is
  the change record.

### Turn admission

- A `TurnAdmissionContributor` produces per-turn data once, when a turn is
  admitted, as `TurnExtensions`. The kernel freezes them on the turn and
  hands them to every tool call (`ToolInvocation::turn`) and prompt
  contributor of the turn; a turn restored after a restart has none.
- The application collects them before the kernel admits the turn
  (`hanafubuki_agent_kernel::collect_turn_extensions`, beside `collect_world_state`):
  contributors run one after another in `(rank, id)` order under an entry
  lease, with a `TurnAdmissionRequest` (session id, the turn's mode). An
  error, a panic, a contributor whose plugin is stopping, or a value of a
  type another contributor already returned leaves that contributor out of
  the turn, recorded as its plugin's issue as for a context contributor;
  the result never depends on registration order. The one admission that
  still fails is a Skill the user referenced explicitly that the turn cannot
  resolve: without the `skills` plugin's values (its catalog failed) it is
  refused ("the Skill catalog is unavailable") rather than run without it.
- Long work runs on the `BlockingService`. A contributor plugin that failed
  to start leaves its values absent, so its tools see a missing value.
- The kernel reads one value itself: `SelectableSkills`, the Skills an
  explicit reference or a `$mention` may select, with the instructions each
  commits as `ConversationItem::SkillContext`, and its
  `mention_exclusions`, the names a plain `$mention` names instead of a
  Skill. The `skills` plugin's contributor provides it with the catalog and
  snapshots the exclusions in the same admission: every tool in the
  `ToolRegistry` and, for each MCP tool lease that is not `Hidden`, its
  server id, local name and flattened alias. The kernel resolves the user's
  references against it, adding its own `AgentCoreTool` names to the
  exclusions (ASCII case-insensitive); a `[$name](skill://<id>)` link or an
  explicit reference still selects. Without the plugin no Skill is
  selectable, so nothing needs excluding.

### External producers: MCP servers, Skills, web, memory

What an MCP server, a Skill author or an extraction model produces is read
tolerantly; one bad item costs only itself, and the strict rules (MCP
configuration, credentials and OAuth, path containment, URL validation,
persisted formats) are unchanged.

- MCP catalog (`hanafubuki-agent-mcp` `catalog.rs`): built tool by tool. Each name
  segment, the server id and the tool name, is sanitized per character to
  `[A-Za-z0-9_-]` (`my.tool` is `my_tool`), and one still over 128 bytes is
  cut and ends with `_` and 12 hex digits of a hash; fitting the joined name
  to the provider is `ProviderToolTable`'s, and calls go out by the raw
  `remote_name`. A tool
  with an invalid or oversized input schema, a name that cannot be encoded,
  or a later duplicate name (two names that sanitize alike among them; the
  first in listing order wins) is skipped; a
  description over 16 KiB is cut on a character boundary and an invalid
  output schema dropped. Each is a `McpCatalogIssue`, recorded as a Warning
  diagnostic of the server with the tool's name (at most 16 per listing)
  beside a `tool_discovery_partial` summary; the server stays connected,
  with zero tools if none is usable. The tool-count bound stays strict and
  is checked once, while `tools/list` is paged, and counts undecodable
  entries. `nextCursor: ""` ends paging; a page over 4 MiB, over 2048
  entries or with an oversized cursor is still an error.
- MCP annotations are hints from a server the user configured, never a
  grant. A tool whose `readOnlyHint` is true and whose `destructiveHint` is
  not (`McpToolHints::read_only`) is a `ConcurrentRead` with `ReadOnly`
  effect, so it runs beside other reads and is offered in Plan mode; every
  other tool is `Exclusive` and `Mutating`. Every MCP tool's memory effect
  stays `PollutesContribution`: its output is external content whatever the
  hint says, as a web fetch's is. Approval does not read the read-only hint: the
  server's (or the tool's) approval policy decides, and `Allow` skips the
  review only for a tool whose `destructiveHint` is false, a missing hint
  counting as destructive (`McpToolHints::destructive`).
- MCP liveness (`manager/reconnect.rs`, `liveness.rs`): a connection's
  transport says when rmcp's loop ended; when the server's side ended it (a
  stdio process exited, an HTTP transport closed), the server is `Failed`
  with the reason (`the MCP server process exited with code 3`), its entry
  is retired, so its tools leave the next step's plan and the process is
  disposed with its receipt once no step holds it, and the manager
  reconnects it with the configuration and secrets the closed connection
  was opened with, after 2 s, 10 s and 30 s within 10 minutes (the module
  host's `RestartPolicy`, `McpManager::with_reconnect_policy`). The status
  carries `reconnect` (the next attempt, or none once used up) and the
  badge reads "Reconnecting". A reconnect does not start once a connect,
  refresh or disconnect from outside got there first; a connect from
  outside (the user's Connect or Refresh, a settings, credential or proxy
  change) empties the window, and shutdown cancels a scheduled one. A step
  keeps the connection it was planned with (a refresh retires the old one
  only once no step holds it); a call on a lease whose connection closed
  returns "the MCP server's tools changed; search for the tool again or
  retry the call" (`McpError::CatalogChanged`) without asking for
  approval. Stored secrets go stale with OAuth: a reconnect with an expired
  token fails and the user's Connect resolves fresh ones. A tool call that
  panicked in the client (see [Panics in plugin code](#panics-in-plugin-code))
  retires its connection the same way, with the diagnostic code
  `tool_call_panicked`.
- MCP settings: what a new server starts with (enabled, timeouts,
  concurrency, approval, exposure, the HTTP OAuth policy) is
  `McpServerDefaults::default()`, sent with every `McpApplicationSnapshot`
  as `serverDefaults`; the settings dialog fills its form from it.
- MCP lists and reads (`listing.rs`, `request::read_resource`): pages and
  reads go through `Peer::send_request`, so a result rmcp's typed structs
  reject arrives as `ServerResult::CustomResult` and is decoded entry by
  entry, the way tools/call results are. A tool that does not decode is a
  `tool_skipped_undecodable` catalog issue; a resource or template that does
  not decode is left out. Discovery records a
  `resource_skipped_undecodable` / `resource_template_skipped_undecodable`
  Warning naming each (at most 16 per listing) and counts them in
  `resource_discovery_partial`; an Agent's page adds one Warning with the
  count, so paging again cannot crowd out other diagnostics. The page's
  other fields keep rmcp's rules and an error response still fails the
  page. rmcp decodes a
  result with `_meta` and an undecodable entry as a `CallToolResult`, so that
  page still fails whole.
- MCP output: a content block the client cannot decode becomes the text
  `[unsupported MCP content: <type>]`, and a `resources/read` entry that does
  not decode `[unsupported MCP content: resource]` (its `type` when it has
  one); over 64 items keeps 63 and a note of how many were omitted; a mime
  type with parameters is its base type and unpadded base64 is accepted
  (both stored canonical, `canonical_media_type`, `canonical_base64`).
- MCP status: a server's `McpDiagnosticSummary` counts its current
  diagnostics (those since its latest connection attempt began) as errors
  (`Error`, `OutcomeUnknown`) and warnings, and heads with the most severe,
  most recent one; the settings row and the `/mcp` card show that line and
  open it to every current entry, and the verbose card keeps the whole ring.
- Skills (`hanafubuki-agent-skills`): `SKILL.md` frontmatter and `openai.yaml` are read
  field by field: a wrong type on an optional key drops that key with an
  issue, a name over 96 characters or a description over 4096 bytes is cut
  on a character boundary with an issue, and an unreadable directory below a
  root is skipped with an issue (an unreadable root still fails that root).
  `name` and `description` stay strict on type. The issues reach
  `SkillCatalogSnapshot::issues`, each with a `SkillIssueSeverity`: `Error`
  for a directory that was not loaded (a Skill that failed, an unreadable
  directory or root), `Warning` for what loaded anyway (a field dropped or
  cut, a Skill shadowed by a nearer one, the watcher's diagnostic); the
  settings panel counts the two apart. `allowed-tools` is an unknown key:
  Skills never add or restrict tools. A turn catalog that cannot be built leaves
  the `skills` contributor out of the turn (its plugin shows the warning).
  A Skill whose declared MCP server is not Ready is not usable in the turn
  and is listed as unavailable with that server, its state and why
  (`SkillTurnCatalog::unavailable`); connecting it stays the user's action
  in Settings or `/mcp`.
- Web: a search endpoint configuration the service cannot use (SearXNG
  selected without an address; the settings validate the address itself)
  leaves the `web` row running with `web_search` omitted from every step (logged once
  per start; a call that still arrives reads "web search is not configured:
  <reason>"); `fetch_web_page` keeps working. Other configuration faults
  still fail the row.
- Memory: a notes line that cannot be read is skipped (logged by line
  number, never its content) and the others are read. Phase-one extraction
  output ignores fields its schema does not name and keeps what the named
  ones still give: `hasMemory: false` is the no-op gate and wins over stray
  fields; otherwise a missing text is empty, an over-long one is cut on a
  character boundary, NULs are dropped and a bad `rolloutSlug` is rewritten
  to lowercase kebab-case (`memory` when nothing is left). Only an
  extraction with neither text is no memory, and only output that is not an
  extraction object at all fails the job. A contribution mode the store does
  not know reads as `disabled`.
- Memory consolidation: the agent may write only what publish accepts
  (`MEMORY.md` up to `MAX_MEMORY_RESOURCE_BYTES`, `memory_summary.md` up to
  `MAX_MEMORY_SUMMARY_BYTES` and starting with `v1`); a write over the bound
  is a tool error naming it, so the model can shorten and retry.

### Prompt fragments

- A `PromptContributor` renders model-visible fragments from the turn's
  `TurnExtensions`, what the contributors prepared for the request's
  sampling step and the session's context window (`PromptRequest`). A
  `PromptFragment` has a kind (a `ContextSectionKind` whose renderer the
  same plugin registers), an optional version (shown only by a versioned
  renderer) and content. The `skills` plugin's contributor lists the turn's
  catalog (`skill_catalog`, slot `Capabilities`: each Skill's id and
  directory, in `ResolvedPath::native_text`, so `exec_command` can run its
  scripts, then the unavailable Skills); the `memory` plugin's
  renders the published memory summary (`agent_memory`, slot `Memory`).
- `prepare` is the one async, I/O-allowed call: once per sampling step,
  when the step starts, the kernel asks every contributor of the step's
  snapshot, one after another in `(rank, id)` order, with a `PromptStep`
  (session, turn, generation, sampling step) under a lease on its entry and
  with a child of the turn's cancellation. It returns typed values
  (`TurnExtensions`), merged into the step's prepared values. A contributor
  whose plugin is stopping prepares nothing; an error, or a value of a type
  an earlier contributor prepared, leaves that contributor's values out of
  the step with a warning. Only the turn's cancellation fails the step. The
  default `prepare` prepares nothing; long work runs on the
  `BlockingService`.
- `contribute` is pure and bounded (no I/O, no blocking, no per-call state,
  never a call into the kernel), so the kernel calls it without a lease. It
  returns nothing for a request that lacks what it renders. Values it needs
  from outside the turn are loaded in `prepare`, never in `contribute`.
- The kernel never runs a contributor under the session's lock: it
  reads the turn's extensions and context window under the lock, releases
  it, prepares the step and renders the fragments, and builds the request
  under the lock again (which still checks the turn or revision it read).
  It does so once when a sampling step is prepared, with a registry
  snapshot taken then, and every request counted for that step (the
  admission estimate, the step's request, an automatic compaction's check
  of the next request) uses that one rendering, memory context included. Only a standalone compaction,
  which belongs to no step, renders without preparing
  (`PromptRequest::prepared` is `None`, so no memory context). It places
  the fragments with the sections the conversation opens with, by slot
  ([Context renderers](#context-renderers)), in every request: a window
  compacted mid-turn renders them all again beside its own initial context
  (mode and task checklist), never relying on the summary to carry them.
- A budget a plugin measures in tokens uses
  `hanafubuki_agent_contracts::{estimate_tokens, token_budget_bytes}`, the
  estimate the kernel admits requests by.
- The kernel reads one prepared value itself: `AgentMemoryStep`, the step's
  memory policy (`AgentMemoryStepPolicy`), which the `memory` plugin's
  `prepare` always returns. The kernel asks it which prefix of a streaming
  answer to show, for a final answer's visible text and validated memory
  citations (committed with the answer and carried by `SamplingCommitted`),
  and, when a tool's output pollutes the session's contribution, whether
  external context is excluded. A step without one (the `memory` plugin is
  off) shows answers as they are, commits no citations and marks no
  pollution: turning the plugin off stops everything memory does to a turn.
- Instructions are not prompt fragments: the session's instructions and the
  kernel's Plan-mode prompt are session configuration.

### Approval rules

- A plugin that owns an action tightens its approval with an `ApprovalRule`,
  registered next to the tools that produce the action. A rule that no single
  plugin owns would stay in the kernel.
- `evaluate` inspects the `ApprovalAction` only. It is pure and bounded (no
  I/O, no blocking, no per-call state), so the kernel calls it without a
  lease. Every call of a step is judged by the step's frozen
  `ApprovalRuleSnapshot`, so a rule cannot disappear under a call that is
  still draining.
- A rule returns `None` (no opinion), `RequireUser` or `Deny { reason }`.
  `ApprovalRuleSnapshot::verdict` combines monotonically: any denial rejects
  the call with the reason as its model-visible error (several give their
  distinct reasons sorted and joined); otherwise any `RequireUser` makes the
  action `Manual` even under `AutoReview` or `FullAccess`; otherwise the
  turn's permission mode decides.
- A rule never relaxes approval. An action a tool declares `NotRequired` or
  `Forbidden` never reaches the rules; a hard refusal belongs in the tool's
  `permission`.
- The user may answer a `Manual`-mode prompt with `AllowForSession`: later
  actions of the same tool (`(provider id, tool id, generation)`) in the
  session are authorized without asking (`ApprovalDecisionSource::SessionAllowance`)
  whatever the turn's mode, but only where the rules have no opinion. A
  rule's `Deny` still rejects and its `RequireUser` still asks, and that
  prompt does not offer `AllowForSession` (`offers_allow_for_session`;
  answering it so is refused). The allowance lives in the session's runtime
  state only: a new Agent instance, a fork and a changed tool (for MCP, a
  reconnect too) start without it.

### Continuation hooks

- A hook asks to keep a turn going after a candidate final answer. The kernel
  snapshots the registry when it starts a sampling step and, if the step ends
  with a final answer, asks the hooks in order. Each call holds an entry
  lease and observes a child of the turn's cancellation, bounded by the
  turn's tool idle timeout. A hook whose plugin is stopping is skipped; one
  cancelled because its plugin stopped completes as `IgnoredFailure`.
- `Pass` asks the next hook, `Stop` ends the turn without the remaining
  hooks, and `Continue` returns 1 to `MAXIMUM_CONTINUATION_FRAGMENTS_PER_HOOK`
  non-blank fragments of at most `MAXIMUM_CONTINUATION_FRAGMENT_BYTES`, which
  the kernel commits as `ContinuationContext` (with the hook's identity and
  generation) before it samples again. A failure or timeout fails the turn
  under `FailTurn` and is skipped under `Ignore`.

### Observers

- An `AgentObserver` learns what the kernel committed through
  `AgentNotification`: `TurnAdmitted` (with the turn's `AgentTurnOrigin`
  and the provisional title its admission committed, below),
  `SamplingCommitted` (with the answer's memory
  citations, validated by the step's memory policy), `TurnFinished` (after the terminal commit of a completed,
  cancelled or failed turn; turns interrupted by a restart are settled during
  recovery, before plugins run), `MemoryContributionChanged` (with its
  `AgentMemoryContributionCause`: `Initial` when a session is created or
  forked, `User` for the user's toggle, `Polluted` after a polluting tool)
  and `SessionArchived` / `SessionRestored`, each with the session revision
  that holds it. A notification is added with its first
  consumer.
- The kernel notifies after the commit is durable and its UI events are
  emitted, never under a session's lock, and in commit order. A
  failed commit notifies nothing.
- `observe` is synchronous and runs on the committing thread, often a turn's
  async task, or the history store's writer for the terminal commit of a
  dropped turn: it signals a worker or does a small bounded local update,
  and never waits for the kernel. It returns nothing, so it cannot change an
  outcome; what must be durable or reach the model is not an observation.
- A call holds no lease: an observer whose plugin is stopping is skipped from
  the moment its retirement starts.
- A turn's origin tells an observer whether to act on it. `AgentTurnInput`
  carries an `AgentTurnOrigin`: `User` for a message the user submitted from
  the interface (the facade's submit command), `Programmatic` for anything
  else (`start_turn`, which the goldens use, and the fixed instruction of a
  plan implementation). It carries a `provisional_title` too: the facade
  computes it for a session without a title (immediate feedback), and the
  admission commits it in the same commit as the turn when the session
  still has none, its `session_title_updated` event right after the
  admission's two. Neither is persisted with the turn. `TurnAdmitted` is
  notified after the admission is durable and its events are emitted (for a
  submitted turn, once the session's sink is retained and before the turn's
  task starts); a refused or failed admission notifies nothing.
- Background work an observer starts runs in a worker the plugin owns,
  spawned with `spawn_scoped_worker` from `apply` and fed through a bounded
  channel the observer only `try_send`s to; the worker is cancelled and
  awaited when the plugin stops (rule 5). It reaches the kernel through a
  narrow port the application constructs (`AgentTitleApplication` for the
  `titles` plugin), never through the facade, and runs blocking calls on the
  `BlockingService`. Kernel mutations it makes are compare-and-swap, so what
  the user did in the meantime wins, and their events go to the session's
  retained sink (`replace_automatic_session_title_if_expected`).

### Logging

kardia logs reach the `log` facade through `hanafubuki_module_contracts::LogExporter`, which the host installs on its root.
kardia has no `trace` level, so content that
[AGENTS.md](../../AGENTS.md#agent-content-logging) confines to `trace` is
never logged through a kardia logger.

## Writing a plugin

Choose where it lives:

- A **plugin crate** `crates/plugin-<name>` (`hanafubuki-agent-plugin-<name>`) when it binds
  one domain crate and needs only contract types (`web`, `workspace-fs`,
  `execution`, `mcp`; a tool source too).
- An **application plugin** under
  `crates/agent/src/plugins/` when it needs
  the Agent's own services (image normalization, `AgentSkillApplication`,
  `AgentMemoryApplication`, `AgentTitleApplication`). It is exported from
  `plugins/mod.rs`; there is no Cargo change.
- A **host plugin** in the host's own crate when it needs the host's
  domains (Hanafubuki's live in `hanafubuki-agent-integration/src/plugins/`
  and use narrow read ports such as `AgentTaskReadPort`). It is written
  against contracts and the facade's public API only, and is one of the
  host's rows in `AgentHostPorts::plugins`; the rules below apply to it
  unchanged.

Checklist:

1. Crate: `Cargo.toml` with `hanafubuki-agent-contracts`, `kardia-core`, and the
   `hanafubuki-agent-approval`, `hanafubuki-agent-tool-contracts` and domain crates the tools use;
   `src/lib.rs` with module declarations and `pub use plugin::<name>_plugin;`
   only ([AGENTS.md](../../AGENTS.md#rust-source-and-test-layout)).
2. `<name>_plugin() -> PluginDef`: `PluginDef::new` (`PluginDef::typed` for a
   settings-derived configuration), `.name("<name>")`, and `.inject(..)` with
   the `NAME` of every registry or host service it uses (`PluginFaults::NAME`
   for `spawn_scoped_worker`). `apply` gets each with `ctx.get::<T>()?`,
   registers with `register(Arc::new(..))?`, spawns long work with
   `spawn_scoped_worker`, and returns; it keeps no handle to undo a
   registration.
3. Each tool: a `ToolDescriptor` with a `ToolName::parse` name, a JSON schema
   with `additionalProperties: false`, the exposure, execution class, effect,
   activity action and memory effect; `permission` returns `NotRequired` for
   a read or the `ApprovalAction` of a mutation; `invoke` validates the
   arguments, observes cancellation, re-checks a mutation's approval
   (`verify_tool_approval`), and runs blocking work on the `BlockingService`.
   A tool source registers an `AgentToolSource` in the `ToolSourceRegistry`
   and returns such handlers per step as `SourcedTool`s.
4. No `.unwrap()`/`.expect(` in production sources; map errors into
   `kardia_core::Error` (`Error::new`, `Error::msg`).
5. Wire it: an entry in the root `Cargo.toml`'s `[workspace.dependencies]`
   (members are `crates/*`), a dependency in
   `crates/agent/Cargo.toml`, a row in the layer table
   (`xtask/src/dependencies.rs`), and one row in
   `crates/agent/src/host/manifest.rs` (a service it adapts
   goes through an `AgentPluginInputs` field). `Cargo.lock` follows.
6. Test it as in [Testing a plugin](#testing-a-plugin), and regenerate the
   goldens if the model sees the change.
7. Document it: the crate row in [architecture.md §2](../architecture.md#2-crates-and-layering)
   and the plugin table and tool count in
   [spec.md §4](../spec.md#4-plugins-and-tools).
8. Validate from the repository root with `cargo xtask check`,
   `cargo fmt --all -- --check`,
   `cargo test -p hanafubuki-agent-plugin-<name>` and
   `cargo test -p hanafubuki-agent --lib`.

Worked example (verified 2026-11-05 in Hanafubuki's tree, before the Agent
had its own repository, then removed): a crate
`hanafubuki-agent-plugin-example` with one read-only `Direct` tool `echo_text`
(`Cargo.toml`, `src/lib.rs`, `src/plugin.rs`, `src/tool.rs`,
`tests/plugin.rs`). Outside the crate it touched the workspace manifest, the
facade's `Cargo.toml` and `crates/agent/src/host/manifest.rs` by hand,
`Cargo.lock`, and the regenerated goldens that list the tool, plus the
documentation in step 7. No kernel or contracts file changed.

## Testing a plugin

- Mount it on a fresh `kardia_core::Context` under a plugin that provides the
  registries it injects, and `PluginFaults` when it uses
  `spawn_scoped_worker` (or `hanafubuki_agent::test_support::mount`
  for an application or host plugin, which provides both), wait for
  `settled()`, and assert `state()`:
  `settled()` also resolves while a plugin is still `Pending` on a missing
  service.
- Assert its registrations, update its config, dispose it, and assert that
  every registration is gone. A configuration the plugin rejects leaves it
  `Failed` with nothing registered.
- Test handlers and domain logic without kardia where they do not need it.
- The request goldens (`crates/agent/src/tests/golden.rs`, over the Agent's
  own plugins with a host that adds no rows, context or instructions,
  `test_support::test_host`) pin what the model receives (instructions, tool catalog, input items: in
  Default and Plan mode, with a Skill catalog, workspace instructions, the
  memory summary, after a compaction, with MCP tools from a scripted
  in-process server, and around a tool call and a `tool_search`), the
  presentation events of a tool round trip, and that a recorded history
  corpus still opens. A change the model sees changes the request goldens on
  purpose. Regenerate them with
  `HANAFUBUKI_AGENT_UPDATE_GOLDEN=1 cargo test -p hanafubuki-agent --lib tests::golden`
  and review the fixture diff. The `golden/history/` and
  `golden/history-reasoning/` corpora are re-recorded only with
  `HANAFUBUKI_AGENT_UPDATE_GOLDEN_HISTORY=1`, after an intended
  history-format change; otherwise they keep proving that older history
  opens. A host keeps goldens of its own over its rows (Hanafubuki's are in
  `hanafubuki-agent-integration`), which a model-visible change here
  changes as well. The fixtures are Linux-only.
