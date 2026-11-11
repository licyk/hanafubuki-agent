# Plan: installable plugins on Kardia, compatible with Codex, with our own plugin format

Repositories:

- **The Agent:** `/root/code_workspace/hanafubuki-agent`, main at `9c8aed8`. It uses kardia-core and kardia-macros 0.2.0.
- **Kardia:** `/root/code_workspace/cordis-rs` (`licyk/kardia`), at the v0.2.0 release. Every crate is published at 0.2.0 on crates.io, and every crate is GPL-3.0-only.
- **pykardia:** `/root/code_workspace/pykardia`, a Python port of Kardia that includes a guest SDK.
- **Codex:** `/root/code_workspace/codex`, at `57a808a392`.

Hanafubuki embeds the Agent through the facade. This work reaches it through:

- the facade;
- the settings;
- the plugin status model;
- the data directory;
- the host twin (`crates/host`), if decision 3 changes it.

Each phase lists its notes for Hanafubuki.

The request has five parts:

1. A plugin system that installs, enables, disables and uninstalls plugins.
2. The rest of the Agent keeps working when one plugin crashes.
3. It is built on Kardia's capabilities.
4. It ports Codex's plugin system, for compatibility.
5. Kardia is extended so plugins in our own format can be developed and run.

| Part | Sections |
| --- | --- |
| 1. Install, enable, disable, uninstall | §2.3, §3, §6 |
| 2. Crash isolation | §5 |
| 3. Built on Kardia | §2.4, §4.2, §4.5 |
| 4. Codex compatibility | §1.2, §3 |
| 5. Our own format on an extended Kardia | §4 |

Phases are in §8 and decisions in §9. What is not built is in §10.

## 0. Terms

"Plugin" already means two things here, and this work adds a third. The plan uses these terms:

| Term | Meaning |
| --- | --- |
| **Built-in row** | A Kardia plugin compiled into the Agent, listed in its manifest (`crates/agent/src/host/manifest.rs:79-121`): `web`, `mcp`, `skills`, `hooks`, `goal` and the rest. Turned off with `disabledPlugins`. |
| **Host row** | A Kardia plugin compiled into a host such as Hanafubuki and passed in through `AgentHostPorts::plugins`. |
| **Installed plugin** | What this plan adds: a **package** installed from a directory, a git repository or a marketplace, with an id of the form `<plugin>@<marketplace>`. |
| **Content** | The parts of a package that are data: Skills, MCP server declarations, hooks, and commands (which become Skills). This is all a Codex or Claude Code plugin is. |
| **Code** | The part of a package in our format that runs: a **guest**, a program speaking Kardia's guest protocol, run **out of process**. |
| **Guest host** | The Agent side that runs guests: one Kardia fiber per guest, with proxies into the Agent's registries. |

## 1. What exists today (verified)

### 1.1 The Agent

**The set of rows is fixed in code.** `agent_plugin_manifest` (`crates/agent/src/host/manifest.rs:79-121`) builds the rows, and host rows are spliced in at line 108. `ModuleHost::new` (`crates/host/src/host/mod.rs:159-208`) takes the rows once and has no way to add or remove one later ("the manifest is fixed", `host/row.rs:81-116`). There is no discovery, package format, install directory or catalog. `AgentStoragePaths` has no `plugins` directory.

**Rows can be turned on and off live, but only names already in the manifest.**

- `set_plugin_enabled` (`crates/agent/src/service/plugins.rs:72-87`) writes `disabledPlugins`.
- `ModuleHost::reconfigure` (`host/reconfigure.rs:24-109`) then retires or mounts the row without restarting the Agent.
- `check_enablement` (`host/mod.rs:304-321`) refuses unknown names.
- Neither the TUI nor the CLI has a plugin command. `retry_plugin` has no caller in this repository, and safe mode cannot be reached from the CLI.

**Crashes are contained only within the process.**

- **Panics.** A panic at any kernel→plugin call is caught (`PluginFaults::guard`/`guard_async`, `crates/host/src/fault.rs:197-223`) and becomes a `PluginFault`. The row is then retired and remounted after 2 s, 10 s and 30 s, at most three times in 10 minutes (`crates/host/src/restart.rs:11-35`).
- **Start and stop bounds.** A start is bounded at 20 s. A stop is reported stuck after 10 s and never aborted (`host/bounds.rs`).
- **Calls with no timeout.** Context `contribute` and turn-admission `contribute` (`crates/kernel/src/context.rs:26-45`, `admission.rs:22-38`), tool-source `contribute` (`tool_catalog/sourced.rs:45-70`) and prompt `prepare` (`service/sampling/prompt.rs:67-100`) have none. A hung contributor blocks every new turn until its row is disabled.
- **Aborts.** An abort, segfault, stack overflow or out-of-memory error in plugin code kills the process. When the Agent is embedded, it takes Hanafubuki with it.

**Out-of-process work is already supervised for two kinds of extension.**

- **MCP stdio servers** are spawned through the `ExecutorManager` as reserved pipe processes (`crates/mcp/src/stdio/process.rs`). They have reader workers, liveness checks, reconnects under the restart policy (`manager/reconnect.rs`), and per-server panic isolation (`fail_after_panic`).
- **Hook commands** are the one-shot version (`crates/hooks/src/command.rs`).
- **The sandbox helper** (P14) can confine a process.

**Installable things today:**

- **Skills** (`crates/skills`): `SKILL.md` with Codex's `agents/openai.yaml`. The roots are `system`, `user` (the managed `skills/packages/`) and repository roots. Install copies a directory in through staging and publish.
- **MCP servers:** `mcp.toml`.
- **Hooks:** Codex's format, ported, read from the user and project directories only. 5 of Codex's 14 events run. Trust is a hash of each hook (`hooks-state.json`).

