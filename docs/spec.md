# The Agent as built

The Agent is a kernel (`hanafubuki-agent-kernel`) extended by plugins on
kardia, composed and embedded through the facade (`hanafubuki-agent`). The
rules of that composition are in [plugins.md](rules/plugins.md), and how its
parts fit together, with the reasons, in [architecture.md](architecture.md);
this document describes what is built.

The Agent runs embedded today, and Hanafubuki is its host. Where a section
names a host's part (its Tauri commands and their guard, its event sinks,
the `agent` module row, its settings page, its quit), it describes
Hanafubuki's, whose own side is in its
[agent-integration.md](https://github.com/licyk/Hanafubuki/blob/main/docs/agent-integration.md);
a host embedding the Agent supplies the same through the host interface
([plugins.md](rules/plugins.md#the-host-interface)).

## 1. Composition and host

- Each Agent instance runs on its own multi-thread Tokio runtime,
  a `ContainedRuntime` (`hanafubuki-agent-runtime`, the Agent's copy of the
  feature runtime's mechanism, Hanafubuki's [backend-spec.md §3.2](https://github.com/licyk/Hanafubuki/blob/main/docs/backend-spec.md#32-setup)): two to four workers and
  up to 128 blocking threads, all named `agent-rt` and marked contained, so a
  panic in Agent code unwinds to its task instead of aborting the process
  ([plugins.md](rules/plugins.md#where-agent-code-runs)). The
  start builds it and the slot keeps it (`AgentHandle::runtime` for the
  running Agent); shutdown takes it (`AgentSlot::take_runtime`) and stops it
  last with `ContainedRuntime::shutdown`, the hard stop that drops whatever the
  instance still runs (a plugin `apply` that never returned, a disposal
  waiting on it) without touching the application. A restart and a turn off
  stop it the same way (`stop_instance`) before the next instance is built. Every
  Agent command runs through the guard in `src/agent/commands/guard.rs`:
  `agent_command` spawns the command's future on the running Agent's runtime
  and awaits it (`agent_compact_session` among them), `agent_blocking_command` runs its
  closure on the runtime's blocking pool, `agent_sync_command` (the two
  synchronous Agent commands) runs the body under `catch_panic`. Each first
  resolves the Agent slot and returns "the Agent is unavailable: <reason>"
  without running the work when no Agent runs. `agent_state_command`
  (`agent_status`, `agent_ack_events`) guards a synchronous command that
  needs no running Agent; `agent_slot_command` (`agent_restart`,
  `agent_restart_safe_mode`, `agent_rebuild_history_index`) runs a command
  on the slot itself, under
  `catch_panic_async`. A panic becomes the command's error ("Agent internal
  error in <command>") and is reported as a kernel fault of that Agent; a
  call whose Agent stopped under it (its task dropped with the old runtime,
  or an error once a newer instance runs) returns "the Agent restarted;
  <command> did not finish" (`AgentSlot::interrupted`). The Agent's
  `BlockingOperationPort` is `ContainedRuntime::blocking_port`.
- The application runs without the Agent. Composition holds a
  `DesktopAgent` beside `DesktopApplication`, whose `AgentSlot`
  (`hanafubuki-agent`, `slot`) is not the service: `Disabled`
  (`setting` or `crashLoop`), `Starting`, `Running` (the service, with its
  instance's generation number), `Faulted { reason, at, restartsInWindow,
  nextRestartAt }` (a kernel fault stopped it), `Stopping` (a restart or a
  turn off), `Failed { stage, reason }` (`construct`, `recover`, `start`) or
  `Stopped` (shutdown). `agent()` returns an `AgentHandle` (the
  running service) or `AgentUnavailable`, whose message is "the Agent is
  unavailable: <reason>". Every holder resolves the slot per call: the
  command guard (which returns that message for every Agent command), Task
  Center (`AgentSlot::list_run_projections`, which the integration's
  `AgentTaskActivities` maps to `AgentActivitySource`: no Agent, no Agent
  rows),
  diagnostics (`flush_for_diagnostics`, `plugin_status_for_diagnostics`:
  nothing to read; the bundle's `app/agent-status.json` holds the slot's
  status), the settings command (no reconfiguration without an Agent) and the
  Agent's Skill watcher. The slot tells an `AgentStatusEventSink` (root:
  `TauriAgentStatusEventSink`, the payload-free `agent://status-changed`)
  on every change; `agent_status` (a synchronous command under
  `agent_state_command`) returns `AgentStatus`: the state with its reason or
  stage, the generation (starts claimed so far), `safeMode`, `degraded` (a
  plugin of the running Agent `failed` or its teardown stuck, derived from
  the plugin status) and the quarantined sessions: those recovery skipped at
  start, and those the instance's history refused since (a session list row
  that does not decode, reported through `AgentInstanceInputs`'
  `history_refusals`, `AgentSlot::refusal_sink`; a stale instance's report
  is ignored), each added once and announced. Vue reads it again on
  `agent://status-changed` and on `agent://plugins-changed` (`degraded`
  follows the plugins). The title bar's Agent button names the state and
  opens the Agent's surface in every state; while the Agent does not run,
  the surface shows why and calls the slot commands to repair it.
- Supervision (`slot::supervisor`). A kernel fault — a panic caught
  by a turn's task (`AgentKernelFaultSink`, injected into the kernel with
  `AgentService::without_default_model_reporting_faults`), an Agent
  command's guard, a member of the Agent's worker groups
  (`BackgroundWorkerGroup::contained_with_panic_handler`: history writers,
  the memory supervisor, MCP stdio) or the host's and service's own tasks
  (`spawn_kernel_task`: the plugin-fault follower, a remount, the settings
  follow-up, the MCP start) — reaches the slot through the sink of its
  instance (`AgentSlot::fault_sink(generation)`; a stale instance's report
  is ignored, one during a start fails that start). The slot makes the Agent
  `Faulted` at once, stops it on its supervisor thread (`agent-supervisor`,
  contained), and starts the next instance after 2 s, 10 s and 30 s for the
  first three automatic restarts in a 10-minute window
  (`AgentRestartPolicy`, which is `hanafubuki_agent_host::RestartPolicy`,
  the copy of the policy the application host restarts a faulted module with;
  `AUTOMATIC_RESTART_DELAYS`, `AUTOMATIC_RESTART_WINDOW`); after those, a fault leaves it `Faulted`
  without `nextRestartAt` until the user restarts it, which empties the
  window. A plugin's fault stays the plugin's (§3). `AgentSlot::restart`
  (with `AgentStartOptions { safe_mode, rebuild_history_index }`) is the
  user's transition, `enable` and `turn_off` (`disable`, then a bounded
  wait) the `agent` module's mount and stop: each schedules the
  supervisor thread. A restart stops the instance with the
  shutdown sequence and budget (`stop_instance`), sweeps its process owner
  group (`terminate_owner_group`), stops its runtime, resets the crash-loop
  marker (a user's restart) and starts again; safe mode keeps every row
  outside the core off for that instance (`AgentHostOptions::safe_mode`,
  `disabledPlugins` unchanged); the index rebuild opens the history
  with `FileAgentHistoryStore::open_rebuilding_index`. `disable` cancels a
  start under way and leaves `Disabled { setting }`. The Agent's Skill
  watcher restarts on each new instance's discovery roots
  (`SkillCatalogWatcher::restart`, from the start once it runs).
- Construction runs after the host's `Ready` (Hanafubuki's [backend-spec.md §3.2](https://github.com/licyk/Hanafubuki/blob/main/docs/backend-spec.md#32-setup)): `start_agent` takes the
  `AgentLaunch` composition captured (the reusable constructor and the path
  of the Agent's process-group ledger) and calls `DesktopAgent::start`, which
  adds the Agent's own `ExecutorManager` (`hanafubuki-agent-process`, over the
  child-process environment `DesktopAgent` was built with: the application's
  inherited overlay, and the proxy overlay the root's proxy target replaces
  in both environments) and its Skill watcher, hands them to the slot for every
  later instance and starts the first (a disabled slot starts when it is
  turned on), and returns at
  once: the slot's start (`startup`) claims the next generation,
  builds the instance's runtime and process owner group
  (`ProcessOwnerGroup::recorded_in`) and runs there as a task. It takes
  the data directory's lock (once per run; a held lock fails the start at
  construction, naming the directory), records a
  crash-loop attempt, reaps the process groups an earlier run left
  (`ProcessGroupRecord::reap_leftovers`, `data/agent/process-groups.json`;
  lifecycle.md), builds the
  service on the runtime's blocking pool (the `AgentConstructor` receives
  the runtime for its blocking port and the instance's `AgentInstanceInputs`,
  the Agent's Executor already scoped to the owner group, imports the Agent's settings once
  (`import_legacy_agent_settings`, §8), resolves the System and user Skill roots,
  builds the host ports (`hanafubuki_agent_integration::agent_host_ports`
  over `DesktopAgentReadSource` and `AgentInstanceMutationPorts`: mutations,
  provisioning, discovery, diagnostics; the network settings over
  `ConfigState`; the `KeyringSecretStore`) and calls
  `AgentApplicationService::open` with `AgentServiceInputs` (an `AgentConfig`
  on `config/agent/` among them)), runs `recover` there (quarantined sessions go to
  the slot), starts the plugins, schedules the MCP servers' start in the
  background, and installs the service as `Running`; the Agent's Skill
  watcher then starts on the instance's Skill roots, with the slot as its
  port. A failure at
  any stage is logged at error, leaves the slot `Failed`, clears the attempt
  and never reaches the application. Shutdown cancels a start under way
  (`AgentSlot::close` cancels the start's token): construction and recovery
  block and cannot be cancelled, so the start checks the token between
  stages, and the plugins' start is cancelled where it stands. A built Agent
  that will not run (its recovery or start failed, or shutdown cancelled the
  start or closed the slot meanwhile) is retired first: turns, MCP (its
  background connects cancelled, their processes disposed), plugins and
  workers stop in shutdown order, bounded by `RETIRE_BOUND` (5 s), shorter
  than shutdown's wait for a start (8 s), so its runtime is stopped only
  after that teardown. A start still blocked in construction or recovery at
  that point has started no plugin, connection or process (those come only
  from the cancellable stages), so the runtime's hard stop leaves only its
  detached blocking thread. A construction that fails part-way stops the worker groups
  it already started (history writers, the memory supervisor).
- `AgentApplicationService` (`hanafubuki-agent`) is the IPC facade. It
  owns the kernel's `AgentService`, executions, skills, memory, MCP, their
  workers, and the `AgentHost`. `recover` loads the durable sessions once
  before anything uses the service and settles the execution activities the
  previous process left running (a session refusing that is logged, the
  others are settled).
- Degraded capabilities: a Skill catalog that does not open
  (`AgentSkillApplication` holds the error) leaves the Agent without Skills:
  every Skill command returns the reason and the `skills` plugin fails with
  it, which its plugin status shows. A required MCP server that does not
  connect at start-up is its MCP status (`Failed`), logged at error; the
  Agent runs without it, and the start tries every server before it
  reports. The `web` row's derivation error is that plugin's status, as
  before. The workspace roots, the workspace policy, the MCP settings file
  (which repairs itself; only an I/O error fails), the history store's
  layout or index (a session it cannot admit is quarantined instead,
  §8), the memory store and an invalid MCP proxy policy (unreachable:
  the settings refuse a manual proxy without an address) fail the Agent's
  construction.
- Crash-loop detection: `AgentStartMarker` (`data/agent/start-marker.json`,
  `{"unfinishedStarts": n, "lastAttempt": {"pid", "atUnixMillis"}}`, written
  with `write_json_atomic`) counts starts that never finished. A start
  increments it before it constructs; it is cleared once the Agent has been
  `Running` for `AGENT_START_SETTLE` (60 s), when the start failed in an
  orderly way, when the instance stopped in order (a restart, a fault's
  stop, a turn off), and at an orderly shutdown. The last attempt is not
  counted when the crash record (`data/agent/crash-record.json`, written by
  the panic hook before a fatal panic aborts; composition names it with
  `configure_crash_record` once the paths resolve) is of the same process
  and not older: a fatal panic is never on an Agent thread. At
  `CRASH_LOOP_STARTS` (3) the next start leaves the slot
  `Disabled { crashLoop }` and logs a notice; a run that recorded no attempt
  does not clear it, so the stop survives orderly exits until the user
  restarts the Agent or turns its module off and on, either of which resets
  the marker (a mount alone leaves a crash-loop stop alone).
- The Agent is the feature module `agent` on the application host
  ([modules.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/modules.md#the-agent-module)), turned off and on
  through `disabledModules` (`app_set_module_enabled`, at once; the Modules
  page lists it). Its row, mounted last, provides the slot; a mount turns
  the Agent on (`AgentSlot::enable`), its stop turns it off in order and
  waits within `AGENT_STOP_BOUND` (8 s; the slot's stop goes on past it).
  The quit closes the slot before any module stops, so the row's stop in
  the module sweep returns at once. The former `agentEnabled` key in the
  Agent settings document is dropped by the tolerant reader (a one-time
  `unsupported_field` recovery warning), not carried over.
- `AgentHost` owns the kardia root: it is the Agent's profile
  (`AgentPlugins`) of the generic `ModuleHost` (`hanafubuki-module-contracts`,
  [modules.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/modules.md#the-module-host)), whose mechanics follow;
  the Agent adds its two core rows, its manifest, its settings derivation and
  enablement, its groups and the contributions of its status, and forwards a
  panic in a host task (the fault follower, a remount) to its kernel fault
  sink as a kernel fault at `plugin host`. Composition creates the
  `AgentKernelRegistries`, constructs the kernel with them (so the `titles`
  plugin's `AgentTitleApplication` can hold the kernel), then hands them to
  the host, which mounts, in
  order and each after the previous one settled or timed out
  (`PLUGIN_START_TIMEOUT`, 20 s: the row fails with "did not start within
  20 s", its fiber is disposed in the background and the next row mounts):
  `agent-host-services` (the
  `BlockingService`, the application's blocking boundary), `agent-kernel`
  (`hanafubuki_agent_kernel::kernel_plugin`, which publishes the registries) and every
  enabled row of `agent_plugin_manifest`, keeping one fiber per row. kardia logs reach the
  `log` facade through `LogExporter`. Every model operation (a turn, a plan
  implementation, a compaction) starts the host if nothing did; the kernel
  plans from registry snapshots and never waits for the host.
- Manifest rows are `AgentPluginRow::unconfigured` or
  `AgentPluginRow::from_settings` (today only `web`), each with its
  `AgentPluginGroup` (`capability` or `context`; the two fixed rows are
  `core`). Services a plugin adapts reach it through `AgentPluginInputs`. A
  row's configuration derives from `AgentPluginSettings`: the Agent's
  settings and the proxy of its `NetworkSettingsPort`. The global settings
  command `update_global_settings_patch` schedules
  `apply_settings_to_plugins` on the running Agent's runtime after the patch
  is persisted (`AgentSlot::apply_saved_settings`), and the Agent's own
  settings writes (`write_settings`, `set_plugin_enabled`) call it after
  theirs; neither waits: the host reads the latest settings under its plugin
  lock (so the later of two concurrent saves wins), disposes a row that
  `disabledPlugins` now names, mounts one it no longer names (with its
  current configuration, after its previous fiber has stopped, so the old
  registrations are gone), and calls `Fiber::update` only where a settings
  row's configuration changed; it does not wait for the transitions (each
  bounded by `PLUGIN_START_TIMEOUT`), and each outcome is logged. The MCP
  proxy follow-up runs as its own task: it reads the latest settings under
  the MCP configuration gate and, when the policy changed, reconnects the
  enabled Streamable HTTP servers concurrently, so a reconnect that hangs
  holds neither the command nor a later change, which applies after it. A
  row not mounted yet takes the new configuration and enablement for its
  mount.
- Bounds on kardia waits (the module host's `host/bounds.rs`,
  [modules.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/modules.md#the-module-host)): every start (first
  mount, update, retry, remount) is bounded by `PLUGIN_START_TIMEOUT` (20 s);
  a disposal still running after `PLUGIN_STOP_BOUND` (10 s) reports its row
  stuck and keeps running on its own; a re-enabled row waits for its previous
  fiber only that long, then fails ("its previous instance has not stopped
  yet") until a retry after the teardown ended.
- Enablement: `disabledPlugins` (the Agent's `config.toml`; at most 64
  names) lists the plugins the user turned off. A disabled row is not mounted
  at start-up and reports `disabled`; its registrations leave the registries
  with its fiber. Core rows (`agent-host-services`, `agent-kernel`) ignore the
  set. A name no row carries is kept and ignored.
- Status: after start-up the host logs every enabled plugin that is not
  `Active`. `plugin_status` reports each plugin (host services, kernel,
  manifest) as `AgentPluginStatus`: name, group, state
  (`pending`/`active`/`failed`/`disabled`/`disposed`), error text, `enabled`,
  `lock` (`core` for the core rows), `restart` (a pending automatic restart,
  or the ones the window counts) and `contributions`, read from one
  snapshot of each kernel registry by entry owner (tool names, tool source
  ids, context contributors with their kinds, turn-admission and prompt
  contributor ids, approval rule ids, observer ids). A plugin whose code
  panicked is disposed by the host and reports `failed` with
  `panicked in <extension point>: <message>` (§3), and the host mounts
  it again by itself under the application modules' restart policy
  (`ModuleHostOptions { module_faults: false, automatic_restarts:
  Some(RestartPolicy::default()) }`: 2 s, 10 s, 30 s, three in ten
  minutes; a start that failed is not restarted by itself). `retry_plugin`
  restarts a `Failed` plugin with its current configuration (a panicked one
  is mounted again once its teardown has ended; a new configuration or
  turning it off and on does the same), and
  `set_plugin_enabled` refuses an unknown or core name, writes
  `disabledPlugins` through `AgentConfig::set_plugin_disabled` on the
  blocking boundary (serialized by the settings file's lock) and applies the
  settings. App diagnostic bundles
  carry the status (`app/agent-plugins.json`).
- IPC: `agent_list_plugins`, `agent_set_plugin_enabled(name, enabled)` (also
  emits `agent_settings_changed`) and `agent_retry_plugin(name)` return the status
  list. The host tells an `AgentPluginEventSink` (root:
  `TauriAgentPluginEventSink`, the `agent://plugins-changed` hint) when
  start-up ends, when a row is enabled, disabled, retried or reconfigured,
  when such a transition settles, when a disabled row's teardown ends
  (its registrations are gone only then), and when a plugin's issues (its
  `warning`) appear, change or clear; Vue re-reads the list on it. The plugin
  manager is the last panel of Agent settings (`AgentPluginsPanel`).

## 2. Storage

Under `data/agent/`:

- `history/`: per-session JSONL (`active/YYYY/MM/DD/session-<ts>-<id>.jsonl`,
  `archived/`, `trash/`, `attachments/`) and `index.sqlite3`.
- `memory/`: `memory.sqlite3`, `ad-hoc-notes.jsonl`, `artifacts/`.
- `skills/`: `system/` (bundled System Skills), `packages/` (user skills),
  `state.json`; repository skills are read from `.agents/skills` in each
  workspace root and its ancestors up to the Git boundary, resolved again
  from the workspace roots on every catalog read (each turn and each
  settings read), so an instance registered later brings its Skills to the
  next turn.
- `start-marker.json`: the crash-loop start-up marker (§1).
- `.lock`: the data directory's advisory lock, held by the start marker
  from the first start until shutdown, so one process uses the directory
  ([architecture.md §10](architecture.md#10-persistence-and-compatibility)).
- `logs/hanafubuki-agent.log` and its rotated `.1`–`.3`: the standalone
  Agent's log file, when the binary installs it; a host logs to its own.
- `mcp-settings.json`, from before the Agent had its own configuration: read
  once by the settings import (§8), never written.

## 3. Contracts and registries (`hanafubuki-agent-contracts`)

Plugins and the kernel meet in the contracts crate. Each registry is a kardia
service holding entries owned by the plugin that registered them
(`Register`); kardia removes them when that plugin stops, after leased calls
drain (`DRAIN_GRACE`, then `RETIRE_CANCEL_BOUND`).

| Registry | Entries | Read by the kernel |
| --- | --- | --- |
| `ToolRegistry` | `AgentToolHandler` (unique names) | per sampling step, with the tool plan |
| `ToolSourceRegistry` | `AgentToolSource` (unique ids), contributing `SourcedTool`s and optionally a `ToolPlanLease` | per sampling step |
| `ContextRegistry` | `ContextContributor` (unique ids, one contributor per `ContextSectionKind`) | at turn admission |
| `ContextRendererRegistry` | `ContextRenderer` (one per `ContextSectionKind`: slot, role, preamble, epilogue, versioned) | snapshot per model request |
| `TurnAdmissionRegistry` | `TurnAdmissionContributor` (unique ids, ordered by rank and id) | at turn admission |
| `PromptContributorRegistry` | `PromptContributor` (unique ids, ordered by rank and id) | snapshot per sampling step, prepared once per step |
| `ApprovalRuleRegistry` | `ApprovalRule` (unique ids) | frozen with each step's tool plan (`ApprovalRuleSnapshot`) |
| `ContinuationHookRegistry` | `AgentContinuationHookRegistration` (unique provider and hook id, at most 64) | per sampling step |
| `ObserverRegistry` | `AgentObserver` (unique ids) | after each durable commit |

The crate also holds the tool contract (`ToolDescriptor`, `ToolEffect`,
`ToolInvocation`, `ToolResult`, `ToolContextFragment`, `TurnExtensions`,
argument helpers, `verify_tool_approval`), the tool-source contract
(`ToolSourceStep`, `SourcedTool`, `ToolSourceTrust`, `AgentToolOmission`),
the turn-admission contract (`TurnAdmissionRequest`) and the `SelectableSkills`
view the kernel reads from it, the prompt contract (`PromptStep`,
`PromptRequest`, `PromptFragment`), the context renderer contract
(`ContextRenderer`, `ContextSlot`, `ContextRole`, `WORLD_STATE_PREAMBLE`), the
one token estimate (`tokens`: `estimate_tokens`, `token_budget_bytes`), the
Agent identity types, `AgentSessionMode`, activity actions, tool progress,
the memory contract types the kernel commits and the observers carry, the
step's memory policy the kernel reads from the prepared prompt values
(`AgentMemoryStep`, `AgentMemoryStepPolicy`, `AgentProcessedMemoryAnswer`),
the host service `BlockingService` and the extension points
(`ExtensionPoint`, which converts into a fault's `FaultPoint`). The registry
mechanism (`Registry`, `RegistryEntry`, `EntryLease`, `Register`,
`RegistryService`, `RegistrationError`), the fault channel (`PluginFaults`,
`PluginFault`, `PluginPanicked`), `spawn_scoped_worker` and `PluginError`
are `hanafubuki-agent-host`'s (the Agent's copy of
`hanafubuki-module-contracts`), re-exported here.

Every registry is built with the kernel's one `PluginFaults`
(`AgentKernelRegistries::faults`, also published by the kernel plugin), and
each entry holds it. The kernel calls plugin code through the entry's
`guard`/`guard_async`; the registries that combine entries themselves do the
same per entry (`ObserverRegistry::notify`, `ApprovalRuleSnapshot::verdict`,
`PromptContributorSnapshot::fragments`), and `spawn_scoped_worker` guards
its task. A panic becomes that call's error (a tool error, an omitted tool or
source, a failed admission, a skipped observer or fragment, a failed hook, a
denial for an approval rule), is logged and is reported as a fault of the
entry's owner; the host's handler queues it, a host task disposes that
plugin and schedules its automatic restart
([plugins.md](rules/plugins.md#panics-in-plugin-code)).

## 4. Plugins and tools

| Plugin | Where | Contributes |
| --- | --- | --- |
| `agent-host-services` | `hanafubuki-agent` (`host`) | `BlockingService` |
| `agent-kernel` | `hanafubuki-agent-kernel` | the nine registries; the renderer of `tool_catalog_omissions` |
| `web` | `hanafubuki-agent-plugin-web` | `web_search`, `fetch_web_page`; configuration derived from the web-search and proxy settings |
| `workspace-fs` | `hanafubuki-agent-plugin-workspace-fs` | `list_directory`, `search_text`, `read_file`, `apply_patch` (freeform where the provider supports it), `move_path`, `delete_path` |
| `view-image` | application | `view_image` (omitted unless the model takes image input and multimodal tool output) |
| `execution` | `hanafubuki-agent-plugin-execution` | `exec_command`, `write_stdin`, `resize_execution`; the activity monitors; the approval rule for `git clone` |
| `mcp` | `hanafubuki-agent-plugin-mcp` | the MCP tool source (one `McpStepBinding` per step as its plan lease); `list_mcp_resources`, `list_mcp_resource_templates`, `read_mcp_resource` |
| `memory` | application | the prompt fragment of the memory summary (`agent_memory`, slot `Memory`, and its renderer), loaded per sampling step, with the step's memory policy (citations, the stream filter, pollution); the memory tool source; the memory observer |
| `skills` | application | turn admission: the turn's `SkillTurnCatalog` (read from the workspace's repository roots as they are now; narrowed to Skills whose MCP dependencies are ready, the others in `unavailable` with the server and why) and its `SelectableSkills`, with the registered tool and visible MCP names a plain `$mention` must not select; the prompt fragment listing the catalog with each Skill's directory and the unavailable Skills (`skill_catalog`, slot `Capabilities`, and its renderer); `skills_list`, `skills_read`, `enable_skill` and `disable_skill` (deferred); the approval rule for `SkillManagement` |
| `titles` | application | the title observer and the `agent-titles` worker: a generated title for a user turn that gave its session a provisional one |
| `tasks` | integration | `list_tasks`, `read_task_log` |
| `instances` | integration | `list_instances`, `fetch_instance_remote_catalog`, `control_instance`, `manage_instance_extension`, `manage_instance_version`, `manage_instance_snapshot`, `manage_instance_model`, `configure_instance` (deferred), `provision_instance` |
| `ui-context` | integration | context and renderer: `application_ui_context`, which also shows the UI context steering carries (the integration's `HostContextPort`) |
| `workspace-context` | application | context and renderers: `workspace_instructions` (the user's `data/agent/AGENTS.md`, then the workspace roots'), `workspace_roots`, `environment` |
| `permission-mode` | application | context and renderer: `permission_mode` |

That is the mount order: the integration's host rows (`tasks`, `instances`,
`ui-context`, from `hanafubuki-agent-integration/src/plugins/`, because they
use the application's domains) mount after the Agent's capability plugins
and before its context plugins. The Agent's own application plugins live in
`crates/agent/src/plugins/` because they use the
Agent's services.

Tools: 3 kernel tools (`AgentCoreTool`: `new_context`, `update_plan`,
`request_user_input`) and 30 registered tools, 3 of them deferred.
`tool_search` searches the deferred tools; it is the plan's own binding, not a
tool any plugin registers. Per step, the tool sources add the tools of the
connected MCP servers (exposure `Direct`, `DirectModelOnly`, `Deferred` or
`Hidden` by server policy; a concurrent `ReadOnly` read for a
`readOnlyHint` tool that is not destructive, `Exclusive` and `Mutating`
otherwise, every one polluting memory contribution; identity
`mcp.<server>`, the catalog's tool id and generation)
and up to 4 memory tools (`list_agent_memories`, `search_agent_memories` and
`read_agent_memory`, read-only, while retrieval is enabled;
`append_agent_memory_note`, bound to the turn it was planned for, while
generation is and the session contributes). The golden Default-mode
request (no MCP server, memory off, image input on) carries 31 tools: the
27 direct registered tools, the 3 kernel tools and `tool_search`; the Plan-mode
one carries the 15 read-only registered tools and `request_user_input`.

Vue labels tool activity by the open `AgentActivityAction` and falls back to
the generic tool label, so a plugin tool needs no frontend change.

## 5. Sessions and turns (`hanafubuki-agent-kernel`)

- A session has a configuration (provider, protocol, model, instructions, mode,
  reasoning effort, context window, compaction settings), a mode (`Default` or
  `Plan`), a lifecycle (`Available`, `Archived`), a title, and a memory
  contribution mode. Sessions can be forked before a turn or at the head.
- A turn runs as a managed Tokio task through phases `Sampling`,
  `RunningTools`, `WaitingForUserInput`, ending `Completed`, `Cancelled`,
  `Failed`, or `Interrupted`. Each loop iteration may auto-compact, samples the
  model with a frozen tool plan, runs tools, consumes steering input, and asks
  the continuation hooks. Budgets and hard limits (`FIRST_BATCH_HARD_LIMITS`)
  bound steps, tool concurrency, sizes, and time. A panic in the turn's own
  work (`execute_admitted_turn` catches it around the loop) is logged at
  error with the session and turn ids, aborts the turn's pending approvals and
  ends the turn `Failed` with `InternalFailure` (`AgentError::Internal`), not
  as the cancellation its drop guard would commit; the session takes its next
  turn.
- Thinking (`service/sampling/stream/reasoning.rs`): each reasoning item the
  model streams gets a kernel id (`AgentReasoningId`, drawn from the message
  counter; provider ids never leave the stream) and is published as
  `ReasoningStarted { reasoning_id, sampling_step }`, `ReasoningDelta {
  reasoning_id, delta }` (Markdown; a new summary part arrives with its blank
  line) and `ReasoningCompleted { reasoning_id, duration_millis }`, the time
  from the item's start to its completion. The kept text is the item's
  summary when it has one, else its raw text, each from the completed item
  when it carries any, else as streamed; text the completed item adds to
  what was shown is shown before the completion. Thinking is not counted
  toward `maximum_model_output_bytes`: each item is cut at
  `maximum_reasoning_text_bytes` (256 KiB) on a character boundary, and the
  rest is neither shown nor kept. The step commits each item as
  `ConversationItem::Reasoning { text, duration_millis, sampling_step,
  truncated }` with its other items, in the order the model produced them;
  it is presentation only: `as_model_input` skips it, the token estimate
  does not count it, compaction drops it and permission reviews and memory
  never read it. A response that stops early completes its open items with
  what streamed; a response of only reasoning still commits nothing. The
  timing snapshot (`ReasoningUpdated`, `AgentReasoningSnapshot`) is kept
  beside it for status and recovery.
- Tool plan: per sampling step the kernel (`AgentToolPlanner`) plans the core
  tools, the registered tools (with each handler's `availability` for the
  step's `ToolPlanContext`) and the tool sources, and freezes the plan with
  its plan leases and the approval-rule snapshot. Each source is asked once,
  under a lease on its entry, with a `ToolSourceStep`; a failing `Optional`
  source is omitted and a `Required` one fails the plan. Its `SourcedTool`s
  are planned as handler tools with the source's identity (provider id, tool
  id, generation) and trust `RequiredExternal` or `OptionalExternal`: an
  optional tool that cannot be planned is omitted, and when the direct
  catalog is over budget only sourced direct tools are deferred, fairly
  across providers. Each tool's wire name for the step's protocol (a native
  namespace on Responses, a flattened name fitted to the protocol's limit
  elsewhere, a hash-suffixed one on a collision) and its schema lowering are
  in [plugins.md](rules/plugins.md#tool-names); calls resolve by
  lookup of that name. Plan mode keeps only `ToolEffect::ReadOnly` handler
  tools, registered or sourced, and `request_user_input`.
- Calls the step cannot run (`service/tool_execution/rejection.rs`,
  `CallDisposition`): an unknown, `Hidden`, mode-hidden or
  provider-incompatible name gets the one error "no tool named `<name>` is
  available in this step" (history records the call under the `unavailable`
  provider and tool id); an unadmitted Deferred tool the discovery error;
  arguments a decoder could not parse (`ModelToolCall::argument_error`) or
  over `maximum_tool_argument_bytes` (64 KiB; `{}` is kept instead) an error
  with a 512-byte preview; a malformed `tool_search` a field-named error.
  Each is the call's output and the turn goes on; none runs a tool.
- A `tool_search` commits its output and then a
  `ConversationItem::ToolSelection` (`call_id`, the selected
  `AgentToolSelection`s). The planner admits the selections of the model
  context since the last compaction (`sampling/request.rs`), so they last
  across turns; compaction keeps no `ToolSelection`, and the model searches
  again. A selection whose tool is gone or whose generation changed matches
  nothing. The item is not model input; history from before it simply has
  none.
- Before a call that runs, the kernel normalizes its arguments by the tool's
  input schema (`tool_catalog/arguments.rs`: integral floats and numeric
  strings to numbers, a null optional removed, an undeclared key removed
  under `additionalProperties: false`); handlers ignore unknown keys, read a
  missing optional as absent, clamp limits with `ArgumentNotes` and name the
  field of an error ([plugins.md](rules/plugins.md#model-tool-arguments)).
- Tool pipeline (`execute_handler_tool_call`), under the step's gate
  (`ToolCallRuntime::enter`: an in-flight permit, then read access for a
  `ConcurrentRead` tool or exclusive access): mode policy, for a sourced
  tool the arguments against its schema (`validate_dynamic_tool_arguments`),
  the handler's `permission`, approval when review is required (§6), then
  `invoke` under an `EntryLease` (the tool's registry entry, or its source's)
  with a child cancellation token, bounded by `ToolDescriptor::timeout`, else
  the turn's `tool_idle_timeout` (10 min). An approval that waits (the user,
  or the reviewer model) releases the gate and takes it again once approved,
  so the step's other calls run meanwhile; the user is asked one prompt at a
  time per step. A call past its timeout has its token cancelled and gets
  "the tool did not finish within N s and was stopped"; the turn continues.
  The MCP tools' timeout is their server's call timeout plus one elicitation
  wait (`MAXIMUM_MCP_ELICITATION_WAIT`), since the server's own timeout
  pauses during an elicitation. A call still running when its plugin stops
  is cancelled after the drain grace and reported to the model as a tool
  error.
- Commit (`tool_execution/scheduler.rs`): a step's calls run together
  (`CompletionSet`), and each output is committed as soon as every earlier
  call's is, so history keeps call order. A call that ends in an error (the
  turn's cancellation among them) commits nothing; the others still run to
  their end and commit, and the step returns the first error. So a cancel
  commits every finished call (their effects happened) and only the
  unfinished ones get the synthetic interrupted output at the terminal
  commit.
- Turn limits (`AgentHardLimits`): a turn samples at most
  `maximum_sampling_steps` (32) times; the next step fails the turn with
  `LimitExceeded { "sampling steps" }` (`budget_exceeded`) after every
  earlier output was committed. At `turn_deadline` (30 min) the turn is
  cancelled as by `request_turn_cancellation` and ends `Failed` with
  `TurnDeadlineExceeded` (`deadline_exceeded`, "stopped at its time limit of
  30 minutes; send a message to continue"). A provider's context-window
  error is answered with one automatic compaction and a retry; a second one
  before a response succeeds ends the turn `Failed` with
  `ContextWindowExceededAfterCompaction` (`context_window_exceeded`, "the
  conversation is still too long for the model's context window after it was
  compacted; start a new conversation or attach less context"). Each call carries the step's plan
  leases (`ToolInvocation::plan_leases`) and the turn's `TurnExtensions`,
  which hold the `skills` plugin's frozen `SkillTurnCatalog`. A `ToolContextFragment` in a
  result is committed right after the output as the
  `ConversationItem::SkillContext` of the Skill its `source` names, once per
  turn and Skill (a source that is not a Skill id is a tool error);
  `skills_read` of a `SKILL.md` returns one, which adopts the Skill, and an
  output without the body, so the instructions reach the model once (a
  Skill a `$mention` selected is not committed again).
- World state: each turn admission (and plan implementation) collects a
  snapshot from the context contributors (`collect_world_state`), which
  leaves out a section past `MAXIMUM_AGENT_PLUGIN_WORLD_STATE_SECTIONS` = 7
  of plugin kinds (a host's among them) besides one per known kind (5)
  (`MAXIMUM_AGENT_WORLD_STATE_SECTIONS` = 12 in all). The kernel validates
  the sizes (32 KiB each, 64 KiB in total; a persisted update may name
  twice the section count, a replace and a remove per kind), versions each section by a SHA-256 of its
  kind name and content, and commits only the changes as a persisted
  `WorldStateUpdate` (sections in kind name order). The model sees the
  latest section of each kind, framed by its kind's `ContextRenderer`; every
  run of adjacent sections is ordered by renderer slot (`Instructions`,
  `WorldState`, `Capabilities`, `Memory`) and then by renderer registration
  order, and a kind without a renderer is left out of the request
  ([plugins.md](rules/plugins.md#context-renderers)). The kernel
  itself writes `tool_catalog_omissions` per sampling step, and steering
  replaces the host context it carries (an `AgentHostContext`: a kind and
  its content; here `application_ui_context`) mid-turn. `workspace_instructions`
  holds the user's `AGENTS.md` (or `AGENTS.override.md`) from the Agent
  data directory and then the workspace roots', under one 32 KiB budget. The instructions (the
  session's configured instructions, by default the composition's base
  instructions with the host's `AgentHostInstructions` joined in, plus the
  kernel's `plan.md` in Plan mode) and the prompt fragments (the
  memory context among them) are assembled by the kernel per request and
  are not world state.
- A context or turn admission contributor that fails (error, panic, its
  plugin stopping, an undeclared, repeated or past-the-bound kind, a type
  another contributor produced) is left out of that turn and recorded as an
  issue of its plugin (`PluginFaults::set_issue`), shown as the plugin's
  `warning` and the Agent's `degraded` until it succeeds again. An explicit
  Skill reference the turn cannot resolve without the Skill catalog is
  refused. The UI context never refuses a message or steering: an unusable
  one is dropped with a warning, an oversized one cut on a UTF-8 boundary to
  `MAXIMUM_HOST_CONTEXT_BYTES` (4 KiB), the kernel's bound for a host
  context, which recovery holds persisted steering to. A user turn's UI
  context is a world-state section from its contributor; steering carries
  its own, rendered by the integration's `HostContextPort` (steering recorded
  before the host context had a kind, as `applicationContext`, reads back
  without one).
- Session configuration is normalized in one place
  (`normalize_session_config`) at creation, every update and recovery:
  reasoning effort and remote compaction on a protocol other than
  Responses (`ModelApiProtocol::supports_reasoning_effort`,
  `supports_remote_compaction`), or on an unconfigured session, are
  dropped with a warning, a zero context window becomes unknown, the
  auto-compact threshold and the stream retry policy are clamped. The
  facade resolves the shared model policy unchanged and leaves this to the
  kernel. Recovery re-normalizes an older session instead of quarantining
  it; identity (provider id, model) and the integrity checks stay strict.
- Turn data: before admission the facade collects the turn's
  `TurnExtensions` from the turn admission contributors
  (`collect_turn_extensions`); the kernel freezes them on the turn (in
  memory; a recovered turn has none) and hands them to tool calls and prompt
  contributors. Admission resolves the user's Skill references and
  `$mention`s against the `SelectableSkills` among them and commits each
  selected Skill as `ConversationItem::SkillContext`. A plain `$mention` of
  a name in `SelectableSkills::mention_exclusions` (the registered tools and
  the visible MCP servers, tools and aliases, which the `skills` plugin
  snapshots with the catalog) or of a kernel core tool selects nothing. Per
  sampling step the kernel snapshots the prompt contributors, has each
  prepare the step (`prepare`, async, under an entry lease, outside the
  session lock: the `memory` plugin reads the published summary and returns
  the step's memory policy), renders their fragments outside the lock from
  the extensions it read under it and the prepared values, and shows them
  with the sections the conversation opens with, by slot, in every request
  it builds: after the instructions and world state, the Skill catalog, then
  the memory summary. A window compacted mid-turn renders every fragment
  again beside its own initial context (mode and task checklist); nothing a
  section shows is summarized. The step's memory policy filters the streaming answer, takes the
  validated citations out of the final answer and decides whether a
  polluting tool output marks the session's contribution; without the
  `memory` plugin a step has none and does neither.
- Continuation hooks are snapshotted per sampling step and asked by
  priority, provider id and hook id when a step ends with a final answer; a
  `Continue` commits its fragments as `ContinuationContext` and samples again.
  None is registered today.
- Steering adds input to a running turn; `request_user_input` pauses for
  answers; Plan mode ends with a proposed plan that `agent_implement_plan`
  turns into a Default-mode turn.
- Compaction (manual, automatic, or after a context-window error) stays in
  the kernel and replaces history with a local summary or a provider
  compaction under the loop's reservation, checkpoints and rollback.
  `prepare_sampling_step` builds a step's tool plan and prepared prompt
  (`SamplingStep`) before the admission check; the check, the step's request
  (`begin_sampling`), and an automatic compaction's check of the request
  after it all build that request with `step_model_request` and count it
  with `estimate_model_request_tokens`, the stream included, so tools and
  the memory context are counted everywhere. A standalone compaction builds
  its own plan and renders without a step (no memory). Token estimates
  throughout the Agent (the kernel's, the Skill catalog budget, MCP resource
  listings, memory evidence) use the one bytes-per-token estimate in
  `hanafubuki_agent_contracts::tokens`.
- The context window: a session's `context_window_tokens` is resolved when
  it is created or its model is selected (`ModelCatalog`): the configured
  window, else the one the provider's model listing reported in this run
  (`ModelCatalogEntry::context_window`, from `max_input_tokens`,
  `context_length`, `max_model_len` or `context_window`), else a small
  built-in table of well-known model families; otherwise none, and no
  automatic compaction. A session created before the window was known keeps
  none until its model is selected again; reselecting a model also moves the
  recorded usage to the new window.
- Cancellation is cooperative first, then aborts the task, and is confirmed
  within 5 s; `cancel_turn_confirmed` awaits the session's terminal state,
  so `agent_cancel_turn` holds no blocking worker. Turns interrupted by a
  restart are recovered as `Interrupted` (`service/recovery/`: validation,
  the persisted record, then the restart repairs in `repair.rs`). Validation
  rejects a session that persisted a counter the kernel later increments
  (revision, generation, message, history item, turn, compaction and activity
  ids, steering sequence, model session generation, memory contribution and
  history item revisions) above `u64::MAX / 2`
  (`validation/counters.rs`), so those `checked_add(1)` expectations are
  invariants. A session recovery rejects is quarantined, not fatal:
  `load_recovery_candidates` loads each candidate on its own, and
  `recover_sessions` returns an `AgentRecoveryReport` (restored count and the
  `AgentQuarantinedSession`s with their reasons) after logging each skipped
  session at error. `load_recovery_candidates` also returns, as failures,
  the sessions the history store refused as it opened (§8), whatever
  their recovery state, so each appears once in the report. Nothing of a
  quarantined session is rewritten or deleted; opening it fails with its
  unresolved recovery obligation, or the store's reason, until a later start
  recovers it. The memory source projection leaves out a session whose
  runtime does not decode. Only a failure to read the history index fails
  recovery.
- Sessions (`service/state/cell.rs`): `SessionTable` maps each live session
  to a `SessionCell`, and its lock is held only to look a cell up, insert or
  remove one. A cell owns its `SessionRecord` behind its own lock, held only
  for in-memory work on it: never across an await, a history append or
  other I/O, and never with another cell's lock, so sessions never wait for
  one another. Turn and compaction ids come from atomic counters. A removed
  cell answers `SessionNotFound` to whoever still holds it.
- Waiting: a cell has two signals, `persisted` (its staged commit settled,
  or the session was removed) and `terminal` (a turn or compaction ended),
  each with an async side (a `watch` epoch) and a blocking side (a `Condvar`
  on the cell's lock). Async code awaits them: the turn loop, its admission
  and compaction (`lock_after_persistence`) and cancellation confirmation
  (`wait_until` under a Tokio timeout). Blocking waits are for callers off
  the async runtime: the synchronous IPC queries and mutations, which run on
  blocking workers (`agent_blocking_command`, or the facade's blocking boundary, which
  also runs the session activation before a model operation and submits
  turns); `list_snapshots`; `flush_for_diagnostics`; and
  `stop_operations`/`shutdown` on the shutdown thread, where every session's
  wait and progress flush and then the wait for the managed turn tasks
  (their own `Condvar`) share one deadline. The drop guards, execution
  progress and the kernel's async `start_turn` wait on no blocking side.
- Persistence: every mutation commits durably to history before it is
  visible; a failed commit restores the in-memory state. A session record
  keeps its identity apart from five groups (settings, status, conversation,
  event journal, runtime; `service/state/record.rs`), and rollback restores
  the groups whole. A commit is staged under the cell's lock
  (`persisting_revision` holds every other writer back) and settles when
  its append is acknowledged: the persisted revision is published, or the
  record is rolled back, and only then are its events emitted
  (`service/durable.rs`). Async code (the turn loop, its admission in the
  kernel's `start_turn`, compaction, execution progress) queues its appends
  (`AgentHistoryStore::enqueue_commit`/`enqueue_delta`) and awaits the
  `AgentHistoryAppendAck`. The settlement runs inside the acknowledgement,
  on the history writer, not in the awaiting future: it happens even if the
  future is dropped meanwhile, and nothing waiting on the session depends on
  the turn's runtime; an acknowledgement the store drops rolls the record
  back. IPC session mutations and the application's turn admission (on its
  blocking boundary, so the admission and the managed task's spawn are
  never separated) append on the calling thread
  (`append_commit`/`append_delta`).
- Drop guards: a turn or compaction dropped before its end (an aborted task,
  or a `start_turn` dropped while its admission is appended) ends through
  its guard, which can neither await nor wait. The guard stages the
  cancellation and hands it to the store with nothing awaiting it
  (`StagedPersistence::into_follow_up`); its acknowledgement publishes it,
  wakes the terminal waiters and emits its events (and `TurnFinished`) on
  the writer. If a commit was staged when the guard dropped, the guard
  defers its commit on the cell (`SessionCell::stage_or_defer`); whatever
  settles the staged commit stages the deferred one under the same lock,
  and an acknowledgement hands it back to the store as a follow-up
  (`AgentHistoryFollowUp`), which the writer appends next. A store that
  stops appending fails the follow-up with the commit it follows.
- Execution progress (`service/progress.rs`): a tool's
  `AgentToolProgressSink::report` stays synchronous and only queues the
  update on the session's cell, from the call or from a monitor thread that
  outlives it. One consumer task per session, spawned on the turn's runtime
  when an update finds none running and ending when the queue is empty,
  applies and commits the updates in order with awaited appends. The queue
  holds 64 updates; past that an execution's output is coalesced into its
  queued output when nothing of that execution follows it (its text is
  appended, keeping the newest 16 KiB), and the other updates are always
  queued. A turn awaits every update queued before its terminal commit; a
  dropped turn cannot, and updates still queued then commit after its
  cancellation, in order. The application's own reports (a background
  terminal's stop, startup recovery) go through
  `report_session_execution_progress`, which waits (5 s at most) for the
  updates queued before it and commits on the calling thread. A session's
  removal and `shutdown_history` close its queue and abort its consumer;
  `stop_operations` first flushes what was queued until then.
- Request images (`service/sampling/images.rs`): a sampling step, the
  automatic compaction estimate and a compaction collect the image
  attachments the history references under the session's lock, release it
  while the history store loads them on its worker
  (`AgentHistoryStore::enqueue_image_attachment_load`, the session's writer
  for the file store), then re-check under the lock and assemble from the
  loaded bytes. An attachment the history references but that was not
  loaded means the history changed in between (`StaleRevision`); attachments
  are content-addressed, so the request is byte-identical to one read under
  the lock. A compaction keeps its loaded images for its remote replacement
  and its next-request estimate.
- Observers: after a commit is durable and its events are emitted, outside
  every session lock, the kernel notifies the observers
  (`AgentNotification`: `TurnAdmitted` with the turn's origin (`User` or
  `Programmatic`) and the provisional title its admission committed,
  `SamplingCommitted` with the answer's memory
  citations, `TurnFinished` after a completed, cancelled or failed turn's
  terminal commit, `MemoryContributionChanged` with its cause (`Initial`,
  `User` or `Polluted`), `SessionArchived`, `SessionRestored`). They run synchronously, do
  bounded work and cannot change the outcome; the `memory` and `titles`
  plugins have one each.
- Automatic titles: the facade's submit command marks the turn `User` and,
  for a session without a title, passes the submitted text collapsed and cut
  to 36 characters as `AgentTurnInput::provisional_title`. The admission
  commits it with the turn if the session still has no title
  (`session_title_updated` follows `turn_started` and `history_committed`),
  and `TurnAdmitted` reports it. Neither the origin nor the provisional
  title is persisted with the turn; `start_turn` and plan implementations
  are `Programmatic` and untitled.

## 6. Approval (`hanafubuki-agent-approval`)

`AgentPermissionMode` is `Manual` (default), `AutoReview`, or `FullAccess`,
frozen per turn from the setting. Each tool call is classified into an
`ApprovalAction` (kind, risk class, canonical digest) by its handler's
`permission`. The kernel (`route_approval`) first asks the step's frozen
approval rules. A rule can only tighten: any `Deny` rejects the action
without asking anyone (several denials give their reasons sorted and joined),
otherwise any `RequireUser` asks the user (and that prompt offers only
"allow once"), otherwise the session's allowance of the tool authorizes it,
otherwise the turn's mode applies. The user's `AllowForSession` (offered by
a `Manual`-mode prompt, `ApprovalRequest::offers_allow_for_session`) allows
the tool, by `(provider id, tool id, generation)`, for the rest of the
session: kept in the session's runtime state only, so a new Agent instance
(a restart) and a fork start without it, and a changed tool (for MCP, also a
reconnect) asks again. The `skills` plugin's rule requires a human for every
`SkillManagement` action, and the `execution` plugin's rule for a command
whose program is `git` with a `clone` argument. The mode's flow
(`service/interaction/approval/`): `Manual` asks the user
(`ApprovalRequested`, answered with `agent_resolve_approval`, timeout at most
15 minutes); `AutoReview` asks a reviewer model call and records a
`PermissionReview`, and three consecutive denials stop the turn; `FullAccess`
authorizes directly. MCP tools also have per-server and per-tool `Allow`,
`Ask`, `Deny` policies, which the `mcp` plugin applies in `permission`.
Approval yields a one-shot `AuthorizedAction`; mutation tools (instances,
Skill management) recompute their action in `invoke` and require the consumed
approval to match it exactly. Instance mutations go through
`InstanceMutationService` and `InstanceProvisioningService`, so they obtain
the same start permits and maintenance leases as the UI does.

## 7. Execution, filesystem, web

- `AgentExecutionService` (`hanafubuki-agent-execution`) runs `exec_command` through
  reserved Executor processes with a clean environment allowlist plus Codex's
  non-interactive defaults (`NO_COLOR`, `TERM=dumb`, `C.UTF-8`, `cat` pagers;
  the request overrides them), a validated working directory, and output and
  time bounds; PowerShell scripts are prefixed to emit UTF-8. Long commands
  continue as background terminals; tty sessions take `write_stdin` and
  `resize`. Results are plain text (`render_plain_text`); a poll holds back an
  unfinished trailing escape sequence for the next one. The `execution`
  plugin's activity monitor attaches as the execution's observer, and
  disposal of a finished execution waits (bounded) until it has reported the
  final output and status; the activity preview commits only newline-ended
  lines and shows the unfinished one provisionally. The generic argv `start`
  exists only under `test-support`. There is no OS sandbox; approval is the
  gate.
- `WorkspaceFsService` (`hanafubuki-agent-workspace-fs`) resolves paths into immutable plans
  within the workspace roots the host's `WorkspaceRootsPort` names (here
  the integration's: managed instance root first, then registered instance
  directories), protects the application data and log directories,
  revalidates before I/O, and writes atomically. `preview_patch` is pure.
- `WebSearchService` (`hanafubuki-agent-web-search`) searches through DuckDuckGo or SearXNG
  and fetches pages directly or through a reader, with a target policy and a
  cache. The `web` plugin builds it from its configuration.

## 8. History, memory, MCP, models, skills

- `FileAgentHistoryStore` (`hanafubuki-agent-history`) implements the kernel's
  `AgentHistoryStore` port: JSONL records (`AGENT_HISTORY_FORMAT_VERSION`) are
  canonical; the SQLite index is a projection rebuilt when needed (open
  discards one it cannot use; `open_rebuilding_index`, the
  `agent_rebuild_history_index` restart, discards it on request). What open
  cannot admit costs only its session, never the store and never a file: a
  rollout of another format is moved, with the session's attachment
  directory, into a `.recovery` directory beside it
  (`quarantine_file`, `<name>.<pid>.<n>.invalid`, whose contents no
  projection scan reads; the path is logged and is in the session's reason); a rollout that does not
  project (undecodable, or failing the record checks of session, ordinal
  and checkpoint identity, which stay strict) stays in place with a pending repair
  that every later start retries; a session whose file is missing, doubled
  across the active and archived trees or outside the store, or whose
  committed item or image attachment does not read, is left as it is. Each
  such session keeps its id reserved, is refused by transcript and runtime
  reads (`projection_lag`, `ProjectionUnavailable` with the reason) and is
  returned to recovery as a failure, which quarantines it (§5); the other
  sessions open, and so does the Agent. The session list leaves out a row
  whose summary does not decode and refuses that session the same way
  (logged once and told to the store's `AgentHistoryRefusalSink`, through
  which the slot adds it, with its reason, to the Agent status's quarantined
  sessions and announces the change; a rebuilt index projects the summary
  again); the cursor still passes the row, so the rest of the list
  returns. The sweep of
  unreferenced attachments (at start-up, and for a session whose turn
  admission failed) only collects garbage and deletes a blob only when no
  session file still on disk references it. A blob lives under the session
  that staged it but any session may reference it (a fork child keeps the
  source session of a restored prompt's images), so the sweep keeps what
  any projected item references and every 64-digit hexadecimal digest that
  a rollout moved into `.recovery` names (read as bytes once per open, not
  decoded; one that cannot be read keeps every blob); such a blob is freed
  once that file is restored or removed. It deletes nothing while a session
  is refused (its references are unknown), nothing of a session the index
  does not hold, and skips a session it cannot read. Only the index or the
  layout (schema, SQL, directory I/O) fails open. Writes go
  through a per-session writer worker, which acknowledges each append after
  it and its projection are written: `append_commit`/`append_delta` wait for
  the acknowledgement, `enqueue_commit`/`enqueue_delta` only find or start
  the writer (reading the index for the session's file when none runs) and
  queue, holding the session gate for no longer. A follow-up the
  acknowledgement hands back (`AgentHistoryFollowUp`) is appended next,
  before the writer takes its next command, and fails with the append when
  the writer cannot make it. A writer whose thread ended (a panic, logged by
  its `BackgroundWorkerGroup`, which reports it through
  `spawn_observed`'s `BackgroundWorker`) is withdrawn and replaced by the next
  command that holds the session gate, so one failed write does not stop the
  session's history. An append whose record is durable but whose projection
  failed refuses the session's reads only until its writer projects the
  rollout again, on a later append or when the writer stops; a session
  refused for its content (at open, or by the session list) stays refused
  until a repair or the next start judges it again. `enqueue_image_attachment_load` reads a model
  request's attachments (digest and image checks included) on the same
  writer and acknowledges their bytes in order, so the kernel reads no
  attachment file itself; the trait's default, for stores held in memory,
  loads inline. Forks share the parent's prefix through a lineage table.
  A deletion (`delete_sessions_confirmed`, which moves the rollouts and
  each session's whole attachment directory to trash) is refused while a
  session outside it still needs one of its sessions: a fork in the lineage
  table (`ReferencedByFork`), or a session file the index does not hold, a
  rollout moved into `.recovery` or one refused at open and left in place
  with a pending repair, that forks from it (a `forkedFromSessionId` or
  `historyBase` `sessionId`) or names one of its blobs' digests
  (`ReferencedByQuarantinedSession`, naming each such file and its session;
  read as bytes, not decoded, by `store/kept_files.rs`; a file that cannot
  be read refuses every deletion). Nothing of either is touched. Restored
  and opened, such a fork is in the lineage table; removed, it keeps
  nothing. The kernel's `prepare_session_deletion` asks the store first
  (`check_sessions_deletable`), so a refused deletion stops before the
  application revokes the session's memory or stops its terminals, and its
  reason is the delete command's error, which the interface shows.
  `shutdown(deadline)` queues each writer's stop behind its commands and
  waits until the deadline; a writer still busy then is reported
  (`WriterDidNotStop`) and left detached. Nothing durable is lost: a commit
  is acknowledged only once it is on disk, so only appends a writer had not
  acknowledged (never committed for any turn; recovery restores the last
  acknowledged revision) remain unwritten.
- `AgentMemoryRepository`: memory derived from session history by the
  `agent-memory-supervisor` worker and a consolidation sub-agent with its own
  tools; sessions opt in or out of contributing. The `memory` plugin's
  prompt contributor reads the summary and the memory settings as they stand
  for every sampling step (in `prepare`; nothing is cached, since the
  summary file is served as it stands whatever the generation) and returns
  the step's memory policy (the external-context setting, read when a tool
  pollutes, citation parsing and validation, and the streaming filter). The
  kernel decides pollution (`ToolDescriptor::memory_effect`) only under that
  policy, so with the plugin off no session is marked polluted. The
  `memory` plugin's observer signals a scan when a turn has finished (so the
  pass never competes with a live turn for the model), queues citation usage
  after an answer commits, records a committed contribution change in the
  store (signalling a scan for the user's toggle and whenever a session
  stops contributing; a new or forked session that starts contributing,
  through the kernel's `initialize_session_memory_contribution`, is picked
  up by a later scan rather than competing with its first turn), and
  signals a scan when a session is archived or restored; with the plugin
  off none of these happen. Memory's
  own operations (startup, a note appended, a reset, a revoked session) and
  a settings change still signal scans directly. A pass pages through the
  idle history sessions newest first (`AgentMemoryHistoryQuery::older_than`,
  100 a page, at most 1000 a pass) and claims up to `maxSessionsPerPass` of
  them: whether a session may be extracted (contributing, a revision not yet
  extracted, no backoff, no lease) is the memory store's to decide, so newer
  sessions that were already extracted never keep an older one waiting.
  Phase one's evidence budget is 70% of the extraction model's context
  window as a session would get it (`AgentService::model_context_window`:
  configured, else the provider's listing, else the built-in table, else
  160K), at most 120K tokens. A failed job backs off, both phases alike: 30 s
  doubling to a day, phase one per source (`memory_sources.retry_count`,
  `next_retry_at`) and phase two globally (`memory_meta.global_retry_count`,
  `global_next_retry_at`), reset by a success or a reset. Citation usage is
  recorded on the cited sources' current extractions whatever the pipeline
  is doing (a usage change that moves the ranked cutoff of a running
  consolidation fails its publish as stale, which the backoff retries).
  Publishing a consolidation stages and validates the artifacts, commits the
  database (the new generation is then the published one) and only then
  swaps them in; a crash or a failed rename after the commit leaves them in
  `staging/generation-<n>`, which the next open or the next consolidation
  claim installs before anything else reads the artifacts as a baseline.
  Opening the repository migrates a version-4 database in place (it lacked
  the phase-two backoff) and discards an existing database (it is derived
  and rebuilt) only when it is not one, is damaged (`SQLITE_CORRUPT`,
  `SQLITE_NOTADB`) or has another schema (any other version, statements the
  old schema does not fit); a busy, locked or I/O failure fails the open, so
  the Agent's construction fails with the reason and a restart retries, and
  every connection waits up to 5 s for a lock (`busy_timeout`).
  `agent_reset_memory` is the one user-triggered discard. The contribution
  mode is one type, `hanafubuki_foundation::agent_settings::AgentMemoryContributionMode`
  (default `Disabled`), which the kernel persists with the session and the
  store per source, re-exported by the contracts and the memory crate.
- The `titles` plugin's observer queues (bounded, `try_send`) every
  `TurnAdmitted` of a `User` turn that committed a provisional title. Its
  worker (`spawn_scoped_worker`, on the ambient runtime, one title at a
  time, stopped with the plugin) reads the turn's user message
  (`AgentService::turn_user_message`) and the session's Provider from the
  current settings through `AgentTitleApplication` on the `BlockingService`,
  asks a private model session for a title (prompt cut to 960 bytes, 30 s),
  and commits it with `replace_automatic_session_title_if_expected`, a
  compare-and-swap on the provisional title (a rename in the meantime wins)
  whose event goes to the session's retained sink. With the plugin off the
  provisional title stays.
- `McpManager` (`hanafubuki-agent-mcp`): stdio and Streamable HTTP transports, OAuth
  (`McpOAuthManager`, tokens in the keyring), elicitation (form or URL), status
  and catalog events on `agent://mcp-changed`. The application owns its
  lifecycle; the `mcp` plugin (`hanafubuki-agent-plugin-mcp`) only plans and runs
  tools through it. The Agent's start connects the enabled servers in the
  background (`service/mcp/startup.rs`), concurrently,
  `MCP_CONNECT_CONCURRENCY` (4) at a time, each bounded by its own connect
  and initialize timeouts, so the Agent is `Running` before they connect;
  each server's status (`starting`, `ready`, `failed` with its error, for a
  hung server "MCP operation timed out") says how it went. Every connect the
  application makes observes the MCP lifetime token, which shutdown cancels
  first; a cancelled connect disposes the process it spawned.
  `McpManager::shutdown(deadline)` closes every connection, each within
  its `shutdown_timeout` and all by the deadline; one not confirmed closed
  by then stays retired with its cleanup obligation (`cleanup_pending`).
  A connection whose server side ended (`liveness.rs`: the transport the
  rmcp loop drops; for stdio the reader records why, e.g. the exit code)
  makes the server `failed` with that reason at once and is retired, and
  the manager reconnects it with the configuration and secrets it was
  opened with under the module host's `RestartPolicy` (2/10/30 s, at most
  3 in 10 minutes; `reconnect` in the status, `connection_lost` stage
  diagnostics). Once they are used up the server stays `failed` until the
  user's Connect, which, like any connect from outside the manager, empties
  the window; shutdown cancels a scheduled reconnect, and a reconnect that
  completes after shutdown began disposes its process instead of installing
  it. A step's lease keeps the connection it was planned with; a call on
  one that closed gets `McpError::CatalogChanged` ("the MCP server's tools
  changed; search for the tool again or retry the call"). A panic in a
  tool call's client path is contained by `McpTool::invoke`
  (`catch_panic_async`): the call gets "the tool failed with an internal
  error", and `McpManager::fail_after_panic` retires the connection and
  reconnects the server as above (`failed` with "the MCP client panicked
  during a tool call: <message>", diagnostic `tool_call_panicked`); the
  other servers and the `mcp` plugin keep running. The `mcp`
  plugin's descriptors follow the tool annotations: `readOnlyHint` without
  `destructiveHint: true` is a concurrent read, `ReadOnly` (offered in Plan
  mode); anything else is `Exclusive` and `Mutating`. Every MCP tool's
  memory effect is `PollutesContribution` (external content). Approval reads only the destructive hint and
  the server's policy, as before. Tool names are sanitized per segment
  (`my.tool` → `my_tool`).
- MCP settings (`McpSettingsStore` of `hanafubuki-agent-config`,
  `config/agent/mcp.toml`, at most 8 MiB) are our own data and are read
  tolerantly (`hanafubuki_agent_mcp::repair_settings`): unknown keys are ignored and
  kept in the file; a timeout, `maximumConcurrentCalls`, a per-tool
  call limit or a credential generation outside its range (or not a whole
  number) is clamped to the nearest bound; a server that still cannot be read
  or fails `McpServerConfig::validate` (a URL with credentials or a fragment,
  overlapping public and credential headers or environment names, the stdio
  program, argument and environment rules, the filter and policy bounds), a
  duplicate id, one past 64 servers, and an unusable credential are dropped
  alone and every other entry is kept. A file that is not TOML or over the
  bound is replaced by the defaults. Each case keeps a copy of the original
  in `.recovery/` before the repaired values are written. The repairs
  (`McpSettingsRepair`: kind, server id or credential key, field, a reason
  without any value from the file) are logged and kept by the store for
  as long as the Agent that opened it runs, because the next read finds a
  clean file;
  `McpApplicationSnapshot.settingsRepairs` carries them and the MCP settings
  panel shows them until dismissed. Writes stay strict: a server write is
  validated, never clamped. A new server's form starts from
  `McpApplicationSnapshot.serverDefaults` (`McpServerDefaults::default()`:
  enabled, optional, Legacy protocol, timeouts 15/30/30/120/10 s, 4
  concurrent calls, `ask`, `deferred`, OAuth `disabled`).
- `hanafubuki-agent-model-api`: `ModelApiProtocol` `Responses`, `ChatCompletions`,
  `AnthropicMessages`; retry policy; tool lowering. The model listing
  (`ModelCatalogClient`) reports each entry's `context_window` when the
  provider names one (`max_input_tokens`, `context_length`, `max_model_len`
  or `context_window`; anything but a positive integer is ignored). Provider streams are
  external input (`tolerant_decode.rs`): empty tool arguments are `{}`,
  unparseable ones `{}` plus `argument_error`; a missing or null usage count
  is 0 and the other kept, a usage that is not an object or holds a count
  that is not a whole number is no usage; a Chat chunk without `choices`
  records its usage, a Chat or Anthropic stream without a finish or stop
  reason ends with `end_turn` unknown, a Chat tool delta without `index`
  continues the latest call (a new id starts the next one), and a Chat body
  that ends cleanly without `[DONE]` after content completes (with no
  content it stays `StreamInterrupted`; a body cut mid-event stays a
  protocol error). Responses `response.incomplete` is the terminal
  `ModelEvent::ResponseIncomplete { reason }`: the kernel keeps the items
  completed and the text streamed so far (an unfinished reasoning or other
  item is accepted), logs the reason at warn, and fails only when nothing
  was produced. An item completed without `output_item.added` is started
  by the kernel then completed; a missing `summary_index` is 0. Thinking
  is `ModelEvent::ReasoningDelta { item_id, kind, index, delta }` (`kind`
  `Summary` or `Raw`) between a reasoning item's `OutputItemAdded` and
  `OutputItemCompleted` (which carries its `summary` and `content` parts):
  Responses `reasoning_summary_text.delta` and `reasoning_text.delta`, its
  request sending `reasoning: {effort, summary: "auto"}` whenever an effort
  is set (OpenAI sends no summary otherwise); Chat `delta.reasoning_content`
  or `delta.reasoning` (the first that has text), one reasoning item per
  stretch of thinking, ended by the answer, a tool call or the response's
  end; Anthropic `thinking` blocks and their `thinking_delta`
  (`signature_delta` is dropped, and thinking is not requested, since with
  tools its signed blocks would have to be sent back). A 200 body
  under another content type is read as SSE unless it starts with `{`, `[`
  or `<`. Empty responses, auth and HTTP errors stay errors. The kernel builds model
  ports from the protocol. Wire bodies are traced only as described in
  [AGENTS.md](../AGENTS.md#agent-content-logging); every trace record that
  carries content, the Agent's tool calls and results included, is cut by
  `hanafubuki_agent_model_api::bounded_trace_content`.
- Providers: `AgentProviderRegistrySettings` in the Agent's
  `providers.toml`, with one global model policy applied to every provider;
  changed only through `AgentConfig::mutate_providers` with an expected
  revision (the provider commands keep their shape).
- The Agent's configuration (`AgentConfig`, `hanafubuki-agent-config`) is
  three TOML files in `config/agent/`, each its own revision domain and lock:
  `config.toml` (`AgentSettings`: `permissionMode`, `memory`,
  `disabledPlugins`, `webSearchProvider`, `searxngBaseUrl`,
  `webFetchUseJinaReader`; no revision), `providers.toml` and `mcp.toml`
  (both owner-only). Reads are per field (an invalid value takes its
  default, a SearXNG provider without a usable endpoint becomes `Auto`, an
  unusable plugin name or Provider is dropped alone); unknown keys are logged
  and kept; a file that needed repair or cannot be read is copied into
  `.recovery/` before the repaired values are written; writes edit the
  document with `toml_edit` and rename it into place. IPC:
  `agent_settings_read` returns `AgentSettingsView` (the settings, their
  defaults and the web tools' form fields); `agent_settings_write(patch)`
  takes an `AgentSettingsPatch` (strict; `disabledPlugins` is not in it),
  validates it as a whole, applies it to the plugins, signals a memory scan
  and emits `agent_settings_changed`. Both need a running Agent, like the provider
  and MCP commands. On first start (no `config.toml`) the integration
  imports `config/agent.json`, `config/agent-providers.json` and
  `data/agent/mcp-settings.json` tolerantly and leaves them as they are.
- `SkillService` (`hanafubuki-agent-skills`): discovery across repository, system, and user
  roots (the repository roots set per read, `set_repo_roots`, from
  `repository_skill_roots` of the workspace roots), a per-turn catalog, and
  mutations. Every read stamps the roots (names, sizes, modified times) and
  rescans when the stamp or the repository set changed. The Agent's
  `SkillCatalogWatcher` (`hanafubuki-agent`'s `skill_watcher`;
  `notify`: inotify, FSEvents, ReadDirectoryChangesW) watches the roots that
  exist when an instance starts, recursively, ignores
  access events, and invalidates each root that changed once its changes
  have been quiet for 250 ms; a root created later is picked up by the
  stamp on the next read. `allowed-tools` is not parsed. Skill files are read field by field
  (`yaml_fields.rs`): a wrong-typed optional key, an over-long name or
  description, an unreadable nested directory and a bad `openai.yaml` field
  each cost only themselves, with an issue in `SkillCatalogSnapshot::issues`
  whose `severity` is `warning`, beside the `error` issues of directories
  that were not loaded.
- MCP catalogs and output are external input: a bad tool is skipped (or its
  description cut, its output schema dropped) with a Warning diagnostic of
  its server, `nextCursor: ""` ends paging, an undecodable content block is
  a placeholder, over 64 items keep 63 and a note, mime parameters and
  missing base64 padding are normalized. `tools/list`, both resource lists
  and `resources/read` are requested through `Peer::send_request`
  (`hanafubuki-agent-mcp` `listing.rs`, `request::read_resource`), so a result
  rmcp cannot decode arrives as `ServerResult::CustomResult` and is decoded
  entry by entry: an undecodable tool is a `tool_skipped_undecodable`
  catalog issue, an undecodable resource or template is left out with a
  Warning naming it, and an undecodable read entry is the text
  `[unsupported MCP content: resource]`. These requests bypass rmcp's
  response cache. Known limit: rmcp's untagged `ServerResult` decodes a
  result carrying `_meta` and an undecodable entry as a `CallToolResult`,
  whose entries are already gone, so such a page still fails whole. Each
  server's status line (`McpDiagnosticSummary`, in the settings snapshot and
  the `/mcp` card) counts the diagnostics recorded since its latest
  connection attempt by severity and names the most severe, most recent one;
  it opens to all of them.
- Web search configuration the service cannot use omits only `web_search`;
  the ad-hoc memory notes log skips an unreadable line (a log over 64 MiB is
  still `Corrupt`); phase-one extraction output ignores unknown fields and
  repairs the named ones ([plugins.md](rules/plugins.md#external-producers-mcp-servers-skills-web-memory));
  an unknown contribution mode in the memory store reads as `disabled`. The
  memory database is at schema 5 (a version-4 one is migrated in place).

## 9. Presentation

`TauriAgentEventSink` never blocks the Agent: it marks a session dirty, and the
`agent-presentation-dispatcher` worker emits coalesced
`agent://presentation-available` notices. Vue reads with `agent_read_events`
(deltas merged into an `AgentPresentationBatch`) and acknowledges with
`agent_ack_events`; acknowledgement is flow control and never truncates the
journal.

## 10. Shutdown

`hanafubuki_agent::DesktopAgent::shutdown` (`shutdown.rs`) runs the
Agent's shutdown under `AGENT_SHUTDOWN_BUDGET` (20 s); each stage ends by
its own deadline or by the end of the budget, whichever comes first, and a
stage that misses it is reported while the next one runs:

1. Stop the Skill watcher (2 s). Close the slot (Hanafubuki's [backend-spec.md §3.3](https://github.com/licyk/Hanafubuki/blob/main/docs/backend-spec.md#33-shutdown)): cancel a start under way and wait up to
   `START_WAIT` (8 s) for it to settle (its retire is bounded by 5 s). Only
   a `Running` Agent gets the stages below.
2. Stop accepting and cancel turns and compactions (5 s).
3. MCP (4 s): cancel the background connects (each disposes the process it
   spawned) and wait for the start to end, then close every connection and
   confirm its process gone (§8).
4. Stop the Agent plugins: one disposal of the kardia root (5 s). Disposing
   the root retires every plugin's registrations, so no entry remains in a
   kernel registry. A plugin that has not stopped by the deadline is left
   stopping and reported as an `agent-plugins` shutdown diagnostic.
5. Stop memory workers, close history (§8: nothing durable is lost),
   stop service workers and shut down executions (3 s).
6. Sweep the instance's process owner group of the Agent's own Executor
   (1 s): every Agent process its own owners left running is terminated and
   disposed; the application's Executor holds none of them.
7. Stop the instance's runtime with a 2 s bound for its blocking pool; work
   sent to it afterwards fails as stopped.

The worst cases stay inside the budget: a start under way takes at most 8 s
plus the runtime's 2 s; a running Agent at most 5 + 4 + 5 + 3 + 1 + 2 = 20 s.
The Skill watcher's thread normally ends at once; one held inside a catalog
invalidation takes up to its 2 s from what is left, since every stage ends
by the budget's end. The
MCP and plugin stages run on the Agent runtime and are awaited with
`ContainedRuntime::block_on_until`, whose wait does not depend on the runtime
making progress, so a runtime whose workers are all blocked cannot hold the
exit. The plugins own no MCP, memory, history or execution lifecycle, so the
order between their teardowns does not matter. The presentation dispatcher
is stopped afterwards, in the application's sequence, with its own 2 s
deadline, which holds even when its emitter is stuck.

---

