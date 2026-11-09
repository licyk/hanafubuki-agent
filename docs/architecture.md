# Agent architecture

How the Agent is put together: its crates, the plugin host, and the flow of a
turn through tools, context, Skills, MCP and memory, with the reasons for the
choices that are not obvious from the code. It describes; the rules live in
the rule documents and are linked rather than repeated:

- [docs/rules/plugins.md](rules/plugins.md): the plugin
  contracts, registries, tool pipeline, context renderers, external
  producers, the goldens workflow, and the host interface.
- [AGENTS.md](../AGENTS.md): the workspace rules (layout, locks, paths,
  decoding strictness, content logging, validation).
- [docs/spec.md](spec.md): the Agent as built in detail (plugin table, tool
  counts, storage, IPC, shutdown stages).
- Hanafubuki's [modules.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/modules.md):
  the module host `hanafubuki-agent-host` is a copy of, and the `agent`
  module row the Agent runs under in Hanafubuki.

## 1. Overview and goals

The Agent is a kernel (`hanafubuki-agent-kernel`) that owns sessions, turns,
history and the per-step snapshots, extended by capability plugins on
[kardia](https://github.com/licyk/kardia). The kernel reaches plugins only
through registries in `hanafubuki-agent-contracts`; a plugin contributes
tools, tool sources, context sections and their renderers, turn data, prompt
fragments, approval rules, continuation hooks and observers.

The design serves three goals:

- **Fault isolation.** A failure is scoped to its owner: a tool call fails
  as a model-visible error, a plugin as a fault of that plugin (stopped and
  restarted by the host), an MCP server as a fault of that server
  (`Failed`, then reconnected). Only a kernel fault rebuilds the Agent, and
  the Agent failing never stops the application.
- **Extensibility.** A new capability is a plugin and a manifest row; the
  kernel, the contracts and the frontend do not change
  ([Writing a plugin](rules/plugins.md#writing-a-plugin)).
- **What the model sees is pinned.** The goldens record every
  model-visible surface; a change the model sees regenerates them on
  purpose (§11).

## 2. Crates and layering

Dependencies point downward. The table is the workspace's crate inventory
(`crates/<dir>/`, each package `hanafubuki-agent-<dir>` but the facade
`hanafubuki-agent` in `crates/agent/` and the kernel in `crates/kernel/`);
the allowed and forbidden edges are in
[plugins.md](rules/plugins.md#crates-and-dependencies) and are checked by
`cargo xtask check-dependencies` (the layer table `LAYERS` names every
member's allowed dependencies).

| Crate | Role |
| --- | --- |
| `hanafubuki-agent-cli` | The `hanafubuki-agent` binary (§3.6, [cli.md](cli.md)): the terminal interface with no command, `exec` and the management commands over the facade's standalone runtime; depends on the facade and the TUI. |
| `hanafubuki-agent-tui` | The terminal interface (§3.7), ported from the Codex TUI (`rust-v0.93.0`) onto upstream ratatui 0.30 and crossterm 0.29, with Codex's `file-search` for `@` mentions; a library over the facade alone, run by the binary on the started Agent. |
| `hanafubuki-agent` | The facade a host embeds, and the composition: `DesktopAgent`, the `AgentSlot` supervisor, the Agent's own Executor and Skill watcher (§3.4), the plugin host (`AgentHost`) and its manifest, the Agent's own plugins (`src/plugins/`), the IPC facade `AgentApplicationService` (the settings API among it), the host interface (`AgentHostPorts`, §3.5), the standalone runtime with a default for every port (`AgentRuntime`, §3.6), the data directory's lock and the standalone log file (§10), the memory supervisor, and `panic` for the host's hook and the binary's (§3.3). |
| `hanafubuki-agent-config` | The Agent's own configuration (`AgentConfig`, §10): the settings, the Provider registry and the MCP servers in the directory the host names, or, standalone, in the OS directory (`AgentDirs`) with the settings layered (`ConfigLayers`). |
| `hanafubuki-agent-runtime` | The Agent's copy of the contained runtime (`ContainedRuntime`, threads `agent-rt`); the application keeps its own for the feature runtime. |
| `hanafubuki-agent-plugin-{web,workspace-fs,execution,mcp}` | Plugin crates: each binds one domain crate to the contracts. |
| `hanafubuki-agent-kernel` | The kernel: sessions, the turn loop, the tool planner and pipeline, the core tools, request assembly, world state, Skill selection, compaction, approval flows, recovery. Creates the registries it reads (`AgentKernelRegistries`, `src/plugin.rs`). |
| `hanafubuki-agent-contracts` | The extension points: registry services, the tool, tool-source, context, renderer, admission, prompt, approval-rule, continuation and observer contracts, the token estimate (`tokens.rs`), host services. Re-exports the registry and fault mechanisms of `hanafubuki-agent-host` and `CancellationToken`. |
| `hanafubuki-agent-foundation`, `hanafubuki-agent-process`, `hanafubuki-agent-host` | The Agent's own copies of the application's `hanafubuki-foundation`, `hanafubuki-process` and `hanafubuki-module-contracts` (the generic module host `ModuleHost`, `Registry`, leases, `PluginFaults`, `RestartPolicy`, `spawn_scoped_worker`), trimmed to what the Agent uses. The application keeps its originals; the twins are listed in todo.md and a fix is ported by hand. |
| `hanafubuki-agent-model-api` | Provider protocols (Responses, Chat Completions, Anthropic Messages): encoding, streaming decode, tool lowering, the model listing. Knows no plugin and no context section kind. |
| `hanafubuki-agent-history` | `FileAgentHistoryStore`: JSONL rollouts, the SQLite index, per-session writers. |
| `hanafubuki-agent-memory` | The memory repository, extraction and the consolidation workspace. |
| `hanafubuki-agent-mcp` | `McpManager`: transports, OAuth, catalogs, liveness, reconnect, leases. |
| `hanafubuki-agent-skills` | `SkillService`: discovery, parsing, the per-turn catalog, mutations. |
| `hanafubuki-agent-approval`, `hanafubuki-agent-tool-contracts`, `hanafubuki-agent-execution`, `hanafubuki-agent-workspace-fs`, `hanafubuki-agent-web-search` | Shared approval and tool-spec types, and the domain services the plugin crates bind. |

Three boundaries carry most of the design:

- **Kernel and plugins meet only in the contracts.** The kernel depends on
  no plugin, and no plugin on the kernel, the composition or another plugin.
  The kernel injects nothing a plugin can replace, so replacing a plugin
  never restarts the kernel and drops no session.
- **Model-api is the wire, nothing more.** It sends a
  `ModelInputItem::ContextFragment { role, kind, content }` as given and
  treats `kind` as a label. Wrapper text for a section lives with the plugin
  that owns the section; model-api frames only its own items (continuation
  context, compaction summaries, the remote compaction item).
  `hanafubuki-agent-model-api` may not depend on the contracts, and the
  `check-model-api-context` xtask check keeps context section kinds out of its
  sources.
- **The Agent knows no host domain and depends on no host crate.** What
  is a host's (in Hanafubuki: instances, Task Center, the UI context, its
  part of the instructions) reaches the Agent through the host interface
  (§3.5); `check-dependencies` refuses any dependency on a Hanafubuki crate.
  A host depends on the facade, the plugin API and the configuration only
  (Hanafubuki's [agent-integration.md](https://github.com/licyk/Hanafubuki/blob/main/docs/agent-integration.md)).

## 3. The plugin host and the `agent` module

### 3.1 From the host down

The Agent runs under its host; in Hanafubuki that is the `agent` row of the
application's module host:

```
AppModules (Hanafubuki's module host, its docs/rules/modules.md)
└── agent                      feature row, mounted last; its service is the slot
    └── AgentSlot              per-start runtime, kernel-fault restarts, crash-loop stop
        └── AgentHost = ModuleHost<AgentPlugins>
            ├── Core:       agent-host-services (BlockingService)
            │               agent-kernel (publishes AgentKernelRegistries)
            ├── Capability: web, workspace-fs, view-image, execution, mcp,
            │               memory, skills, titles
            ├── host rows:  in Hanafubuki: tasks, instances (Capability), ui-context (Context)
            └── Context:    workspace-context, permission-mode
```

- **The `agent` row** (Hanafubuki's `agent_row`, `src/modules/manifest.rs`)
  is the last row of `AppModules`. Composition builds the slot once (`DesktopAgent`);
  a mount turns the Agent on (`AgentSlot::enable`), the row's stop turns it
  off in order (`AgentSlot::turn_off`) within `AGENT_STOP_BOUND` (8 s). The
  user turns the Agent off and on through `disabledModules` like any
  feature module. On quit, `ApplicationShutdownCoordinator` closes the slot
  before the module sweep, so the row's stop finds it closed. The rules are
  in [modules.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/modules.md#the-agent-module).
- **`AgentSlot`** (`crates/agent/src/slot/`) holds the
  Agent's state (`Disabled`, `Starting`, `Running`, `Faulted`, `Stopping`,
  `Failed`, `Stopped`) and supervises it: each start builds its own
  contained runtime, constructs and recovers the service, starts the
  plugins, then connects MCP servers in the background. A host resolves
  the slot per call (Hanafubuki's command guard,
  `src/agent/commands/guard.rs`), so with the module off its commands
  answer "the Agent is unavailable".
- **`AgentHost`** (`crates/agent/src/host/mod.rs`) is the
  Agent's profile of the generic `ModuleHost`: two core rows, then every
  enabled row of `agent_plugin_manifest` (`host/manifest.rs`), the host's
  rows among them, in order, each after the previous one settled or timed
  out. Configuration derives
  from the Agent's settings and the host's proxy (`AgentPluginSettings`);
  enablement from `disabledPlugins`. What the host
  does (bounds, reconfiguration, retry, safe mode, status, shutdown) is the
  module host's ([modules.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/modules.md#the-module-host)).

### 3.2 Registries and registration as an effect

The kernel creates the registries before the runtime starts and the
`agent-kernel` plugin publishes them; plugins inject them and register
entries. A registration belongs to the calling plugin and leaves with it: a
plugin keeps no handle to undo it, and a stopping plugin first hides its
entries from new snapshots, drains leased calls, then cancels them. The
kernel reads each registry as a snapshot at a fixed point (per sampling step,
per request, at turn admission, after a commit). Nine registries exist:
tools, tool sources, context contributors, context renderers, turn
admission, prompt contributors, approval rules, continuation hooks and
observers. Their uniqueness, read points and combination rules are the
[Extension points](rules/plugins.md#extension-points) table.

A registration that cannot be admitted (a duplicate id, a reserved or
colliding tool name, a second contributor or renderer for a kind) is a
`RegistrationError` that fails the registering plugin at its start; nothing
of it is registered and no turn is affected.

### 3.3 Faults

Three owners, three outcomes:

| Fault | Caught by | Outcome |
| --- | --- | --- |
| Panic in plugin code | The guard on every kernel call into a plugin (`RegistryEntry::guard`/`guard_async`, `PluginFaults::guard`), kardia for `apply`, effects, disposers and listeners | That call's own error path (a tool error, an omitted tool, a skipped contributor, a denial for an approval rule); a `PluginFault` of the owner; the host stops the plugin and mounts it again under `RestartPolicy` (2 s, 10 s, 30 s, three in ten minutes, then `failed` until a retry) |
| Panic in an MCP server's tool call | `McpTool::invoke` (`catch_panic_async` around the server call) | The call's "internal error"; that server `Failed` and reconnected (§8); the `mcp` plugin and the other servers keep running |
| Panic in kernel code | A turn's task, a command guard, the Agent's worker groups, the host's own tasks (`spawn_kernel_task`) | Reported to the instance's `AgentKernelFaultSink`; the slot marks the Agent `Faulted`, stops it and starts a new instance after 2 s, 10 s, 30 s (`AgentRestartPolicy`, the same `RestartPolicy`), rebuilding sessions from disk |

The plugin host runs with `module_faults` off: that flag subscribes a host to
the application's module fault sink keyed by module name, and plugin faults
already reach the Agent's host through its `PluginFaults`.
`automatic_restarts` is the application modules' policy
(`AgentHostOptions::restart_policy`); a plugin whose start failed, rather
than one that panicked, is not restarted by itself. Kernel faults rebuild
the whole Agent because a panicked kernel may leave in-memory session state
half-changed (the locks do not poison, so nothing rolls back), and history
on disk is the consistent copy.

All of this rests on unwinding, which the process's panic hook allows only
for a panic known to be caught. The Agent marks its threads and guards in
its own foundation, so the host's hook (Hanafubuki's
`install_panic_hook`) also asks the Agent
(`hanafubuki_agent::panic::is_panic_contained`): without that
bridge every Agent panic would abort the application. The root's
`platform::panic_hook` tests pin it in a child process, with and without
the bridge. The details are
[Panics in plugin code](rules/plugins.md#panics-in-plugin-code) and
[Host, configuration and status](rules/plugins.md#host-configuration-and-status).
Standalone, the binary installs the foundation's hook itself, as a host
does its own: `panic::configure_crash_record` with the data directory's
crash record, then `panic::install_panic_hook(logging::logging_blocked)`,
where `logging_blocked` says whether the thread is inside the file logger
(the hook then writes to stderr only). Restoring the terminal on a panic is
the TUI's (Phase 5).

### 3.4 Start and quit

A start claims a generation, builds the runtime and process owner group,
reaps process groups an earlier run left (the Agent's own
`data/agent/process-groups.json`), constructs the service and runs
recovery on the runtime's blocking pool, starts the plugins, schedules the
MCP connects (four at a time, each bounded by its own timeouts) and installs
the service as `Running`. A crash-loop marker (`data/agent/start-marker.json`)
stops automatic starts after three unfinished ones. Shutdown runs in fixed
stages under a 20 s budget: close the slot, stop turns, MCP, plugins
(one disposal of the kardia root), memory, history and executions, then the
runtime ([spec.md §10](spec.md#10-shutdown)). The
Agent's processes are its own: `DesktopAgent::start` builds the Agent's
`ExecutorManager` (its copy of `hanafubuki-process`) over the environment
the host hands it, each instance gets it scoped to its owner group, and a
stop sweeps that group. The Skill watcher is the Agent's too: it follows
each running instance's roots and stops first at shutdown. Plugins own
no MCP, memory, history or execution lifecycle, so the order of their
teardowns does not matter.

### 3.5 The host interface

A host adds to the Agent only through `AgentHostPorts`, built for every
Agent instance and passed in `AgentServiceInputs` (Hanafubuki's are built
by `hanafubuki-agent-integration`'s `agent_host_ports`):

- **plugin rows**, mounted between the Agent's capability and context
  plugins, so the UI context's section precedes the workspace roots,
  environment and permission mode as it always has;
- **`WorkspaceRootsPort`**: the roots, the extra read roots, the directory a
  turn's host input selects (for AGENTS.md discovery, §6.3) and the
  directories file search walks;
- **`HostContextPort`**: the `AgentHostContext` (kind and content) steering
  carries, written to the world state like any section of its kind;
- **`AgentHostInstructions`**: who the Agent is (the opening sentence of
  the base instructions, the application the environment section names),
  the application's section after the tool guidance and its ownership
  paragraph in "Approval and ownership", joined into the base instructions
  byte for byte as one file had them;
- **`NetworkSettingsPort`**: the proxy (`AgentProxy`) the web tools, MCP and
  OAuth use, read at start and on `apply_saved_settings`;
- **`SecretStore`**: where MCP OAuth tokens live (the OS keyring in
  Hanafubuki).

None of them names a type of a host's foundation, process or module host
(Hanafubuki's originals are distinct from the Agent's copies), so the
interface takes plain values (`PathBuf`, strings, maps) or the Agent's own
types, and the host adapts at the edge. `DesktopAgent` takes the
host's child-process environment the same way (the inherited overlay at
construction, the proxy overlay through `replace_child_environment`).

Beside the ports the host names the Agent's configuration directory (an
`AgentConfig`; Hanafubuki's is `<app_data>/config/agent/`) and its data
directory (`AgentStoragePaths`; Hanafubuki's is `<app_data>/data/agent`).

Host input travels as `TurnExtensions` the Agent passes on unread
(Hanafubuki's `DesktopAgentUiContextSnapshot`). The other way, a host reads
the Agent's runs through `AgentSlot::list_run_projections`; Hanafubuki maps
them to Task Activity. The rules are in
[The host interface](rules/plugins.md#the-host-interface).

### 3.6 Standalone

Without a host the Agent is composed by `AgentRuntime::standalone`
(`crates/agent/src/standalone/`), from `AgentRuntimeOptions` (the
directories, an optional System Skills directory, the working directory and
`--add-dir`s, the profile, the `-c` overrides, the first start's options)
and an `AgentRuntimeHost` (the sinks and the secret store a consumer
replaces). It builds what Hanafubuki's composition builds — the start
marker, the slot, a `DesktopAgent` and its `AgentLaunch` — with a default
for every port:

| Port | Standalone default |
| --- | --- |
| Configuration | `AgentConfig::layered` on the configuration directory: profile, trusted project and `-c` over the user's `config.toml` (§10, [config.md](config.md)); read once when the runtime is made, so a bad profile fails it, and again by every instance |
| Data | `AgentStoragePaths::under` the data directory |
| `WorkspaceRootsPort` | the working directory first (the base of relative requests), then its Git root, then the added directories; no extra read roots, no selection; search walks each root once (the working directory as part of its Git root) |
| Plugin rows, `HostContextPort`, `AgentHostInstructions` | none (`NoHostContext`, the default instructions: "a coding and operations agent", the application `hanafubuki-agent`): the model sees what the goldens pin |
| `NetworkSettingsPort` | the configuration's `[network] proxy` |
| `SecretStore` | the OS keyring (`keyring` 4, service `org.licyk.hanafubuki-agent.secrets`, on the instance runtime's blocking pool), unless `AgentRuntimeHost::secrets` names a store |
| `BlockingOperationPort` | the instance runtime's blocking pool, as embedded |
| Status, plugin and MCP sinks | no-op until the consumer's; turn and session events go to the sink each call is given |
| Child processes | this process's environment, with the configured proxy over it as a host's overlay puts its own: an explicit proxy sets `HTTP_PROXY` and `HTTPS_PROXY`, `disabled` removes them, both set `NO_PROXY` to the loopback names; `system` leaves the environment as it is |
| System Skills | the embedded ones published into `<data>/skills/system`, or the given directory as it is |

The first start always runs: a command line that starts the Agent asks for
it, so an earlier run's unfinished starts do not keep it off (the crash
marker still counts them). `start` returns at once and
`wait_until_started` returns the running handle or why there is none;
`shutdown` is the embedded sequence (§3.4). The logger is not part of the
runtime: the binary installs it (§10).

The binary is `hanafubuki-agent-cli` ([cli.md](cli.md)), above the facade
and depending on nothing else of the workspace. Every command composes the
runtime from its global flags (`--permission-mode` and `--log-level` become
the last `-c` overrides, `--data-dir` replaces `AgentDirs::data_dir`), then
installs the panic hook and the log file; the commands that read or write
the configuration (`config`, `providers list`, `mcp list`, `doctor`
offline) never start it, so they work beside a running Agent, and the
others start it, which takes the data directory's lock. `exec` brings its
own sinks: the turn's `AgentEventSink` and the slot's status sink feed one
channel the command's thread reads, so the events it prints are the
facade's own (`--json` serializes `AgentEvent` as it is), and it answers
approvals and questions with `resolve_approval` and `resolve_user_input`
from there. A change of the Agent's status during the turn (a kernel
fault) ends the command, since the restarted Agent has a new generation.

### 3.7 The terminal interface

`hanafubuki-agent` with no command runs the terminal interface
(`hanafubuki-agent-tui`) when standard input and output are a terminal; the
binary composes and starts the Agent as for `exec` (the runtime, the crash
record, the panic hook, the log file, the start), with the interface's
status sink (`TuiEvents::status_sink`), then hands it the running handle
(`hanafubuki_agent_tui::run`) and shuts the Agent down with
`AGENT_SHUTDOWN_BUDGET` once the interface has restored the terminal.

It is Codex's TUI at `rust-v0.93.0`, ported over the facade: an inline
viewport at the bottom of the terminal (the running tool calls, the status
line, the composer and the approval and question overlays), with the
transcript written into the terminal's own scrollback above it
(`custom_terminal`, `insert_history`). Codex patches ratatui and crossterm
with git forks, which crates.io cannot take; the port uses upstream ratatui
0.30 (its `scrolling-regions` feature grows the viewport) with crossterm
0.29 through ratatui's re-export, and drops the terminal palette query for
the fixed theme Codex falls back to without it.

- **One adapter.** `agent_bridge.rs` stands where Codex has its
  `ConversationManager` and `CodexConversation`: it maps the turn's
  `AgentEvent`s onto what the chat widget shows (`TurnUpdate`), and the
  interface's requests (`Op`: a message, an interrupt, an approval, an
  answer) onto the facade. A message while a turn runs steers it
  (`steer_turn_with_sink`). Tool events carry neither arguments nor output,
  so a finished call's output, and for `apply_patch` the patch read by the
  Agent's own parser (`preview_patch`), come from the turn's committed
  history (`read_turn_page`); the diff has no line numbers, since that
  reading has none.
- **One loop, one channel.** A thread reads the terminal (`tui/input.rs`)
  into the channel the Agent's sinks also send to (an `emit` only sends);
  the app loop on the calling thread handles each event and draws when a
  frame is due. A facade call that waits runs as a task of the Agent's
  runtime and reports back on the channel, so the loop never blocks on the
  Agent; the approval and answer calls, which do not wait, run on the loop.
- **The terminal is restored** on every return (a drop guard), and on a
  panic that ends the process: the interface's hook restores the terminal,
  then runs the Agent's, which aborts. A contained panic (an Agent thread
  the Agent recovers from) leaves the interface running.
- **Keys.** Enter submits, Shift+Enter (with the keyboard enhancement
  flags) or Ctrl+J inserts a newline, Up and Down walk the session's
  messages; Esc or Ctrl-C interrupts a running turn; Ctrl-C twice, Ctrl-D
  twice on an empty composer, or `/quit` quits.
- **Sessions through the bridge.** The interface opens every session
  through the bridge (`AgentBridge::open`, over `open_session`): a new one,
  one named by `resume`, the latest, or one picked from the resume picker.
  The session's latest transcript items are read back into what the chat
  replays (`agent_bridge/replay.rs`: messages, plans, tool calls with the
  output and the `apply_patch` diff read from their committed items, the
  way a finished call's are), after its header. `/new` and `/resume` switch
  the bridge's session; the events of the one left are ignored.
- **The slash commands** (`slash_command.rs`, the composer's popup) map
  onto the facade in `agent_bridge/commands.rs`: `/new` `create_session`,
  `/resume` the history store's session pages (`list_sessions_page`,
  searched by title, cursor-paged, read on the loop as the picker scrolls),
  `/compact` `compact_session_with_sink`, `/model` the Providers' model
  catalogs (`list_provider_models`) and `set_session_model`, `/mode`
  `set_session_mode` (the session modes; the permission mode is the
  configuration's and `/status` only shows it), `/status`
  `settings_view`, `provider_registry_snapshot` and `workspace_roots`,
  `/mcp` `mcp_snapshot`, `/skills` `list_skills`. `/diff` runs `git diff`
  in the first workspace root: the interface's own short-lived process on
  a thread of its own, not a tool call of the Agent's, so the Agent's
  process ownership does not cover it.
- **Overlays on the alternate screen.** The transcript pager (Ctrl-T), the
  `/diff` pager and the resume picker take over the alternate screen; the
  cells committed meanwhile wait and are written into the scrollback when
  the overlay closes, and the restore leaves the alternate screen when one
  is open (a fatal panic's restore included). The app keeps the session's
  committed cells for the transcript pager.
- **`@` file mentions** come from Codex's `file-search` crate, ported as
  the interface's `file_search` module rather than a crate of its own,
  since only the interface uses it: the workspace roots walked with
  `ignore` (a root nested in another walked once, as its own) into a
  `nucleo` matcher on its threads, re-ranked as the `@token` changes.
  crates.io's `nucleo` 0.5 has no accessor for the match scores Codex's
  pinned git revision reads, so the matches carry none (nothing showed
  them); the matcher waits on a std channel with a deadline where Codex
  selects over `crossbeam` channels; the locks are `parking_lot`'s.

Not ported: Codex's live tail and backtracking in the transcript pager,
its custom prompts, Skill and app mentions and image attachments in the
composer, its status card's account and rate limits, and syntax
highlighting (Codex's `.tmTheme` assets are not shipped). The
third-party attribution of the ported files is checked by `cargo xtask
check-notices` against `architecture/notices-policy.toml`.

## 4. Tools

### 4.1 Planning per step

Every sampling step builds one immutable `AgentToolPlan`
(`AgentToolPlanner`, `crates/kernel/src/tool_catalog/`): the kernel's
core tools (`AgentCoreTool`: `new_context`, `update_plan`,
`request_user_input`), the registered tools (each handler's `availability`
for the step), and the tools each tool source contributes for the step
(`ToolSourceStep` in, `SourcedTool`s and an optional `ToolPlanLease` out),
then `tool_search` when anything is deferred. The plan freezes the step's
approval-rule snapshot and the sources' plan leases with it; a call runs
against the plan that advertised it, and the leases keep what the tool was
planned from (an MCP connection) alive until the step ends.

A sourced tool carries an identity (provider id, tool id, generation) and a
`ToolSourceTrust`: an `Optional` tool that cannot be planned is omitted and
reported in the `tool_catalog_omissions` world-state section, a `Required`
one fails the plan. The kernel validates a sourced tool's schema and spec
once per identity (`AgentToolSpecCache`). Plan mode keeps only `ReadOnly`
tools and `request_user_input`.

### 4.2 Names on the wire

A tool's identity inside the Agent is its `ToolName` (namespace segments and
a local name); the registry and the planner key it by the flattened form
`namespace__name`, which is never parsed back. The wire name is chosen per
request by `ProviderToolTable` (`tool_catalog/provider_table.rs`):

1. `assign_provider_names` writes each planned tool's wire name into its
   provider spec for the step's protocol. Responses takes namespaces, so a
   namespaced function keeps its namespace and local name; elsewhere (and
   for a namespace another provider already uses) the name is flattened.
2. Every name is fitted to the protocol's
   `ToolProviderCapabilities::maximum_tool_name_bytes` (Responses 128,
   counted over `namespace__name`; Anthropic Messages 128; Chat Completions
   64, OpenAI's function-name limit). In planning order the first tool keeps its name; a later colliding
   or over-long one is cut and suffixed with `_` and 12 hex digits of a
   hash of its `(provider id, tool id)`, the same on every step, and a
   collision is reported in `tool_catalog_omissions` as "renamed to …".
3. A returned call is resolved by lookup of its `(namespace, name)`
   (`AgentToolPlan::resolve_call`); the decoders return the two apart
   (`ModelToolCall::namespace`) and history records both, so Responses
   replays a call natively and other protocols replay it flattened
   (`flattened_tool_call_name` in model-api).

Schemas follow the same per-protocol rule: a protocol whose
`strict_json_schema` capability is set (Responses) receives each schema
whole; the others receive `portable_model_schema`
(`tool_catalog/provider_projection.rs`), the keywords every
OpenAI-compatible endpoint accepts. Tools are never sent `strict`, and the
kernel validates arguments against the whole schema either way. The rules
are in [Tool names](rules/plugins.md#tool-names).

### 4.3 Reserved names and collisions

Registration refuses what can be refused early: `RESERVED_TOOL_NAMES` (the
core tools and `tool_search`), a flattened name already registered, and one
longer than a tool id. So only sourced tools, whose catalogs come from
outside, can collide at planning time, and those are renamed, not failed.
Nothing about a name fails a step.

### 4.4 Exposure and deferred tools

`ToolExposure` (`Direct`, `Deferred`, `DirectModelOnly`, `Hidden`) is part of
a tool's descriptor; when the direct catalog is over budget, only sourced
direct tools are deferred, fairly across providers. A deferred tool is found
through `tool_search` (a BM25 index over name, description and schema). The
search commits its output and then a `ConversationItem::ToolSelection`
naming the selected identities. The planner admits the selections found in
the model context since the last compaction, so a selection lasts across
turns until the next compaction, which keeps none; the model then searches
again. The item is not model input: on a provider without native search the
selected tools become Direct in the plan, on one with it the replayed search
output shows them. A selection whose tool is gone or whose generation
changed matches nothing.

## 5. The tool-call pipeline

The kernel owns the pipeline and plugins cannot reorder it. For one step:

1. **Stream and commit.** The response streams to its end; each `ToolCall`
   is committed to history before anything runs.
2. **Disposition** (`service/tool_execution/rejection.rs`). An unknown,
   hidden or unadmitted name, or arguments that did not parse or are too
   large, becomes a model-visible error output and the turn goes on.
3. **Arguments** are normalized against the tool's input schema
   (`tool_catalog/arguments.rs`) and decoded tolerantly by the handler
   ([Model tool arguments](rules/plugins.md#model-tool-arguments)).
4. **Gate.** The call takes the step's gate (`ToolCallRuntime::enter`,
   `parallel_tool_runtime.rs`): an in-flight permit, then read access for a
   `ConcurrentRead` tool or exclusive access, first come, first served.
   Under it run mode policy, the schema check of a sourced tool, and the
   handler's `permission`, so classification sees what the step's earlier
   calls did.
5. **Approval routing** (`route_approval`,
   `service/interaction/approval/`): any rule `Deny` rejects; else any
   `RequireUser` asks the user; else the session's allowance of the tool
   authorizes; else the turn's permission mode decides (`Manual` asks,
   `AutoReview` asks a reviewer model, `FullAccess` allows). A decision
   that waits, for the user or the reviewer, releases the gate and retakes
   it once approved, so the step's other calls proceed; the user is asked
   one prompt at a time per step. `AllowForSession`, offered only where no
   rule required a person, allows the tool by `(provider id, tool id,
   generation)` for the rest of the session, in runtime state only.
6. **Invoke** under an `EntryLease` with a child cancellation, bounded by
   `ToolDescriptor::timeout` or else the turn's `tool_idle_timeout` (10 min).
   An MCP tool's timeout is its server's call timeout plus one elicitation
   wait, because the server's own timeout pauses while the user answers.
   `invoke` re-checks that the approval matches the call it runs
   (`verify_tool_approval`), which turns an approval made stale by calls
   that ran during the wait into an error rather than an unapproved action.
7. **Commit in call order, each as soon as possible**
   (`service/tool_execution/scheduler.rs`, `CompletionSet`): an output
   commits once every earlier call's has. A call that ends in an error,
   cancellation included, commits nothing; the others still finish and
   commit, since their effects happened. On cancel only unfinished calls get
   the synthetic interrupted output (`service/validation/tool_pairs.rs`).
8. **Turn limits** (`AgentHardLimits`, `limits.rs`): past
   `maximum_sampling_steps` (32) the turn fails `budget_exceeded` with every
   earlier output committed; at `turn_deadline` (30 min) the turn is
   cancelled as a user's cancel would (`request_turn_cancellation`) and
   fails `deadline_exceeded`. Neither stop is shown to the model.

The rules for each step are in [Tools](rules/plugins.md#tools) and
[Approval rules](rules/plugins.md#approval-rules).

## 6. Context

### 6.1 Sections, slots and renderers

Everything model-visible that is neither history nor a tool is a context
section of a `ContextSectionKind`, an open identifier persisted by its bare
snake_case name:

- **World-state sections** come from `ContextContributor`s at turn
  admission (`collect_world_state`, `crates/kernel/src/context.rs`),
  are versioned by a SHA-256 of kind and content, and are committed as a
  `WorldStateUpdate` holding only the changes. The kernel writes
  `tool_catalog_omissions` itself per step, and steering replaces the
  host context it carries (here `application_ui_context`) mid-turn.
- **Prompt fragments** come from `PromptContributor`s per request: `prepare`
  (async, once per step, may do I/O) then `contribute` (pure). The `skills`
  plugin renders the catalog (`skill_catalog`), the `memory` plugin the
  summary (`agent_memory`).

The model is shown a section through the `ContextRenderer` of its kind
(`crates/contracts/src/context/renderer.rs`), registered by the
plugin that owns the kind: a `ContextSlot`, a `ContextRole`, a `preamble`,
an `epilogue` and `versioned` (the `<agent_world_state kind version>`
element). A renderer is data, so rendering runs no plugin code. The kernel
registers the renderer of the kind it writes (`tool_catalog_omissions`);
`workspace-context`, `permission-mode`, `skills`, `memory` and the host's
`ui-context` (`application_ui_context`) register theirs. A section without a
renderer, because its plugin stopped, is left out of the request.

Every run of adjacent sections is ordered by slot (`Instructions`,
`WorldState`, `Capabilities`, `Memory`), then by renderer registration
order; the prompt fragments join the run the conversation opens with. Stable
slots come first and the memory summary, which may change every step, last,
which keeps the cached prefix long. The kernel frames the context items that
are history rather than sections: a compacted window's initial context, the
task checklist, a selected Skill's instructions (`SkillContext`).
[Context](rules/plugins.md#context) and
[Context renderers](rules/plugins.md#context-renderers) hold the rules.

### 6.2 Change detection

Only the latest version of each kind is shown, where it last changed; an
earlier version is no longer in the request. So there is no "replaces
previously provided" notice and no section-changed event: the persisted
`WorldStateUpdate` (with its `HistoryCommitted` event) is the change record.

### 6.3 AGENTS.md

The `workspace-context` plugin's `workspace_instructions` section is read
on every turn admission: first the user's own file from the Agent data
directory (`data/agent/AGENTS.override.md`, else `data/agent/AGENTS.md`),
then each workspace root's from the root down to the directory the host's
`WorkspaceRootsPort::selected` names (here the instance the user is looking
at), one
32 KiB budget for all with a fair share each (at most 16 KiB a group).
Truncation is reported in the `environment` section. Because it is re-read
every turn and diffed by digest, an edit mid-session reaches the next turn.

### 6.4 The context window and the token estimate

A session's `context_window_tokens` is resolved when the session is created
or its model selected (`ModelCatalog`, `crates/kernel/src/model_catalog.rs`):
the configured window, else the window the provider's model listing reported
in this run (`ModelCatalogEntry::context_window`, read tolerantly from the
fields providers use), else a small built-in table of model families. It is
written into the session configuration, so every reader is unchanged.
Without one there is no automatic compaction.

There is one token estimate, `hanafubuki_agent_contracts::tokens`
(`estimate_tokens`, `token_budget_bytes`, bytes / 4), used by the kernel,
the Skill catalog budget, MCP resource listings and memory evidence. What
makes the counts agree is that they count the same request:
`prepare_sampling_step` (`service/sampling/request.rs`) builds the step's
tool plan and prepared prompt (`SamplingStep`) before the admission check,
and the check, the step's request and an automatic compaction's check of
the next request all build it with `step_model_request` and count it with
`estimate_model_request_tokens`, tools and memory included.

### 6.5 Compaction

Compaction (`service/compaction/`) replaces history with a local summary or
a Responses compaction, chosen by the session's compaction capability. It
is staged on a clone of the session record and installed as a
`Compacted { checkpoint }` model-context boundary, so a failure discards the
clone and leaves history unchanged. It keeps user messages (newest first,
within a budget) and the summary; it never summarizes sections. The next
request re-renders every section from the registries and re-shows world
state from the checkpoint, so a window compacted mid-turn still carries the
Skill catalog and memory, beside its own initial context (mode and task
checklist).

Triggers: before a turn and mid-turn at the threshold (default 80% of the
window), manual, and after a provider's context-window error. The last is
capped: one compaction and retry per step; a second context-window error
before a response succeeds fails the turn
(`ContextWindowExceededAfterCompaction`, `context_window_exceeded`).

## 7. Skills

The `skills` plugin (`crates/agent/src/plugins/skills/`)
adapts `SkillService` (`hanafubuki-agent-skills`) and contributes the turn's
catalog (turn admission), the catalog fragment and renderer, the Skill
tools and the approval rule for Skill management.

- **Roots are resolved per read.** Repository Skills come from
  `.agents/skills` in each workspace root and its ancestors up to the Git
  root (`repository_skill_roots`), computed from the workspace roots before
  every catalog read, the turn's and the settings surface's;
  `SkillService::set_repo_roots` drops the cached scan when the set changed.
  The user root (`data/agent/skills/packages`) and the bundled System
  Skills follow. A new instance's Skills reach the next turn.
- **Directories, not virtual paths.** The catalog, `skills_list` and the
  `<skill>` context show each Skill's id and directory
  (`ResolvedPath::native_text`), so `exec_command` can run a Skill's
  scripts.
- **The body is delivered once, as `SkillContext`.** `skills_read` without a
  `path` returns where the instructions are and a `ToolContextFragment`; the
  kernel commits it as `ConversationItem::SkillContext` once per turn and
  Skill, so a `$mention` followed by a read delivers it once.
  `SkillContext` is the typed record the once-per-turn check, the active
  Skills and the settings surface read.
- **The watcher** (`SkillCatalogWatcher`,
  `crates/agent/src/skill_watcher.rs`, started by the Agent
  on each running instance and stopped at its shutdown) uses `notify` with
  a 250 ms debounce over the roots that exist when the instance starts and
  calls `SkillCatalogInvalidationPort::invalidate_root` on the slot.
  A root that appears later is caught by the stamp every catalog read
  takes, so the watcher is not re-armed per turn.
- **Unavailable Skills.** A Skill whose declared MCP server is not Ready is
  listed as unavailable with the server, its state and why
  (`SkillTurnCatalog::unavailable`), and `skills_read` of it answers with
  that reason. Connecting the server is the user's action in settings.
- Skills never register or restrict tools; `allowed-tools` is an unknown
  frontmatter key.

## 8. MCP

`McpManager` (`hanafubuki-agent-mcp`) owns connections and is owned by the
application; the `mcp` plugin (`hanafubuki-agent-plugin-mcp`) is a tool
source over it, plus three resource tools. Per step the source binds the
connected servers' catalogs (`McpStepBinding`, its plan lease) and
contributes one tool per catalog entry.

- **Catalog.** Built tool by tool; one bad tool costs itself. Each name
  segment is sanitized per character to `[A-Za-z0-9_-]` and fitted to 128
  bytes; fitting the joined name to the provider is `ProviderToolTable`'s,
  and calls go out by the raw remote name.
- **Annotations** (`McpToolHints`): `readOnlyHint` without
  `destructiveHint: true` makes a `ConcurrentRead`, `ReadOnly` tool (it runs
  beside other reads and is offered in Plan mode); anything else is
  `Exclusive` and `Mutating`. Approval reads only the server's or tool's
  policy and the destructive hint, a missing one counting as destructive.
- **Liveness and reconnect.** Each connection's transport is wrapped
  (`McpClosingTransport`, `liveness.rs`) so the manager learns when rmcp's
  loop ended. When the server's side ended it, the server becomes `Failed`
  with the reason, its entry is retired (its tools leave the next plan; the
  process is disposed with its receipt once no step holds it), and
  `manager/reconnect.rs` reconnects it with the configuration and secrets it
  was opened with, under `RestartPolicy` with one window per server. A
  connect from outside (the user's Connect or Refresh, a settings,
  credential or proxy change) empties the window; shutdown cancels a
  scheduled reconnect.
- **Stale calls.** A step keeps the connection it was planned with: a
  refresh retires the old connection only once no step holds it. A call can
  therefore be stale only when its connection closed, and then it returns
  `McpError::CatalogChanged` without asking for approval, and the model
  searches or retries.
- **Per-server panic containment.** `McpTool::invoke` runs the server call
  under `catch_panic_async`; a panic is that call's "internal error" and
  `McpManager::fail_after_panic` retires the connection
  (`McpConnectionLoss::Panicked`), so the reconnect path above takes over.
- **No Agent-side MCP events.** The window learns status and catalog
  changes from `agent://mcp-changed` and re-reads the snapshot; the planner
  pulls per step, so a status- or catalog-changed event would have no
  consumer.
- **Defaults in Rust.** A new server's form starts from
  `McpServerDefaults::default()`, sent as
  `McpApplicationSnapshot.serverDefaults`; Vue owns no defaults.

The tolerant reading of catalogs, listings and outputs, and the liveness
rules, are in
[External producers](rules/plugins.md#external-producers-mcp-servers-skills-web-memory).

## 9. Memory

Memory is derived from session history in two phases, ported from Codex:
phase one extracts each idle session with a small model, phase two runs a
consolidation sub-agent over the extractions and publishes `MEMORY.md` and
`memory_summary.md`. The `memory` plugin
(`crates/agent/src/plugins/memory/`) contributes the summary
fragment (`agent_memory`, slot `Memory`), the step's memory policy
(`AgentMemoryStep`: the streaming filter, citation validation, pollution),
the memory tool source and the observer that signals the supervisor
(`crates/agent/src/memory/supervisor.rs`). With the plugin
off, memory does nothing to a turn.

- **Eligibility before the limit.** A pass pages through the idle history
  sessions newest first (`AgentMemoryHistoryQuery::older_than`, 100 a page,
  at most 1000 a pass) and claims through the memory store until
  `maxSessionsPerPass` sources are claimed (`SourceClaims`); whether a
  session may be extracted is the memory store's decision, so already
  extracted newer sessions never starve an older one.
- **Backoff in both phases.** 30 s doubling to a day: phase one per source,
  phase two globally (`memory_meta.global_retry_count`,
  `global_next_retry_at`).
- **One limit.** The consolidation agent may write only what publish accepts
  (`MAX_MEMORY_RESOURCE_BYTES`, `MAX_MEMORY_SUMMARY_BYTES`, the summary's
  `v1` line).
- **Usage while dirty.** Citation usage is recorded on the cited sources'
  current extractions whatever the pipeline is doing.
- **Tolerant extraction.** The model's output keeps what its named fields
  still give and repairs the slug; only output that is not an extraction
  object fails.
- **Commit before rename.** Publish stages and validates the artifacts,
  commits the database, then installs them; `install_pending_artifacts`
  installs a committed generation left in staging at the next open or
  consolidation claim.
- **One contribution mode**, `AgentMemoryContributionMode` in
  `hanafubuki_foundation::agent_settings` (default `Disabled`), re-exported
  by the contracts and the memory crate.
- **Evidence budget** from the kernel's window for the extraction model
  (`AgentService::model_context_window`).

## 10. Persistence and compatibility

Persisted formats are unstable by rule, and readers decode tolerantly and
quarantine what they cannot use
([architecture.md](https://github.com/licyk/Hanafubuki/blob/main/docs/rules/architecture.md#decoding-and-validation-by-input-source)).

- **History** (`AGENT_HISTORY_FORMAT_VERSION`, 5): JSONL rollouts are
  canonical and the SQLite index a projection. New items are additive
  (`ToolSelection`, the optional namespace on `ToolCall`); a removed field is
  an unknown key, ignored. `ContextSectionKind` stays a bare name rather than
  an `{ owner, id }` pair because every recorded `WorldStateUpdate` names
  kinds that way; the owner is the plugin that registers the kind's
  renderer. A session recovery cannot use is quarantined, never deleted
  ([spec.md §8](spec.md#8-history-memory-mcp-models-skills)).
- **Memory database**: schema 5. A version-4 database is migrated in place
  (it lacked the phase-two backoff), because the discard path for other
  versions also removes the published artifacts.
- **Configuration** (`hanafubuki-agent-config`): three TOML files in the
  directory the host names, each one revision domain with its own lock:
  `config.toml` (permission mode, memory, disabled plugins, web search and
  fetch; no revision, a write is a patch), `providers.toml` (the Provider
  registry with its revision, tokens verbatim) and `mcp.toml` (the MCP
  servers and credential values with their revision). The two with
  credentials are owner-only (`0600` on Unix) and diagnostics leave them out
  by name. Keys are the camelCase of the IPC types, so one typed model serves
  both, and maps keyed by user names (environment variables, tool names,
  credential keys) keep their names. A read decodes field by field: an
  invalid value takes its default (one bad Provider or MCP server is dropped
  alone), unknown keys are logged and stay in the file, and a file that
  needed repair or cannot be read is copied into `.recovery/` before the
  repaired values are written. A write edits the document on disk with
  `toml_edit` (comments, layout and unknown keys survive; a changed value
  keeps its key's comment) and publishes it by an atomic rename. Each file is
  read once per Agent instance and again before every write, so a hand edit
  between two writes is kept.
- **Layers** (standalone only, `AgentConfig::layered` with `ConfigLayers`;
  [config.md](config.md)): over the user's `config.toml`, lowest first,
  the profile `-p` selects (`[profiles.<name>]`), the project's
  `.hanafubuki-agent/config.toml` when the user's file trusts the project
  (`[projects."<path>"] trustLevel = "trusted"`, compared by `PathKey`;
  the project is the working directory's Git root, or the working
  directory), and `-c key=value` (dotted camelCase paths, TOML values; the
  parser is Codex's `config_override.rs`, ported with its attribution).
  Each layer is decoded over the one below with the same per-field
  decoders, the layer below as the defaults, so a key it leaves out or
  cannot use keeps the value below. Only the user's file is repaired or
  written: a write reads the user's layer alone, patches and writes it, then
  applies the layers again, so no layer's value is ever baked into the
  user's file. A project's file that cannot be read is logged and skipped.
  `[network] proxy` and `logLevel` (`AgentStandaloneSettings`) are read the
  same way; the settings API never writes them, `config set` does
  (`AgentConfig::set_user_value`, below). An embedded configuration has no layers and
  ignores both (their keys are reported unknown), since its host supplies
  the proxy and the logger.
- **Writes by key** (`AgentConfig::value`, `set_user_value`, the CLI's
  `config get|set`): a key is a dotted TOML key (`toml_edit::Key::parse`,
  so a project's path is a quoted segment). A setting's value is read with
  every layer applied; `profiles.<name>.<key>` and
  `projects."<path>".trustLevel` from the user's file. A write changes the
  user's file only, and only that key (the document written is the key's
  path alone, which the `toml_edit` merge applies), after checking that the
  key is a setting (present in the defaults' document), a profile's setting
  or a project's `trustLevel` (`trusted` or `untrusted`), and that its value
  decodes without a repair the file did not already need, so a value the
  Agent would replace by its default is refused instead of written.
- **Data directory lock**: `<data_dir>/.lock`, an advisory lock (std's
  `File::try_lock`: `flock` on Unix, `LockFileEx` on Windows; no `fs4`
  needed) that the OS drops with the process. The history's SQLite index and
  writers, the memory store and the process-group ledger assume one writer
  process, and a start reaps the groups the ledger lists, so two processes
  on one data directory would corrupt the history and kill each other's
  processes. The start marker holds the lock (`AgentStartMarker`, the data
  directory being its file's directory): the first start takes it before it
  touches the marker or the ledger, restarts keep it, and shutdown releases
  it last. That is the same in both modes; a start that finds the lock held
  fails at construction with an error naming the directory, so a host's
  application keeps running without its Agent. Turning a host's Agent off
  (`AgentSlot::turn_off`) keeps the lock until the application quits, so a
  standalone Agent on that data directory is refused meanwhile. A start
  refused the lock writes nothing there: its owner group records processes
  only when they spawn, and it records no start attempt.
- **Logs**: the standalone binary installs `logging::install_file_logger`
  on `AgentStoragePaths::logs_dir`: `hanafubuki-agent.log`, rotated past
  8 MiB into `.1`…`.3` (the oldest dropped), at the configured level
  (`logLevel`, `-c logLevel=…`; `set_log_level` changes it once the
  configuration is read). Records of other crates are kept only at `info`
  and above, since the HTTP and TLS stacks trace request details; the
  content rules of AGENTS.md hold for the Agent's own. A host keeps its own
  logger: the facade installs none unless asked.
- **Settings import**: the Agent's settings used to live in the application's
  `config/agent.json`, `config/agent-providers.json` and
  `data/agent/mcp-settings.json`. When `config.toml` does not exist, the
  integration's `import_legacy_agent_settings` (run by the Agent's
  constructor) reads them with the same tolerant decoders, writes the
  configuration (`config.toml` last, so an import that stopped part-way runs
  again) and leaves the old files untouched; the application no longer reads
  their keys. The old `agentEnabled` key is gone; the Agent's enablement is
  the `agent` module in `disabledModules`.
- **Session runtime state** that is not persisted: `AllowForSession`
  allowances and a turn's `TurnExtensions`. A restart or fork starts without
  them.

## 11. Testing

- **Goldens** (`crates/agent/src/tests/golden.rs`, fixtures in
  `crates/agent/src/tests/golden/`, over the Agent's own plugins with a host
  that adds nothing) pin what the model receives: Default and Plan
  mode, a tool round trip and its presentation events, a Skill catalog,
  workspace instructions, the memory summary, a compacted request, MCP tools
  from a scripted in-process server
  (`crates/agent/src/test_support/mcp_server.rs`, direct and
  deferred, long and colliding names) and a `tool_search` round trip. The
  fixtures a host's tests build the Agent with are the composition's
  `test_support` (feature `test-support`); a host keeps goldens over its own
  rows (Hanafubuki's in `hanafubuki-agent-integration`). A model-visible
  change regenerates them on purpose with
  `HANAFUBUKI_AGENT_UPDATE_GOLDEN=1` and a reviewed fixture diff
  ([Testing a plugin](rules/plugins.md#testing-a-plugin)).
- **The history corpus** (`crates/agent/src/tests/golden/history/`) is
  history an earlier build recorded (a copy of Hanafubuki's, byte for byte);
  it must keep opening and continuing. It is re-recorded only with
  `HANAFUBUKI_AGENT_UPDATE_GOLDEN_HISTORY=1` after an intended format
  change.
- **Pipeline tests** (`crates/agent/src/tests/pipeline.rs`)
  drive the real kernel and the Agent's own plugins through whole turns with a scripted model and the scripted MCP
  server: exclusive calls and commit order, cancel with finished and
  running calls, approval while the step proceeds and `AllowForSession`, an
  MCP call past its timeout, a panicking MCP call failing only its server,
  unknown tools and bad arguments, namespaced wire names, and Plan mode's
  read-only MCP tools.
- **Kernel contracts** (`crates/kernel/tests/`): `loop_contract`,
  `compaction_contract` (including the capped retry and re-rendered
  sections), `persistence_contract`, `contracts`.
- **Faults**: `plugin_faults.rs`, `restart.rs` and `plugin_host.rs` in the
  facade's tests; the `agent` row's quit order is Hanafubuki's
  (`src-tauri/src/modules/tests/agent.rs`).
  Test plugins (`new_with_test_plugins`) get no automatic restart, so a test
  can watch a plugin stay failed until its retry.
- **Architecture checks** (`cargo xtask check`): the dependency direction
  and layers (no Hanafubuki crate), the source layout, module sizes, strict
  decoding, `check-model-api-context`, path boundaries, locks and the plugin
  panic rule.
- **The panic bridge** is the host's to test: Hanafubuki's
  `src-tauri/src/platform/panic_hook/tests.rs` runs a child process with its
  hook (an Agent runtime panic unwinds with the bridge and aborts without
  it).

## 12. Design choices and alternatives

Short rationale for choices a reader might otherwise undo.

- **No streaming dispatch.** Calls start once the response has streamed and
  its items are committed. Starting each call as its item completes (as
  Codex does) would shorten some steps; the commit and cancel rules (§5) are
  written so it could be added without changing them, and nothing needs it
  yet.
- **One fault owner per MCP server, not a child fiber per server.** A
  registration's owner is the nearest named fiber (`Fiber::name()`). A named
  child (`mcp:<server>`) would fault under a name the host's fault handler
  does not know, which stops rows only, so the fault would only be logged
  and the child would stay registered and panic again on every call. An
  unnamed child faults as `mcp` and stops the whole plugin. Routing child
  faults would need a new mechanism in `hanafubuki-agent-host` and a
  second restart path beside the reconnect, for no isolation the contained
  call lacks: all servers share one process anyway. This was settled by a
  spike through `AgentHost`.
- **The connection is the catalog revision.** A stale MCP call is detected
  by its lease's connection having closed, not by a revision counter. A
  counter would fail calls on a connection a user refresh retired but that
  still runs, breaking the guarantee that a step keeps its connection
  (`manager_refresh_keeps_the_old_generation_until_plan_lease_release`).
  The `AllowForSession` identity is not the revision either: a tool's
  generation folds in the connection generation, a digest of the
  configuration and credential generation, so an allowance survives a
  reconnect of an unchanged tool.
- **The Skill body as `SkillContext`, not as tool output.** Compaction keeps
  neither, so it does not decide between them; the `$mention` path has no
  tool output, and `SkillContext` is the typed record the once-per-turn
  check reads. So the body has one form whichever way a Skill is selected.
- **MCP tools keep the memory effect `PollutesContribution`**, read-only or
  not: a read-only tool's output is still external content, like a web
  fetch.
- **`module_faults` off, `restart_policy` on** for the plugin host (§3.3):
  the flag is about the application's module fault sink, which plugin faults
  never use.
- **Renderers are data.** Rendering a request runs no plugin code, so a
  renderer needs no guard, lease or timeout, and a request cannot fail
  because of one.
- **Registration refuses, planning renames.** Refusing a colliding
  registered tool at the plugin's start fails one plugin once; failing the
  step instead would fail every turn until the plugin is turned off.
  Sourced names come from outside and cannot be refused, so the planner
  renames them deterministically.
- **Turn limits are not model-visible.** History has no item for a failed
  turn's reason; the user reads the turn's error message, and nothing needs
  the model to see one.
- **Not built:** code mode (JS over all tools), a third compaction path,
  background compaction, cross-session MCP connection pooling, deferring all
  MCP tools regardless of catalog size, MCP prompts, subscriptions,
  sampling and roots, Skill-registered tools or `allowed-tools`
  enforcement, a Skill install flow that writes settings from the Agent, and
  an out-of-process Agent.

Compared with Codex (`codex-rs`), which the Agent's mechanisms were checked
against:

| Area | Codex | Hanafubuki |
| --- | --- | --- |
| Tool snapshot | A tool router per step | A tool plan per step, with plan leases |
| Names | `{namespace, name}`, lookup only | The same; one wire-name table per request, fitted per protocol |
| Deferred tools | `tool_search`; selection lives in history | The same, through `ToolSelection` items until compaction |
| Schemas | Reduced keyword set, never strict | Whole where the protocol takes it, portable elsewhere, never strict |
| Approval | Before execution | Outside the gate while a decision waits; `AllowForSession` |
| Tool timeout | None at registry level | Per descriptor, else the turn's idle timeout |
| MCP liveness | Reconnect only for one built-in server | Every server: `Failed`, then bounded reconnect |
| AGENTS.md | Per step; misses in-place project edits | Per turn with a digest diff; user and project files |
| Context sections | Typed world state | Typed sections with plugin-owned renderers and slots |
| Compaction | Several paths; initial context re-rendered | Local and Responses; every section re-rendered; capped retry |
| Memory | Two phases, leases, citations | The same design, with eligibility before the limit and backoff in both phases |

## 13. Known limits and follow-ups

- The file tools' policy roots are fixed when the Agent is built; only
  Skill roots and the context sections follow the workspace per read.
- A session created before its model's window was known keeps none until its
  model is selected again; recovery does not resolve it.
- A `tool_catalog_omissions` update first appended when a step begins is not
  in that step's admission estimate.
- An MCP reconnect reuses the secrets the connection was opened with, so an
  expired OAuth token fails it until the user's Connect.
- rmcp decodes a result carrying `_meta` and an undecodable entry as a
  `CallToolResult`, so such a listing page still fails whole.
- The Skill watcher watches only the roots that exist at start; later roots
  rely on the read stamp.
- Not verified off Linux: the `notify` watcher's macOS and Windows backends
  (FSEvents, ReadDirectoryChangesW), Windows paths in the directories the
  Skill catalog shows (`ResolvedPath::native_text`), the MCP stdio exit
  reason and a dead child's disposal on Windows, and the memory artifact
  install rename on Windows.
- The built-in context windows cover only families whose limits are
  published (GPT, o-series, Claude, Gemini 2.5); any other model needs the
  provider's listing or the setting before it compacts automatically.
- Open items are tracked in [todo.md](../todo.md); the Agent's design
  follow-ups are still in Hanafubuki's
  [todo.md](https://github.com/licyk/Hanafubuki/blob/main/todo.md) (the
  Agent refactor section and "Plugins and host", "Kernel", "Application"
  and "Later capabilities").