### 1.2 Codex's plugin system

**A Codex plugin is content, not code.** It is a directory with a JSON manifest that declares Skills, MCP servers, hooks, apps (ChatGPT connectors) and interface metadata. Codex never loads plugin code: no dlopen, no WASM, no plugin RPC.

**Manifests.**

- Codex probes, in order:
  1. A root `plugin.json` with the Agent Plugins 1.0.0 `$schema`.
  2. `.codex-plugin/plugin.json`.
  3. `.claude-plugin/plugin.json`.
  4. `.cursor-plugin/plugin.json`.

  So Claude Code and Cursor plugins are read natively (probe: `utils/plugins/src/plugin_namespace.rs:43-80`; the legacy schema: `RawPluginManifest`, `core-plugins/src/manifest.rs:48-73`).
- When a component isn't declared, Claude's default locations apply: `skills/`, `.mcp.json`, `hooks/hooks.json`. Each `commands/*.md` file is converted into a Skill at install.
- Hooks receive `PLUGIN_ROOT`/`PLUGIN_DATA` and `CLAUDE_PLUGIN_ROOT`/`CLAUDE_PLUGIN_DATA`.
- A manifest path must start with `./`, contain no `..`, and stay inside the package. A bad path is ignored with a warning.

**Distribution.**

- **Marketplaces** are `.agents/plugins/marketplace.json`, `.agents/plugins/api_marketplace.json` (OpenAI's API marketplace) or `.claude-plugin/marketplace.json` (also the `.cursor-plugin` variant) (`marketplace.rs:20-25`). Their entries have local, `url` (git), `git-subdir` or `npm` sources.
- **Identity** is `<plugin>@<marketplace>`.
- **Installs** go into `plugins/cache/<marketplace>/<plugin>/<version>`, staged and swapped in atomically.
- **Per-plugin data** lives in `plugins/data/<plugin>-<marketplace>` for legacy packages, and in `plugins/data/agent-plugins/<sha256 prefix>` for Agent Plugins (`store.rs:122-147`). **Uninstall keeps it** (`manager.rs:2489-2493`).
- **Enabled state** is only `[plugins."p@m"] enabled` in `config.toml`, with an optional `[plugins._default]`. The marketplace's own enablement also gates a plugin, and `_default` cannot override it (`config/src/types.rs:1119-1122`). There is no enable/disable command; the TUI writes the config value.

**Loading.**

- One bad plugin never fails the others.
- User MCP configuration beats a plugin server of the same name. Among plugins, the first in alphabetical key order wins.
- Reload happens without a restart: the plugin and Skill caches are cleared and the MCP and hook runtimes refreshed.

**Trust.** Nothing is signed. A plugin hook needs approval of a declaration hash plus a SHA-256 of the whole package, and the package is re-hashed before each run.

**Size.** `core-plugins` is 51,991 lines, 27,175 of them outside tests. The OpenAI-specific files are about 10k production lines: the remote Plugin Service and sharing, the curated `openai/plugins` sync, ChatGPT Apps and auth projection, and telemetry. The portable core this plan uses is roughly 6–8k production lines.

**`codex-plugin-host` is not Codex's plugin runtime.** It is the parent side of an experimental sandboxed V8 "UI plugin" helper: the shipped `codex-code-mode-host` binary has a `--codex-ui-plugin-process` mode (`main.rs:99`), but `PluginProcess::spawn` is called only from tests at this commit. Its process design is still worth borrowing:

- a READY handshake (`codex-ui-plugin-process/3`);
- length-prefixed JSON frames of at most 1 MiB;
- a process-group or Job Object kill;
- a 16 KiB stderr tail ring;
- an exit report;
- a failure that closes only the affected instance.

### 1.3 Kardia

- **kardia-core.** A panic in `apply`, a disposer, an effect body or a listener becomes an error. The fiber settles as `Failed`, and everything it registered is reverted. Fibers that depend on it drop to `Pending` and come back when a provider returns. There are no timeouts and no restart policy; the Agent's `crates/host` supplies both. `registry().replace` hot-swaps a plugin's code.
  - **Nothing outside a fiber can put it into `Failed`** (verified). The only route is a load whose `apply` returns an error (`lifecycle.rs:147-160`). A failing async effect cleans up only itself, and the fiber stays `Active` (`effect.rs:394-420`). `restart()` keeps a failed fiber failed; only `update` and `registry().replace` clear the error (`fiber.rs:153-205`).
- **kardia-loader.** It keeps a declarative tree of entries in step with live fibers in both directions: create, remove, `disabled`, config updates written back. `Resolver::resolve(name) -> PluginDef` is the hook for a new plugin format, and `TreeBackend` is the hook for persistence. Realms give each entry, or a labelled group of entries, its own service namespace.
- **kardia-hmr.** It has three guest backends behind one guest ABI: `cordis_meta`, `cordis_apply`, `cordis_call` and `cordis_dispose`, with the host functions `log`, `provide`, `view` and `invoke`. A guest is a real Kardia plugin to the host: a fiber with `inject`, `provide`, config, `update` and dispose.
  - **`ProcessSource`** runs one child per fiber over JSON lines, but:
    - it spawns with `std::process::Command` (`process.rs:33-42`), which breaks the Agent's Processes rule, and it runs the module id as the program, with no arguments (`process.rs:34`);
    - it does not pipe stderr, so a guest's log lines go to the host's terminal;
    - `read_line` has no size limit (`process.rs:131`);
    - its calls block on synchronous I/O with no timeout, and the guest bridge runs `apply` and dispose as blocking calls inside async code (`guest.rs:213-224`);
    - nothing watches for the child exiting, so a dead guest stays `Active` until its next call fails;
    - it spawns a process just to read `cordis_meta` (`guest.rs:149-152`).
  - **`WasmSource`** sets no fuel, epoch or memory limits.
  - **`DylibSource`** gives no isolation.
  - The transport traits (`Code`/`Instance`, `guest.rs:17-34`) are `pub(crate)`, so a host cannot plug in its own transport.
- **kardia-guest and pykardia.** These are the guest SDKs for Rust and Python, with one shared, documented ABI.
- **kardia-bridge.** It is a transport and session layer between a host and a client runtime, not a plugin runtime, and it trusts every client.

## 2. Architecture

### 2.1 One package, two kinds of contents

An installed plugin is a **package**: a directory with a manifest. Packages come in three formats:

| Format | Manifest | Contents |
| --- | --- | --- |
| **Codex / Claude Code / Cursor** | `.codex-plugin/`, `.claude-plugin/` or `.cursor-plugin/plugin.json` | Content only. |
| **Agent Plugins 1.0.0** | Root `plugin.json` with its `$schema` | Content only, at fixed locations. |
| **Ours** | `.hanafubuki-plugin/plugin.json` | A superset of the Codex legacy manifest (same fields and defaults), plus `runtime`, `contributes` and `permissions` blocks for code (§4.1). |

So one package in our format can carry both content and code. Install, enable, disable, update and uninstall work the same way for every format.

### 2.2 Layers

| Layer | Home | Role |
| --- | --- | --- |
| **Package store** | new domain crate `crates/packages` (`hanafubuki-agent-packages`), with no Kardia, kernel or facade dependency | Identity, manifest parsers for the four formats, marketplaces, sources (local, git), the versioned cache, data directories, atomic staged install and uninstall, content hashes, the install record. A pure domain crate, like `skills` and `mcp`. |
| **Content sources** | the existing domains: `skills`, `mcp`, `hooks` | Each gains a **plugin source**: a Skill root with `plugin:` ids, plugin-owned MCP servers, and `HookSource::Plugin`. |
| **Guest host** | a new built-in row, `plugins` (`crates/plugin-packages`), mounted by the facade | Owns a **kardia-loader subtree** with one entry per enabled code package (decision 3). A `Resolver` maps a package id to a `PluginDef` built on the extended kardia-hmr guest bridge. Proxies register the guest's contributions into the Agent's registries. It keeps each guest's state and runs its own restart loop (§4.2), as the MCP manager does for servers. |
| **Facade** | `crates/agent` | Lifecycle API (install, uninstall, enable, disable, update, list, read, marketplaces); applies enablement to the content sources and the guest host; status. |
| **Interface** | `crates/tui` (`/plugins`), `crates/cli` (`plugins`, `plugins marketplace`) | Port of Codex's plugin UI and CLI, restricted to what's ported. |
| **Kardia** | `cordis-rs`: kardia-hmr, kardia-guest, possibly kardia-core; pykardia | The extensions in §4.5. |

**Naming and layers.** `packages` is deliberately not called `plugins`. The `plugin-` crate prefix is what `cargo xtask check-dependencies` and `check-plugin-panics` treat as a plugin crate. The guest host is a plugin crate, so it is `plugin-packages` and both checks apply to it. It needs `RestartPolicy`, which `contracts` does not re-export, so its `LAYERS` row also allows `hanafubuki-agent-host`.

### 2.3 State on disk and in the configuration

**Data directory** (`AgentStoragePaths` gains them, through `foundation::paths`):

```
plugins/cache/<marketplace>/<plugin>/<version>/   installed package (read-only to the Agent after install)
plugins/data/<plugin>-<marketplace>/              the package's own data directory (PLUGIN_DATA), as Codex names it
plugins/data/agent-plugins/<sha256 prefix>/       the same for Agent Plugins packages, as Codex names it
plugins/marketplaces/<name>/                      git-added marketplaces
plugins/install.json                              install record: id, version, source, content hash, granted permissions
plugins/linked.json                               linked packages (docs/plan-modular-architecture.md, M7): id, directory, granted permissions
```

`install.json` (and `linked.json`) is our own persisted state, which changes as packages are enabled and permissions granted. So it is read tolerantly per field and quarantined when unreadable, like the rest of our persisted data. The content hash it records is of the package; the package is what gets re-hashed and compared. Third-party manifests are external producers and are read tolerantly too: one bad field or component costs only itself.

**Configuration** (`config.toml`, layered). The tables are Codex's, but the keys follow this file's camelCase convention (decision 13), since the two programs don't share configuration files:

```toml
[plugins._default]          # optional
enabled = true
[plugins."demo@my-mkt"]
enabled = true
[plugins."demo@my-mkt".mcpServers.server1]   # Codex's per-plugin MCP overlay
enabledTools = ["a"]
[marketplaces.my-mkt]
sourceType = "git"          # git | local
source = "https://…"
ref = "main"
```

As in Codex, a marketplace that is turned off turns off its plugins, whatever `_default` says.

Built-in rows keep `disabledPlugins` (decision 7). The two lists cover different things: compiled-in rows versus installed packages. `/plugins` shows both, labelled.

### 2.4 How the parts fit Kardia

- **Built-in rows and host rows are unchanged.** They are Kardia plugins mounted by `ModuleHost`, with the same faults, restarts and status.
- **Code packages** are fibers in the `plugins` row's kardia-loader subtree.
  - Install or enable a package: `loader.create(entry)`. Disable: `disabled: true`. Uninstall: `loader.remove`. A package's configuration: `fiber.update`. A package's upgrade: `registry().replace`.
  - The `TreeBackend` persists the tree through a settings service the facade provides to the row. Only the facade may depend on the config crate (`xtask/src/dependencies.rs:318-326`), and `TreeBackend::commit` is synchronous, so the service queues the write for the facade's blocking writer.
  - Each guest's fiber is a child of the `plugins` row. Disabling `plugins` itself (in `disabledPlugins`, or by safe mode) stops every code package at once.
- **Content packages** need no fiber. Their content flows into the domains' sources, and those domains are already built-in rows: `skills`, `mcp`, `hooks`.

### 2.5 The facade's view

A new `AgentPackageStatus` sits beside `AgentPluginStatus`. It holds:

- the id, version, format and source;
- enabled state and trust;
- the granted permissions;
- the content it contributed (Skills, MCP servers, hooks);
- for code, the guest's state (from its fiber) and its last exit report;
- issues.

`plugin_status()` stays the built-in and host rows' view. `/warnings` gains package issues.

## 3. Codex compatibility (content packages)

### 3.1 What is ported

Ported from `codex-rs/plugin`, `core-plugin-common` and the portable part of `core-plugins` (§1.2), with `file:line` derivation markers in `architecture/notices-policy.toml`:

- **Identity:** `<plugin>@<marketplace>` with Codex's name rules.
- **Manifests:** the four formats and their probe order; the path rules; Claude's defaults; `commands/` → Skills at install (Codex's migration: frontmatter required, 4,000 bytes at most).
- **Marketplaces:** `.agents/plugins/marketplace.json` and `.claude-plugin/marketplace.json` (also the `.cursor-plugin` variant), with `local`, `url` and `git-subdir` sources. `npm` is decision 9. An invalid entry is skipped; only a malformed file fails its marketplace.
- **Install:**
  - copy into staging;
  - inject a fallback manifest from the marketplace entry when the package has none;
  - re-verify the manifest's bytes;
  - migrate commands;
  - swap the package in atomically, keeping the previous version until the swap succeeds;
  - prune old versions.

  Symbolic-linked manifests are refused. The active version is `local` if present, else the highest semver.
- **Enabled state:** `[plugins."p@m"] enabled` and `_default`. Install writes `enabled = true`, and uninstall removes the entry. As in Codex, uninstall keeps the package's data directory; `plugins remove --purge-data` (and the `/plugins` uninstall confirmation) can delete it too.
- **Precedence:** user MCP configuration beats a plugin server of the same name; among plugins, the first in key order wins. Each conflict is recorded as an issue.
- **Hook trust:** a declaration hash plus a content hash of the whole package (skipping `__pycache__`, `.venv`, `*.pyc` and `*.egg-info`). The package is re-hashed before each hook run, and a changed package stops its hooks until they are approved again.
- **Live reload:** a lifecycle change re-reads the effective package set and refreshes the Skill catalog, the MCP servers and the hooks without restarting the Agent.
- **Model view:** an "available plugins" summary, as Codex's `PluginCapabilitySummary` (description of at most 1,024 characters, on one line). It is a context section of the `plugins` row only when at least one package is enabled, so the goldens of an Agent with no packages don't change.

### 3.2 The three content sources

| Domain | Today | Change |
| --- | --- | --- |
| Skills (`crates/skills`) | Roots `system`, `user`, repo; ids `system:`/`user:`/`repo:` | A `plugin` root per enabled package (and its migrated-command root). Ids `plugin:<p@m>:<skill>`; the model sees Skill names namespaced `<plugin>:<skill>`, as in Codex. The Skill watcher also watches package roots. |
| MCP (`crates/mcp`) | `mcp.toml` servers, user-editable | Plugin-owned servers, translated from `.mcp.json` (both shapes, Claude's keys normalised, `cwd` resolved in the package) or Agent Plugins' `mcp.json` (strict, `${PLUGIN_ROOT}`/`${PLUGIN_DATA}`). They are not written to `mcp.toml` and can't be edited there; they can be overlaid by `[plugins."p@m".mcp_servers.*]`. They run under the existing supervision, one fault owner per server. |
| Hooks (`crates/hooks`) | `HookSource::{User, Project}` | `HookSource::Plugin { id, root, data }` after user and project hooks; package-content trust as in §3.1; the four environment variables. Only the 5 events the Agent runs today run for plugins too; the other 9 are reported, as they are for user hooks. |

### 3.3 What is not ported

- ChatGPT apps and connectors (`apps`, `.app.json`);
- the remote Plugin Service, sharing, `plugin/reconcile`;
- the curated `openai/plugins` sync and recommended plugins;
- analytics;
- `extensions["com.openai"]` (the onboarding Skill);
- the `@openai-bundled` hook allowlist;
- executor (remote environment) discovery;
- Claude Code's output styles, LSP servers and status line, which Codex doesn't read either. Its `agents/` are read as roles since M6 of [the modular-architecture plan](plan-modular-architecture.md) (§3.6).

## 4. Our format: code packages on an extended Kardia

### 4.1 The manifest

The manifest is `.hanafubuki-plugin/plugin.json`. Every Codex legacy field keeps its meaning and its default; the new blocks are:

```jsonc
{
  "name": "jira-helper", "version": "1.0.0", "description": "…",
  "skills": "./skills", "mcpServers": "./.mcp.json", "hooks": "./hooks/hooks.json",   // content, as Codex
  "runtime": {
    "type": "process",                         // "process" | "wasm" (decision 8)
    "command": ["./bin/jira-helper"],          // inside the package; or ["python3", "./plugin.py"] with "interpreter": true
    "protocol": "kardia-guest/2"               // the extended guest protocol (§4.5)
  },
  "contributes": {                              // declared up front: the host builds proxies from this, not from the guest's word
    "tools": [{ "name": "jira_search", "description": "…", "inputSchema": { … } }],
    "contextSections": [{ "kind": "jira_board", "renderer": "markdown" }],
    "prompt": { "slot": "capabilities" },
    "toolLifecycleHooks": true,
    "continuationHooks": true,
    "observers": ["turn_completed"]
  },
  "permissions": {
    "network": true,                           // off by default; per-host grants are decision 14
    "workspace": "read",                        // "none" | "read" (default) | "write"
    "env": ["JIRA_TOKEN"]                       // credentials the user maps at enable time
  },
  "configSchema": { … }                         // the guest's settings (JSON Schema), edited in /plugins
}
```

- **Decoding is tolerant per field, as for any external producer.** An unknown `contributes` entry is ignored with an issue, and a bad tool schema drops that tool only.
- **The meta comes from the manifest.** The guest's `cordis_meta` is not read at load, so loading spawns nothing. The guest's own meta must agree at start, or the start fails.

### 4.2 The guest host

The `plugins` row (`crates/plugin-packages`):

1. **It mounts a kardia-loader subtree** (decision 3) whose `TreeBackend` is backed by the install record and `[plugins]`. One entry per enabled code package, named by its id; its config is the package's settings.
2. **A `Resolver` maps an id to a `PluginDef`.** It is built from the manifest and the extended kardia-hmr guest bridge (§4.5), with a `Spawner` that the Agent supplies, so the process is the Agent's (§4.3). The loader resolves again on every start, so the resolver caches one `PluginDef` per package version; `registry().replace` needs that stable identity to upgrade a package in place.
3. **The guest fiber's `apply`:**
   - starts the process;
   - completes the handshake;
   - sends `apply` with the config;
   - registers the **proxies** for what the manifest declares (§4.4), each an effect of the guest fiber, so they retire with it.
4. **The guest host keeps each guest's state and restarts it itself.** `ModuleHost` restarts whole rows only, never a child fiber in a loader subtree, and nothing outside a fiber can make it `Failed` (§1.3). So, as the MCP manager does for its servers:
   - **On an exit** (from the exit watcher, §4.5), a failed call or a missed deadline, the guest's state becomes `failed` with the exit report. Its proxies answer `unavailable` at once.
   - **Restart:** after the `RestartPolicy`'s delay (2 s, 10 s, 30 s within 10 minutes), the guest host restarts the fiber (`fiber.restart()`). Its effects, the proxies included, retire, and `apply` spawns a new process and registers them again.
   - **A fourth fault in the window** disposes the fiber and leaves the guest `failed` until a retry, which recreates its loader entry.

   Kardia's own `Failed` state is therefore not needed. An API to fail a fiber from outside (§4.5) would let kardia-loader and status read it directly, but the design does not depend on it.

### 4.3 The process

- **Spawned through the Agent's `ExecutorManager`** as a reserved pipe executor in the instance's owner group. It is recorded in `process-groups.json`, reaped at the next start, and terminated only with its receipt (AGENTS.md "Processes").
- **Sandboxed** by the P14 sandbox, extended with a **plugin profile** (P5). Today's `SandboxPolicy` is read-only, workspace-write or full access, and both sandboxed modes let a process read everything (`crates/sandbox/src/policy.rs:21-31`). The plugin profile adds an allowlist:
  - the package directory is read-only;
  - its data directory is writable;
  - the workspace is per `permissions.workspace` (`none`, `read`, `write`);
  - the network is on or off (decision 14), and off unless granted.

  Linux (Landlock) and macOS (Seatbelt) can enforce a read allowlist. Windows' unelevated restricted token cannot deny reads, so there `workspace: "none"` is not enforced and the install consent says so. The environment is cleared, apart from the granted `env` and `PLUGIN_ROOT`/`PLUGIN_DATA`. The working directory is the package's data directory.
- **I/O.** stderr goes to a bounded tail ring (16 KiB, as Codex's helper) that is shown in status and logged at `debug`. Protocol content is logged only at `trace` and bounded (AGENTS.md "Agent Content Logging").
- **The process starts after its trust check:** the package's content hash must match the hash recorded at install or update.

### 4.4 What a guest can contribute

The registries' synchronous methods cannot wait on IPC. So a guest contributes only where the proxy can answer from declared data or through an async call with a deadline:

| Extension point | Proxy | Notes |
| --- | --- | --- |
| Tools | an `AgentToolSource` (sourced tools, like MCP: `Optional` trust, schema validated per identity, renamed on collision) | `availability` is answered from the guest's last state; `invoke` is a `call` with the tool's timeout. A bad schema drops the tool, never the step. Third-party tools are outside the goldens, like MCP's. |
| Context sections | a `ContextContributor` | Async with a deadline (§5.2). Bounded by the 7 plugin world-state kinds and the section sizes. |
| Prompt fragments | a `PromptContributor` | `prepare` is the async call (with a deadline); `contribute` returns what `prepare` stored. |
| Tool lifecycle hooks, continuation hooks | the two hook registries | Async calls with the hook timeouts. |
| Observers | an `AgentObserver` | `observe` queues the event; a worker delivers it, fire-and-forget, dropping events when the queue is full. |
| Approval rules, turn admission | **not offered** (decision 5) | Approval stays the user's and the built-ins'. Admission runs before a turn exists, with no deadline to inherit. |
| Services of other packages (`inject`) | **not offered at first** (decision 6) | A guest sees only the host functions below. |

**Host functions a guest may call.** `log`, plus `view` and `invoke` limited to a small allowlist of Agent services: the session's working directory and the guest's own settings. The list grows only with a first real user, as composition rule 4 requires.

### 4.5 Extending Kardia (in `cordis-rs`)

These changes are made upstream, in the user's own repository, and released before the guest host lands (decision 4):

| Crate | Change | Why |
| --- | --- | --- |
| kardia-hmr | Public **guest transport traits** (today's `Code`/`Instance`, `pub(crate)`), async | So the Agent supplies its own transport and process. |
| kardia-hmr | A **`Spawner`** trait; `ProcessSource` uses it (the default spawns as today) | The Agent spawns through `ExecutorManager`. |
| kardia-hmr | **Async calls with timeout and cancellation**; request ids on the wire; a hung or late reply kills the guest and fails its fiber | `cordis_call` blocks a Tokio worker today with no timeout. |
| kardia-hmr | **Exit monitoring:** a guest whose process exits fails its fiber | Today a dead guest stays `Active` until its next call. |
| kardia-hmr | **Meta from outside:** a `PluginDef` can take its meta from a manifest, so loading needs no process | `load` spawns a process just to read the meta. |
| kardia-hmr | **Command and arguments** for a process guest, and **stderr piped** to a bounded tail the host can read | It runs the module id with no arguments and lets stderr through to the host's terminal. |
| kardia-hmr | **A frame size limit** on the process transport | `read_line` has no bound today. |
| kardia-hmr | The guest bridge's `apply` and dispose **off the async worker** (async transport, or `spawn_blocking`) | They block a Tokio worker today. |
| kardia-hmr | A configurable **WASM `Engine`** (fuel or epoch interruption, `StoreLimits`) | Only if decision 8 wants WASM. |
| kardia-core | **Optional:** an API to fail a fiber from outside (verified missing) | The guest host tracks guest failure itself (§4.2), so this is not required. It would let kardia-loader and plain Kardia hosts see a dead guest as `Failed`. |
| kardia-guest, pykardia guest | Protocol **`kardia-guest/2`**: a version handshake (READY, as Codex's helper), request ids, cancellation, `ping`; version 1 still accepted for existing guests | The SDKs speak the extended protocol; pykardia's process transport tracks it. |
| kardia-guest, pykardia guest | **Licence** (decision 10) | A plugin that links a GPL-3.0-only SDK is itself GPL. |

**Release.** During development the Agent uses `[patch.crates-io]` pointing at a `cordis-rs` branch. The phase that needs the changes lands only after the Kardia release is on crates.io, so CI never builds against a local path. If the changes are additive, they are released as 0.2.x, and nothing changes for Hanafubuki's host twin. Only a kardia-core change that breaks semver forces 0.3.0, which Hanafubuki must take too.

**Dependency rules.**

- `kardia-hmr` comes in with `default-features = false, features = ["process"]` (plus `"wasm"` for decision 8), so wasmtime and libloading stay out unless they're wanted.
- `kardia-loader` and `kardia-hmr` are allowed only in `crates/plugin-packages`. Today `check-dependencies` restricts only kardia, kardia-bridge, kardia-core and kardia-macros (`xtask/src/dependencies.rs:176`), so both are added to that list. The `kardia` umbrella crate stays banned. kardia-hmr itself depends on kardia-loader.

### 4.6 Developing a plugin

- **Rust:** `kardia-guest` (`Guest` trait, `serve::<G>()`).
- **Python:** pykardia's guest SDK (`python -m pykardia.guest.export process …`).
- **Any other language:** the documented protocol.
- **CLI support:**
  - `hanafubuki-agent plugins new <name> [--lang rust|python]` scaffolds a package.
  - `plugins dev <dir>` installs a local directory as `<name>@local` and reloads it on change, with kardia-hmr's `reconcile`.
  - `plugins validate <dir>` checks the manifest, the schemas and the handshake.
- **Examples and docs:** example packages in `examples/plugins/`, and a "Writing plugins" page on the site (both languages).

## 5. Crash isolation

### 5.1 What survives each failure

| Failure | Content package | Code package (guest) | The rest of the Agent |
| --- | --- | --- | --- |
| Bad manifest or component | That component (or package) has an issue; its siblings load | Same; the guest doesn't start | Unaffected |
| MCP server crash | The existing MCP supervision: reconnect under the restart policy | — | Unaffected |
| Hook failure | No effect (a broken hook never stops a call) | — | Unaffected |
| Guest panic | — | Caught by the SDK and returned as an error (kardia-guest does this today) | The call's error path: a tool error to the model, a section left out |
| Guest hang | — | The call's deadline: the guest is killed, its state becomes `failed`, and the guest host's restart loop runs | The turn continues without that contribution |
| Guest abort, segfault or out-of-memory | — | The exit watcher marks the guest `failed`; its proxies answer `unavailable`, and the restart loop restarts its fiber, which retires and re-registers the proxies | Unaffected: the crash is in another process. Embedded, Hanafubuki is unaffected too |
| Protocol garbage or oversize frame | — | Treated as a crash: kill, fail, restart | Unaffected |
| Restart loop | — | After 3 starts in 10 minutes: `Failed`, shown in `/plugins` and `/warnings`, until retried | Unaffected |
| `plugins` row itself panics | Content is unaffected (it's in other rows) | Every guest stops with the row; the row restarts under the policy | Unaffected |

### 5.2 Deadlines in the kernel (decision 11)

The proxies time out their own calls. But a built-in or host row can still hang the calls that have no deadline today. So the kernel gets deadlines at the five untimed points:

- context `contribute`;
- turn-admission `contribute`;
- tool-source `contribute`;
- prompt `prepare`;
- a tool's `permission` (`crates/kernel/src/service/tool_execution/handler.rs:85-90`), which only the turn's cancellation stops today; the `invoke` after it already has a timeout.

Each gets a default bound (for example 10 s, from the turn's limits). A call that runs over is cancelled and treated as a contribution left out, with an issue on its owner. The admission and context calls get a token the turn cancels too: today they receive `CancellationToken::default()` (`context.rs:36`, `admission.rs:34`), which only the plugin's retirement cancels. This is a kernel change. It is model-visible only in the failure case, so the goldens don't change.

## 6. Interface and API

- **Facade:**
  - `packages()` and `package(id)`;
  - `install_package(source, marketplace?)`, which returns what it asks for: permissions to grant, hooks to trust, `env` to map;
  - `uninstall_package`, `set_package_enabled`, `update_package`, `retry_package`, `set_package_settings`;
  - `marketplaces()`, `add_marketplace`, `upgrade_marketplace`, `remove_marketplace`;
  - `grant_package_permissions`;
  - and, because there is no caller today, `retry_plugin` and safe mode become reachable.
- **CLI:**
  - `plugins list|add|remove|enable|disable|update|info|retry`;
  - `plugins marketplace add|list|upgrade|remove`;
  - `plugins new|dev|validate` (§4.6);
  - `--safe-mode` on start.
- **TUI `/plugins`:** a port of Codex's plugin catalog (`tui/src/chatwidget/plugins.rs`, `plugin_catalog.rs`, about 3,000 lines), adapted:
  - **Tabs:** installed, marketplaces, built-in rows.
  - **Package detail:** contents, permissions, trust, guest state and its stderr tail.
  - **Actions:** install, uninstall, enable, disable, update, retry, settings (from `configSchema`), and marketplace add, upgrade and remove.

  Install asks for consent to the permissions and to trust the hooks.
- **Events and status:** a `package_changed` event (installed, removed, enabled, failed) in `exec --json`, which is an appear. The status line can show failed packages.
- **Hanafubuki:** the facade calls; `AgentPackageStatus`; the data directory's `plugins/`; the event.

## 7. Compatibility, security, resources

- **History and goldens.**
  - No history-format change.
  - The goldens of an Agent with no packages are unchanged.
  - The "available plugins" section appears only with an enabled package, and its rendering gets a golden of its own.
- **Configuration.** The new `[plugins]` and `[marketplaces]` tables. `[plugins]` didn't exist before, so it's additive.
- **Trust.**
  - Nothing is signed (as in Codex).
  - Install pins the content hash; a changed package needs re-approval of its hooks and its guest.
  - Code needs explicit consent to its permissions at install and on every permission change in an update.
  - Plugin output never grants approval: tool annotations are hints only, as for MCP.
- **Admin policy.** Codex's `restrict_to_allowed_sources` is out of scope at first (§10).
- **Resources.**
  - One process per enabled code package.
  - Disabling a package stops its process.
  - Shutdown stops guests in the `plugins` row's disposal, bounded by the existing stop stages.
- **Licence.**
  - The process boundary keeps third-party code out of the Agent's address space, so packages are separate programs.
  - The Agent's own crates stay GPL-3.0-only.
  - The guest SDKs are decision 10.
  - Codex-derived files carry their notices markers.

## 8. Phases

Each phase is committed and pushed. Each phase also:

- runs the validation in AGENTS.md (CI on Linux, macOS and Windows);
- regenerates goldens only on purpose;
- reports any module-size exception;
- writes its Hanafubuki notes in `todo.md`;
- updates the docs in both languages, including the architecture page's "Not built" list ("a Skill install flow", "an out-of-process Agent").

Content comes first: P1–P3 deliver installing, enabling, disabling and uninstalling Codex-compatible plugins with no Kardia change.

| Phase | Content | Done when |
| --- | --- | --- |
| **P1** Package store | `crates/packages`: identity, the four manifest parsers, marketplaces (local, git), the versioned cache, data directories, staged atomic install and uninstall, command migration, content hashes, `install.json`; `AgentStoragePaths` `plugins/`; `[plugins]`/`[marketplaces]` in the config crate. Ported tests from Codex's `store_tests`, `marketplace_tests`, `plugin_id_tests` and manifest tests. | Codex's and Claude Code's example plugins install, list and uninstall from a local marketplace and a git one. |
| **P2** Content sources | `plugin:` Skill roots; plugin-owned MCP servers from `.mcp.json`/`mcp.json`; `HookSource::Plugin` with package-content trust; the "available plugins" section; live reload on every lifecycle change. | A Claude Code plugin's Skill, MCP server and hook all work after install, and stop after disable, without a restart. |
| **P3** Lifecycle API, CLI, TUI | The facade calls (§6); `plugins` CLI; `/plugins` (Codex's port); built-in rows in `/plugins` with retry; `--safe-mode`; `package_changed`. | A pty run: add a marketplace, install, disable, enable, uninstall. |
| **P4** Kernel deadlines | §5.2. | A hung context, admission, tool-source or prompt contributor, or a hung tool `permission`, no longer blocks a turn (tests per point). |
| **K1** Kardia: transport, spawner, exit, async calls, meta | `cordis-rs`: §4.5's hmr changes (and, optionally, the fiber-failure API in core); `kardia-guest/2` in kardia-guest and pykardia; tests in both; release (0.2.x if additive). | Kardia's own CI is green; the Agent builds against the release. |
| **P5** Guest host | `crates/plugin-packages`: the `plugins` row, the kardia-loader subtree and its settings-service `TreeBackend`, the `Resolver`, the `ExecutorManager` spawner, the sandbox's plugin profile (§4.3), the exit watcher, the guest state and restart loop (§4.2), and status. | A test guest starts, is disabled, enabled and uninstalled; killing it with `kill -9` marks it failed, and the restart loop brings it back. |
| **P6** Contribution proxies | §4.4: tools, context sections, prompt fragments, the two hook kinds, observers; the host-function allowlist; settings from `configSchema`. | A Rust guest and a Python guest each contribute a tool and a section; their failures stay theirs. |
| **P7** Developer story | `plugins new|dev|validate`; example packages (Rust, Python); the "Writing plugins" page; the protocol reference. | A scaffolded plugin builds, validates and runs under `plugins dev`. |
| **P8** WASM (decision 8) | `runtime.type = "wasm"` with the configurable engine (fuel/epoch, memory limits). | An infinite loop in a WASM guest is stopped and the guest fails alone. |
| **P9** End to end | Crash tests for every row of §5.1 (panic, hang, `kill -9`, abort, protocol garbage, oversize frame, restart loop); a pty smoke test on the binary; final docs. | Every failure leaves the session usable. |

P4 does not depend on the earlier phases and can run alongside them. K1 can also start alongside P1–P3, in `cordis-rs`.

**As built** (todo.md is the record):

- **Line references** are at `9c8aed8`.
- **K1 became two Kardia releases.** 0.2.1 added the guest extensions. 0.2.2 made kardia-loader's `serde_json` `preserve_order` optional, because Cargo turned it on for the whole Agent build and changed what the model sees.
- **K1 also added:**
  - `Fiber::fail`;
  - at the user's request, `kardia_*` aliases for the guest ABI in both hosts, with both SDKs exporting both name sets;
  - pykardia's relicense, which also covers the four `pykardia/core` files every exported guest embeds.
- **The guest host uses both mechanisms.** It keeps its own restart loop and also uses Kardia's `Failed` state. The `plugins` row mounts it as a child fiber rather than being it.
- **The `full_access` wait seen in P6** was the test starting a turn through `start_turn`, which then never read `permissionMode`. It was not a product bug; after P9 `start_turn` reads it (todo.md, P6).
- **P9's crash tests found three bugs, all fixed:** `/warnings` hid failed code; `plugins info`/`retry` printed before the guest settled; their wait ran out where no guest could start.

## 9. Decisions

Each decision has a recommendation. **All were accepted as recommended (2026-11-10).**

1. **Two kinds, one package.** *Recommended:* content (Codex, Claude Code, Cursor, Agent Plugins) and code (our format, a superset), under one store and one lifecycle. *Alternative:* two separate systems.
2. **Where code runs.** *Recommended:* out of process only. In-process third-party Rust (a dylib) gives no isolation and is excluded. *Alternative:* also in-process for trusted plugins, which gives up the crash guarantee.
3. **The dynamic subtree.** *Recommended:* one built-in row (`plugins`) owning a kardia-loader subtree, which leverages Kardia's create, remove, `disabled` and persistence, and leaves the host twin (`crates/host`, ported by hand from Hanafubuki) untouched. *Alternative:* add and remove rows in `ModuleHost`, which gives one status and fault model for every row but costs a twin change in Hanafubuki.
4. **Kardia changes.** *Recommended:* upstream in `cordis-rs`, additive and released as 0.2.x, with `[patch.crates-io]` only during development. *Alternative:* a vendored fork in this repository.
5. **What a guest may contribute.** *Recommended:* tools (as sourced tools), context sections, prompt fragments, tool lifecycle and continuation hooks, and observers; not approval rules or turn admission.
6. **Guests depending on each other's services.** *Recommended:* not at first; each guest sees only the host allowlist. *Alternative:* allow `inject` between packages.
7. **Enablement.** *Recommended:* `[plugins]` for packages (Codex's keys) and `disabledPlugins` for built-in rows, shown together in `/plugins`. *Alternative:* one list for both.
8. **WASM.** *Recommended:* a later phase (P8), after process guests. *Alternative:* now, or never.
9. **Marketplace sources.** *Recommended:* local and git (`url`, `git-subdir`) now; npm later. *Alternative:* npm now (`npm pack --ignore-scripts`, Codex's size caps).
10. **The guest SDKs' licence.** *Recommended:* relicense `kardia-guest` and pykardia's guest SDK permissively (MIT or Apache-2.0), so a plugin's authors choose their own licence. The protocol stays documented either way. *This is yours to decide as Kardia's owner.* *Alternative:* keep GPL-3.0-only, so plugins that link an SDK are GPL.
11. **Kernel deadlines.** *Recommended:* add them at the four untimed points (P4), for built-in and host rows as well as guests. *Alternative:* rely on the proxies' own timeouts.
12. **Codex hook events.** *Recommended:* keep the 5 the Agent runs; the other 9 are reported, as for user hooks. *Alternative:* port the rest of Codex's hook events as part of this work.
13. **Configuration key style.** *Recommended:* Codex's tables (`[plugins]`, `[marketplaces]`) with this file's camelCase keys (`mcpServers`, `enabledTools`, `sourceType`). *Alternative:* Codex's snake_case keys verbatim.
14. **Network permission.** *Recommended:* on or off per package now, which the sandbox can enforce. *Alternative:* per-host grants, which need a network proxy the Agent doesn't have.
15. **Uninstall and data.** *Recommended:* keep the package's data directory, as Codex does, with an explicit purge option. *Alternative:* always delete it.

## 10. Not built

- **From Codex:** apps and connectors; the remote Plugin Service and sharing; the curated sync and recommended plugins; telemetry; admin source restrictions (`requirements.toml`); executor discovery; `@plugin` mentions in the composer (possible later).
- **From Claude Code:** output styles, LSP servers, the status line. Its `agents/` are roles since M6 of [the modular-architecture plan](plan-modular-architecture.md) (§3.6).
- **Our format:**
  - signing;
  - a hosted registry;
  - in-process Rust plugins;
  - guest access to arbitrary Agent services;
  - guest approval rules;
  - guests depending on each other (decision 6);
  - a dylib runtime.
