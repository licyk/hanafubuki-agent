# Plan: a more customizable modular architecture, after KohakuTerrarium

Date: 2026-11-11. Status: implemented. Every §9 decision was accepted as
recommended (2026-11-11), and every milestone, M1 to M7, has landed (2026-11-11).
`todo.md` records what each did, how it deviated from this plan, and what is still
open.

This plan compares the Agent's plugin composition with KohakuTerrarium's module
architecture. It then specifies what to adopt, in which form and in which order.
It is a handoff to the developer who implements it: each milestone in §8 names its
files, its contracts, its tests, the documents it updates, and what Hanafubuki must
change at its next pin. It was written before any of it was implemented; where the
implementation differs, `todo.md` says so.

**Sources.**

- **KohakuTerrarium** at `/root/code_workspace/KohakuTerrarium`, commit `c1d17b45`.
  - The architecture summary this work was to start from
    (`KohakuTerrarium_architecture.md`) is **empty** (0 bytes, untracked).
  - The analysis is therefore taken from the source under `src/kohakuterrarium/`,
    with `docs/en/` and the per-package `README.md` files used only to find the way.
  - KohakuTerrarium is cited by file and function, not by line, since its line
    numbers move.
- **Codex** `57a808a392`, for its agent roles (`codex-rs/agent-roles/`).
- **kardia-core** 0.2.3.
- **This repository** at `e4dbb98`.

Every Agent path cited below was checked against that commit.

## 0. Terms

| Term | Meaning here |
| --- | --- |
| **row** | One compiled-in plugin of the manifest (`crates/agent/src/host/manifest.rs`), mounted by the module host. |
| **guest** | An installed plugin's code, run by the guest host (`crates/plugin-packages`). |
| **package** | An installed plugin: content (Skills, MCP servers, hooks) and/or code ([Installed plugins](../site/content/docs/architecture.en.mdx)). |
| **creature** | KohakuTerrarium's agent definition: a config folder (`config.yaml` plus prompts) that may inherit another. |
| **role** | *New.* A named, inheritable agent definition the Agent applies **per session**: instructions, model choice, a permission cap and a capability scope. This is Codex's term; "profile" is already taken by `[profiles.NAME]` ([Configuration layers](../site/content/docs/config-layers.en.mdx), `#profiles`), which is process-wide. |
| **capability scope** | *New.* The per-session, narrowing-only filter a role produces. It selects which tools, and which plugins' contributions, a session's turns see. |
| **plugin options** | *New.* A row's typed, schema-described settings, which the user edits in the configuration center. |

## 1. What exists today

### 1.1 The Agent (verified)

**Composition.**

- `AgentHost` (`crates/agent/src/host/mod.rs`) is the Agent's `HostProfile` of the
  generic `ModuleHost` (`crates/host/src/host/mod.rs`).
- The manifest is a fixed `Vec<AgentPluginRow>`, built in code by
  `agent_plugin_manifest` (`crates/agent/src/host/manifest.rs`):
  - 15 capability rows;
  - the host's rows;
  - 2 context rows.
- The two core rows (`agent-host-services`, `agent-kernel`) are prepended.
- Mount order is the manifest order. Each row mounts once the previous one settled.

**Configuration of rows.**

- Only `web` derives configuration from the settings (`AgentPluginRow::from_settings`).
  Every other row is `unconfigured`: its services are captured in its `PluginDef`
  constructor through `AgentPluginInputs`.
- The settings a row may follow live as top-level fields of `AgentSettings`
  (`crates/config/src/settings/mod.rs`), for example `webSearchProvider`.
- Tunables of other rows are constants. For instance, the subagents plugin has
  `MAXIMUM_RUNNING_AGENTS = 3` and the wait bounds at
  `crates/agent/src/plugins/subagents/service.rs:43-49`.
- A settings-derived configuration is a `SettingsConfig<S> = fn(&S) -> Result<Value, String>`
  (`crates/host/src/host/row.rs:19`), recomputed on every reconfiguration.

**Enablement.**

- `disabledPlugins` (a set of row names) turns rows off, except the core rows.
- `[plugins."<id>"].enabled` turns packages on and off.
- Both settings are instance-wide, so every session of an instance sees the same
  plugins.

**Extension points.**

- There are ten kernel-owned registries, each with a documented combination rule
  ([Plugin rules](../site/content/docs/plugin-rules.en.mdx), `#extension-points`).
- Registration is an effect of the calling plugin (kardia). It is removed when that
  plugin stops, and running calls are drained by entry leases.
- Every kernel call into plugin code is guarded and bounded.
- Combination never depends on registration order, **with one exception**: context
  renderers are ordered by slot, then by registration order. So:
  - mount order decides the model-visible order of sections;
  - "a remounted plugin's renderers move to the end of their slot" (plugin rules,
    `#context-renderers`). A plugin restart reorders the request and breaks the
    cached prefix.
- `ContextRenderer` (`crates/contracts/src/context/renderer.rs:44`) has no ordering
  field.

**Per-session variation** is limited to:

- `AgentSessionConfig` (`crates/kernel/src/model/session.rs:8`): Provider, model,
  `instructions` (one string), mode, reasoning effort, context window and
  compaction;
- `Plan` mode (only `ReadOnly` tools);
- the `review` plugin's approval rule for sessions of origin `Review`.

The tool plan is built from global snapshots: `build_tool_plan`
(`crates/kernel/src/service/tool_execution/plan.rs:13`), with an
`AgentToolCatalogContext` (`crates/kernel/src/tool_catalog/contributor.rs:4`) that
carries no session-level selection. `ToolPlanContext`
(`crates/contracts/src/tools/handler.rs:75`) carries only model capabilities.

**Subagents.**

- `spawn_agent` creates a child with `create_child_session`
  (`crates/kernel/src/service/session/child.rs:29`), using the parent's model,
  configuration and full tool set.
- The port dropped Codex's `agent_type` on purpose
  (`crates/agent/src/plugins/subagents/tools.rs:9`): "an agent runs its parent's
  model".

**Packages** contribute:

- Skills, MCP servers, hooks, and commands (which become Skills);
- guest code (tools, context sections, prompt fragments, tool lifecycle and
  continuation hooks, observers).

A guest's `configSchema` is read and checked for shape only
(`crates/plugin-packages/src/check.rs:43-52`). Its `[plugins."<id>"].settings` are
**not** validated against it, and no form is generated from it. Claude Code's
`agents/` is listed as not built (`docs/plan-plugin-system.md` §3.3 and §10).

**Configuration center.** `/config` has a hand-written form per domain. A coverage
test, `every_known_scalar_setting_has_an_editor`
(`crates/tui/src/config_center/tests.rs:71`), pins every scalar setting to an
editor.

### 1.2 KohakuTerrarium (from source)

**Module kinds.** Each kind is a Protocol with a base class:

| Kind | Interface |
| --- | --- |
| Input | `modules/input/base.py` |
| Output | `modules/output/base.py`, with named outputs routed by `OutputRouter` |
| Tool | `modules/tool/base.py`, `BaseTool` with execution modes `DIRECT`, `BACKGROUND`, `STATEFUL` |
| Trigger | `modules/trigger/base.py`, ambient wake-ups that push events |
| Plugin | `modules/plugin/base.py`, `BasePlugin` |
| User command | `modules/user_command/base.py` |
| Sub-agent | `modules/subagent/config.py`, a `SubAgentConfig` dataclass, not subclassed |

**Composition and lifecycle.**

- Everything external to the controller is a `TriggerEvent` on one inbox
  (`core/event_inbox.py`). Stackable events fold into one turn.
- The `Agent` is about 16 mixins. Construction order is fixed in
  `core/agent/assembly.py`; start and stop are in `core/agent/startup.py` and
  `core/agent/lifecycle.py`.
- There is no single registry: tools are kept twice (`core/registry.Registry` and
  `Executor._tools`), plus per-kind containers.

**Declaring modules.**

- Every creature-config entry is `{type: builtin|custom|package, module, class, options…}`.
- Non-reserved keys become constructor kwargs (`core/loader.py`, `ModuleLoader`).
- A bare name resolves through built-in catalogs, then installed packages'
  `kohaku.yaml` (`bootstrap/*.py`, `packages/resolve.py`), though not for tools.

**Plugins** (`modules/plugin/manager/manager.py`, `PluginManager`).

- Hooks:
  - transform chains: `pre/post_llm_call`, `pre_tool_dispatch`,
    `pre/post_tool_execute`, `pre/post_subagent_run`;
  - callbacks: `on_load`, `on_agent_start`, `on_event`, `on_compact_*`;
  - contributions: `get_prompt_content`, `get_tool_visibility`, `contribute_tools`,
    `contribute_user_commands`, `runtime_services`, `contribute_termination_check`.
- Plugins are ordered by `priority`.
- `PluginBlockError` is the only veto; every other exception is logged and the chain
  continues (fail-open).
- Applicability filters: `applies_to` by agent name and model regex.
- `get_tool_visibility` results are **intersected**, so plugins can only narrow the
  catalog.
- `option_schema()` (enum/string/int/float/bool/list/dict with bounds) drives every
  editor (Studio, TUI, CLI). `set_options` validates against it and calls
  `refresh_options()`.
- Enabled state and options persist **per session** (`core/agent/history/selection.py`,
  `core/agent/tools/plugin_options.py`).
- Discovered catalog and package plugins are registered **disabled**: "Discovery
  must not activate policy implicitly" (`bootstrap/plugins.py`).

**Creature configuration** (`core/config/load.py`, `merge.py`).

- `base_config` inheritance: a path, `creatures/…` walked upward, or an `@pkg/…`
  reference.
- Merge rules:
  - scalars: the child wins;
  - dicts: shallow merge;
  - `tools`, `plugins`, `subagents`, `triggers`, `mcp_servers`: identity lists keyed
    by `name`;
  - other lists: replaced;
  - `no_inherit` and `prompt_mode: replace` are the escape hatches.
- Prompts are a chain of `system_prompt_file`s, base first.
- `${VAR}` interpolation.

**Packages** (`packages/`).

- `kohaku.yaml` declares slots: `creatures`, `terrariums`, `tools`, `plugins`, `io`,
  `triggers`, `skills`, `commands`, `prompts`, `llm_presets`, `ui`, and more.
- `@pkg/path` references resolve in one place (`resolve_any_path`).
- A name declared by two packages is a hard error.

**Sub-agents** are configured, not coded (`bootstrap/subagents.py`). Each one has:

- its own tool subset (shared instances from the parent registry);
- a prompt;
- a model (`inherit`, a profile or a raw id);
- `max_turns`, a timeout and its own `PluginManager`.

`can_modify` is advisory only: the tool list is the real boundary.

**Multi-agent.** Terrariums (`terrarium/`) wire creatures through broadcast channels
and output wiring, and support transactional hot-plug.

## 2. Comparison

| Dimension | KohakuTerrarium | The Agent | Verdict |
| --- | --- | --- | --- |
| Kernel/plugin boundary | Plugins get `PluginContext` with the whole `Agent` (`host_agent`, `controller`, private fields) | Contracts only; the kernel holds no kardia context; plugins inject declared services | **Agent stronger.** Keep. |
| Failure containment | Hook exceptions logged and skipped: policy fails **open** | Every call guarded and bounded; approval rules fail closed; lifecycle hooks have no say; faults restart the plugin | **Agent stronger.** Keep. |
| Combination | Linear transform chains by `priority`; any plugin may rewrite LLM messages, tool arguments and results | Deny-only / tighten-only / first-match, by rank and id; no rewriting; approval re-checked in `invoke` | **Agent stronger.** Do not adopt transform chains (§7). |
| Ordering of model-visible prompt parts | Explicit `priority` | Renderer slot, then **registration order** (mount order; remount moves to end) | **KT better.** Adopt explicit order (M2). |
| Module configuration | Per-plugin `option_schema`; one schema drives every editor; runtime-validated | One row (`web`) derives config from bespoke `AgentSettings` fields; tunables are constants; guest `configSchema` unused | **KT better.** Adopt typed plugin options (M1). |
| Agent definitions | Creatures: named, inheritable, package-shippable bundles of prompt, model, tools, plugins, sub-agents | None: one instance-wide composition; sessions vary only by model, instructions and mode | **KT better.** Adopt as roles (M3, M4). |
| Per-session customization | Plugin selection and options persisted per session; `get_tool_visibility` narrows per request | Instance-wide enablement only | **KT better.** Adopt a narrowing capability scope (M4), never per-session widening. |
| Sub-agent definitions | Configured: tool subset, prompt, model, budgets | Parent's model and all tools | **KT better.** Role-aware `spawn_agent` (M5). |
| Package contents | Creatures, prompts, presets, io, triggers, UI, … | Skills, MCP, hooks, guest code | Adopt **roles** as a package component (M6). Others not now. |
| Discovery safety | Discovered plugins registered disabled | Packages default on (Codex's `_default`); guests' capabilities limited to declared `contributes` and `permissions` | Comparable. No change. |
| Runtime add/remove | `enable`/`disable` rebuild inventories with rollback; `disable` skips `on_unload` | kardia effects: registrations retire with the fiber, drained; guests are a kardia-loader subtree | **Agent stronger.** Keep (plugin plan decision 3). |
| Hot reload | None for code | Guests restart; kardia-hmr can replace | Optional linked dev installs (M7). |
| Ambient inputs (triggers, inputs, outputs, channels) | First-class module kinds | Turns come from the user or the host | Not adopted (§7). |
| Kernel size | Mid-sized; approval, budget and sandbox policy are plugins | Small; approval and the tool pipeline are kernel-owned on purpose | Keep. |
| Testing | `ScriptedLLM` seam; no plugin helpers | `test_support::mount`, goldens pin model-visible output | Agent stronger. Keep. |

KohakuTerrarium's gains come from treating **what an agent is** as data: a
creature, its plugin options, a sub-agent's definition. They do not come from its
plugin mechanics, which are looser than the Agent's on every safety axis. So the
plan keeps the Agent's mechanism (kardia, the registries, the guards, the deny-only
combinations) unchanged and adds declarative layers above it:

1. Typed options for rows and guests (M1).
2. Explicit context order, so composition order stops being model-visible (M2).
3. Roles: inheritable agent definitions resolved from configuration and packages
   (M3).
4. A kernel-owned, narrowing-only capability scope per session (M4).
5. Subagents spawned in a role (M5).
6. Roles shipped by packages, Claude Code's `agents/` among them (M6).
7. Linked development packages with guest reload (M7).

## 3. Proposed architecture

```text
                 ┌──────────────────────────── definitions (data) ───────────────────────────┐
                 │ built-in roles   package roles (M6)   user/project agents/*.toml   [agents] │
                 │ row descriptors + option schemas (M1)  guest configSchema (M1)             │
                 └───────────────┬────────────────────────────────────────────┬─────────────┘
                                 │ resolve (facade, once per settings/package  │ validate once at
                                 │ publication): inheritance, provenance      │ the config boundary
                                 ▼                                            ▼
   per session  ┌──────────────────────────────┐       per instance ┌───────────────────────────┐
                │ AgentResolvedRole            │                    │ AgentPluginSettings        │
                │  → session overrides         │                    │  (+ pluginOptions)         │
                │  → AgentCapabilityScope ─────┼─ persisted in      │  → row config (fn ptr)     │
                └──────────────────────────────┘  AgentSessionConfig│  → kardia fiber.update /   │
                                 │                                  │    Context::on_update      │
                                 ▼                                  └─────────────┬─────────────┘
   kernel       snapshots per step / admission, filtered by the session's scope   │ (unchanged)
                tools · tool sources · context · turn admission · prompt · continuation
                NOT filtered: approval rules · tool lifecycle hooks · observers · renderers
                                 ▲                                                │
   plugins      rows (ModuleHost, unchanged) · guests (plugin-packages, unchanged)┘
```

Three invariants carry the design:

1. **Definitions narrow; they never widen.**
   - A role, an inherited role or a child's role can only remove tools and plugin
     contributions and tighten the permission mode.
   - Approval rules, tool lifecycle hooks (the user's policy hooks), the sandbox and
     `invoke`'s approval re-check are out of reach of every definition.
   - So a package's role is never more dangerous than the instance it runs in.
2. **The mechanism is unchanged.**
   - No new kardia usage.
   - No dynamic rows in `ModuleHost` (plugin plan decision 3 stands).
   - No change to `hanafubuki-agent-host`, a Hanafubuki twin (§8, per milestone).
   - No new extension point: the scope is kernel-owned data, read where the kernel
     already takes its snapshots.
3. **Resolution happens once, at the boundary.**
   - Roles and options are decoded tolerantly from user files and packages, checked
     once, and passed inward as typed values.
   - The kernel sees an `AgentCapabilityScope`, never a role file.

### 3.1 Typed plugin options (M1)

**Configuration.** A new layered table in `config.toml`, keyed by row name and merged
per row and key across layers, the same way `[plugins]` merges per entry:

```toml
[pluginOptions.subagents]
maximumRunningAgents = 4
defaultWaitSeconds = 30
```

- Guests keep their table, `[plugins."<id>"].settings`: it is the package's, and
  `install.json`/`[plugins]` already key it.
- A row with no options has no table. An unknown row name is kept and ignored, like
  `disabledPlugins`.
- `pluginOptions` cannot appear in a project file unless the project is trusted;
  that is the existing layer rule.

**Schema vocabulary.** One typed Rust schema, `PluginOptionSchema`, in the config
crate (`crates/config/src/plugin_options/`):

```rust
pub struct PluginOptionSchema {
    pub options: Vec<PluginOption>,           // display order
}
pub struct PluginOption {
    pub key: String,                          // camelCase
    pub summary: String,                      // one line for the form
    pub kind: PluginOptionKind,
    pub default: PluginOptionValue,
}
pub enum PluginOptionKind {
    Boolean,
    Integer { minimum: i64, maximum: i64 },
    Number { minimum: f64, maximum: f64 },
    Text { maximum_bytes: usize, secret: bool },
    Choice { values: Vec<String> },
    TextList { maximum_items: usize, maximum_item_bytes: usize },
}
```

- A built-in row declares its schema in its descriptor (below).
- A guest's `configSchema` (a JSON Schema subset) is lowered to this vocabulary once,
  when the package is loaded: `type`, `minimum`/`maximum`, `maxLength`, `enum`,
  `items`, `default`, `description`.
  - A keyword it does not know constrains nothing, as in `validate_dynamic_tool_arguments`.
  - A property it cannot lower is left out with a package issue.
- KohakuTerrarium's vocabulary, which includes `dict`, is wider. Nested objects are
  left out until an option needs one.

**Decoding (our persisted data, tolerant).**

- Each key is decoded on its own.
- A value of the wrong type, or out of range, falls back to the default. It produces
  a configuration warning naming the layer, the row and the key; integers are
  clamped with a note, as tool arguments are.
- An unknown key is kept in the file and ignored, with a warning.
- The result, `ResolvedPluginOptions { values, issues }`, is part of
  `AgentPluginSettings`.

**Writes (the configuration center) are strict.** An invalid value is refused with
the reason before anything is written, like the existing forms. The write path is
`AgentConfig::set_plugin_options(row, patch)`, layered with the override warning C2
added.

**Delivery to the row.**

- `AgentPluginRow::from_settings` takes a non-capturing closure, which coerces to the
  `fn` pointer `SettingsConfig` requires, so `hanafubuki-agent-host` does not change:

  ```rust
  AgentPluginRow::from_settings(Capability, subagents_plugin(inputs.driver.clone()),
      |settings| settings.options_value("subagents")),
  ```

- The plugin decodes its config with `PluginDef::typed::<SubagentsOptions>`. Since
  the values were validated at the boundary, that decode cannot fail on a value from
  the file.
- A row that can absorb a change without restarting registers `ctx.on_update`. The
  subagents plugin does, since its tracking must survive. Otherwise the host's
  reconfiguration restarts it, as today.

**Row descriptors.** A table beside the manifest (`crates/agent/src/host/descriptors.rs`):

```rust
pub struct AgentPluginDescriptor {
    pub name: &'static str,
    pub summary: &'static str,                 // what the plugin gives the model or the user
    pub options: Option<PluginOptionSchema>,
}
```

- A host's rows have descriptors as well. `AgentHostPorts::plugins` becomes
  `Vec<(AgentPluginRow, AgentPluginDescriptor)>`; see decision D2.
- `AgentPluginStatus` gains `summary` and `options: Option<AgentPluginOptionsStatus>`,
  which holds the schema, the effective values with the layer each came from, and
  the issues.
- `/plugins` and the configuration center show them.
- A test pins that every manifest row has exactly one descriptor.

**First users.**

- `subagents` (constants made options):
  - `maximumRunningAgents` (1–16, default 3);
  - `defaultWaitSeconds`, `minimumWaitSeconds`, `maximumWaitSeconds`.
  The bounds stay hard limits in code, and the options choose within them.
- Every guest with a `configSchema`.
- `web` keeps deriving from the shared settings, because the proxy and the search
  endpoint are also the configuration center's network form. Moving it is decision
  D3.

**Configuration center.**

- A generic "Plugin options" form renders any `PluginOptionSchema`: a toggle, a
  bounded number, a choice, a text (masked when `secret`) or a text list.
- It is reachable from `/plugins` (`o` on a row or package) and from
  `/config plugins`.
- The coverage contract extends mechanically: every option of every descriptor has
  the generic editor. Add a case to `every_known_scalar_setting_has_an_editor`, or a
  sibling test, that walks the descriptors.

### 3.2 Explicit context order (M2)

- `ContextRenderer` gains `order: u16`, the position within its slot.
- Every run of adjacent sections in a request is ordered by
  `(slot, order, kind name)`. Registration order no longer matters.
- Ranks are assigned to reproduce today's golden order exactly:
  - the kernel's `tool_catalog_omissions`: 0;
  - the capability plugins: 100, 110, … in manifest order;
  - host rows: the band 500–599;
  - context plugins: 700, 710, …;
  - guests: 900 plus a stable tiebreak by kind.
  So the request goldens do not change, and that is the milestone's acceptance test.
- The only behavioural change: a remounted plugin's sections keep their place. Add a
  golden-style test that restarts a context plugin and asserts the request is
  byte-identical.
- The plugin rules sentence "a remounted plugin's renderers move to the end of their
  slot" is removed, and `#context-renderers` documents the ranks.
- Why: two reasons.
  - Remount stability. A plugin's restart (a fault, a reconfiguration, turning it
    off and on) must not reorder the request and so invalidate the provider's
    cached prefix.
  - Manifest order stops being model-visible. Rows, host rows and guests can then be
    added or reordered for mount reasons alone, with no golden consequence.
- Roles do not depend on it: a scope leaves out contributors, never renderers.

### 3.3 Roles (M3)

#### Definition format (our format, TOML)

```toml
# ~/.config/hanafubuki-agent/agents/explorer.toml
# (or <project>/.hanafubuki-agent/agents/explorer.toml when the project is trusted)
name = "explorer"                         # optional; the file stem otherwise
description = "Reads the code and reports findings; never edits files."   # required
extends = "default"                       # optional; another role's name
instructions = """
You are a read-only explorer. Answer with file paths and line numbers.
"""
instructionsMode = "append"               # append (default) | replace
model = "gpt-5.4-mini"                    # optional; else the parent's / the session's
reasoningEffort = "low"                   # optional
permissionModeCap = "manual"              # optional; tightens only (§3.4)
spawnable = true                          # offered to spawn_agent (M5); default true

[tools]
allow = ["read_file", "search_text", "list_directory", "plugin:web", "provider:mcp.docs"]
deny  = ["fetch_web_page"]

[plugins]
off = ["titles", "goal"]
```

The same entries may be written inline in any `config.toml` layer, which is Codex's
`[agents.<name>]`:

```toml
[agents.explorer]
description = "…"
configFile = "agents/explorer.toml"       # Codex's `config_file`, relative to the layer's folder
```

**Keys and decoding.**

- Keys follow this repository's camelCase (plugin plan decision 13).
- Role files are user-authored persisted configuration, so they are tolerant:
  - each key is read on its own;
  - a wrong type drops that key with an issue;
  - unknown keys are ignored with an issue.
- A role without a `description` is not usable (listed with why), as in Codex
  (`validate_required_agent_role_description`).
- `deny_unknown_fields` is not used. Codex uses it in `RawAgentRoleFileToml`; we
  diverge, as `AGENTS.md` asks.
- A Codex role file's `developer_instructions`, `model` and `model_reasoning_effort`
  are read as aliases of `instructions`, `model` and `reasoningEffort`. A Codex role
  file then works unchanged in our `agents/`. Its other Codex config keys are
  ignored with an issue, since they are process-wide settings here.

**Tool patterns.** Each entry of `tools.allow` and `tools.deny` is one of:

- `name`: a flattened tool name (`namespace__name`), with `*` as the only wildcard
  (for example `mcp__github__*`);
- `plugin:<row or guest:<id>>`: every tool the plugin registers or sources;
- `provider:<provider id glob>`: every sourced tool of a provider (for example
  `provider:mcp.github`, `provider:plugin.demo.*`).

A pattern that matches nothing is a warning, not an error, since the tool may come
and go.

**Sources and precedence**, lowest first. A role of a higher source with the same
name **replaces** the lower one whole.

This differs from Codex. Codex fills a higher layer's missing description and config
file from the lower one (`merge_missing_role_fields`, `codex-rs/agent-roles/src/loader.rs`).
Here, a role that wants to build on another names it with `extends`, so there is one
merge rule rather than two.

1. **Built-in roles**, compiled into the facade: `default` (no narrowing; the role a
   session has without one) and `explorer` (read-only tools, `spawnable`). Decision
   D4.
2. **Host roles** (`AgentHostPorts::roles`): **not in this plan** (D5). The slot is
   reserved in the order, and is added with Hanafubuki's first role.
3. **Package roles** (M6), named `<plugin>:<role>`, so they never collide with
   user roles.
4. The **user's** `agents/` directory and `[agents]` in the user's `config.toml`.
5. The **profile's** `[agents]` (`-p NAME`).
6. The **trusted project's** `.hanafubuki-agent/agents/` and its `[agents]`.

Within one source, two roles with the same name: the first by path keeps it, and the
other is listed as a duplicate. This is Codex's rule
(`discover_agent_roles_in_dir`).

**Inheritance** (`extends`). Single parent, resolved by name across all sources.

- At most 8 levels deep.
- A cycle or a missing parent makes the role unusable, with why. It never falls back
  silently; KohakuTerrarium warns and continues for grandparents, which is the
  weakness to avoid.

The merge is defined per field, and every rule keeps narrowing monotonic:

| Field | Rule |
| --- | --- |
| `description`, `model`, `reasoningEffort`, `spawnable` | The child's if set, else the parent's. |
| `instructions` | `append`: the parent's, a blank line, then the child's. `replace`: the child's only. |
| `permissionModeCap` | The stricter of the two (Manual > AutoReview > FullAccess). |
| `tools.allow` | The child's effective allow set is the parent's effective allow set **intersected** with the child's own. An absent allow means "everything the parent allows". |
| `tools.deny`, `plugins.off` | Union. |

There is therefore no "remove one inherited tool" problem (KohakuTerrarium's
`no_inherit` removes all or nothing): `deny` removes any one.

**Resolution** happens in the facade (`crates/agent/src/roles/`), whenever the
settings are published (load, a write, `config reload`) and whenever packages
change. It produces:

```rust
pub struct AgentRoleCatalog { roles: BTreeMap<String, AgentResolvedRole>, issues: Vec<AgentRoleIssue> }
pub struct AgentResolvedRole {
    pub name: String,
    pub description: String,
    pub source: AgentRoleSource,              // built-in | host | package(id) | file(path) | layer(name)
    pub chain: Vec<String>,                   // itself, then each parent
    pub instructions: Option<String>,         // already merged along the chain
    pub instructions_mode: InstructionsMode,
    pub model: Option<String>,
    pub reasoning_effort: Option<ModelReasoningEffort>,
    pub permission_mode_cap: Option<AgentPermissionMode>,
    pub spawnable: bool,
    pub scope: AgentCapabilityScope,
    pub provenance: BTreeMap<&'static str, AgentRoleSource>,   // per field, for `roles show`
}
```

**Where each part lives.**

- Role file parsing: the config crate (`crates/config/src/roles/`), as user
  configuration.
- The Claude Code `agents/*.md` reader: the packages crate (M6), as a component
  reader beside `commands`.
- Merging and resolution: the facade.
- No new crate.

**Surfaces.**

- `hanafubuki-agent roles list` and `roles show NAME`. `show` prints each effective
  field with its provenance; KohakuTerrarium's `compute_effective` stops at the
  first hop, which is the weakness to avoid.
- `doctor` reports role issues.
- `/warnings` lists them.

### 3.4 The session capability scope (M4)

**The type.** A kernel model type (`crates/kernel/src/model/scope.rs`), persisted
inside `AgentSessionConfig`:

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCapabilityScope {
    /// The role it came from, shown in snapshots and `/status`; not a reference:
    /// the scope is self-contained, so a role file edited or deleted later never
    /// changes a session that already has it.
    pub role: Option<String>,
    pub tools: AgentToolScope,               // allow: Option<Vec<ToolPattern>>, deny: Vec<ToolPattern>
    pub plugins_off: BTreeSet<String>,
    pub permission_mode_cap: Option<AgentPermissionMode>,
}
```

**Normalization.** `normalize_session_config`
(`crates/kernel/src/service/validation/config.rs:309`) canonicalizes it:

- sorted and deduplicated;
- at most 128 patterns of 256 bytes each;
- core row names removed from `plugins_off`.

It runs at creation, on every update and at recovery, as it already does for the
rest.

**Decoding.** The scope is read tolerantly with `#[serde(default)]` and field-level
fallback, like `tolerant_compaction_strategy`. A history written before the field
existed opens with the empty scope, which is today's behaviour.

**Where the kernel applies it.** Each point is one filter where a snapshot is
already taken:

| Point | Where | Rule |
| --- | --- | --- |
| Tool plan | `AgentToolPlanner::build`, after mode policy; `AgentToolCatalogContext` gains `scope: AgentCapabilityScope` | A registered or sourced tool is planned only if allowed and not denied. **Kernel core tools (`RESERVED_TOOL_NAMES`) and `tool_search` are never filtered.** A filtered tool is absent: a call to it gets "no tool named `<name>` is available in this step", it is not in `tool_search`'s index, and it is **not** reported in `tool_catalog_omissions` (it was left out on purpose, not lost). Deferred-tool selections of a filtered tool are ignored. |
| Tool sources | the same planner | A source whose owner is in `plugins_off` is not asked at all. |
| Context contributors, turn admission | `collect_world_state`, `collect_turn_extensions` | Entries whose owner is in `plugins_off` are skipped, and their kinds are removed from that session's world state, as for a stopped plugin. |
| Prompt contributors | the step's `prepare` / `contribute` | Skipped by owner. |
| Continuation hooks | the step's snapshot | Skipped by owner. |
| Permission mode | when the turn freezes its mode | The effective mode is the stricter of the session's (or the setting's) and the cap. `AllowForSession` is unaffected; it never widens past a rule anyway. |

**Never filtered**, whatever a scope says:

- **approval rules**: removing a rule would widen approval;
- **tool lifecycle hooks**: the user's PreToolUse/PostToolUse policy, deny-only;
- **observers**: instance services such as memory and titles, not model-facing;
- **renderers**: data.

These are invariants of the plugin rules page, with tests.

**Owners.** The filter matches the registry entry's owner (`RegistryEntry` knows it),
so a guest's contributions are `guest:<id>` and `plugins_off = ["guest:demo@mkt"]`
works. A `plugins_off` entry naming no plugin is kept and means nothing, as in
`disabledPlugins`.

**Effects on the two plugins whose values the kernel reads.** These follow from
existing rules, and are documented and tested:

- `skills` off: the turn has no `SelectableSkills`, so an explicit Skill reference
  or `$mention` is refused with "the Skill catalog is unavailable" (plugin rules,
  `#turn-admission`), and the Skill tools are absent.
- `memory` off: the memory summary and memory tools are absent, and the step has no
  `AgentMemoryStep` (answers shown as they are, no citations). The session still
  **contributes** to memory according to its contribution mode, because observers
  are never filtered. To stop the contribution, set the session's memory
  contribution mode; a role does not do that.

**Lifecycle of a scope.**

1. **Creation.** The facade's `create_session` (and `AgentRuntime`'s) takes
   `role: Option<String>`. It resolves the role from the current `AgentRoleCatalog`
   and applies:
   - `instructions`. The session's instructions are always the Agent's base
     instructions (`crates/agent/src/prompts/base.md`, the host's additions,
     `closing.md`), then the role chain's merged instructions. `instructionsMode = "replace"`
     replaces **only the ancestors' role instructions**. No role can replace or remove
     the base instructions, which carry the approval guidance.
   - `model` and `reasoningEffort` (through the existing model selection, so the
     context window is resolved as usual);
   - the scope.

   An unknown or unusable role is an `InvalidInput` error naming the usable roles.
   Without `role`, the session gets `default`, the empty scope.
2. **Switching.** `set_session_role(session, role)` is allowed only while the session
   is idle, like `set_session_compaction_strategy`, so no plan of a running step
   changes. It commits a session configuration update durably, then emits the
   snapshot event. The TUI's `/role NAME` and the configuration center use it.
   Switching back to `default` clears the scope.
3. **Children.** `create_child_session` (`crates/kernel/src/service/session/child.rs:29`)
   gives the child `parent.scope.narrow(&child_scope)`:
   - allow sets intersected;
   - deny and `plugins_off` united;
   - the stricter cap.

   A child is never wider than its parent. Forks keep the source's scope.
4. **Recovery.** The scope is in the persisted configuration, so a restored session
   keeps it without re-reading any role file.
5. **Editing a role** affects new sessions, and existing ones only when the user
   re-selects the role. This is predictable, and avoids a session's tools changing
   under it between two turns.

**Snapshots and events.** The session snapshot carries `role` and a compact scope
summary: counts, plus `plugins_off`. The `exec --json` session object gains `role`,
which is an additive event field ([exec --json events](../site/content/docs/json-events.en.mdx)
stability rule).

**Plan mode and review.** Plan mode still keeps only `ReadOnly` tools, after the
scope filter. The `review` plugin's approval rule stays. Giving review sessions a
built-in `review` role is not part of this plan.

### 3.5 Role-aware subagents (M5)

**Tool argument.** `spawn_agent` gains an optional `agent_type` (Codex's name, which
models are trained on) whose value is a spawnable role name.

- A missing `agent_type` is `default`, the parent's scope only.
- An unknown or non-spawnable role is a field-named tool error listing the
  spawnable ones.

**The child session.** The subagents plugin creates it through `AgentSessionDriver`:

- instructions: the base instructions plus the role's (§3.3);
- model: the role's `model` if set, else the parent's (Codex's "inherit");
- scope: the parent's narrowed by the role's;
- origin `Subagent`, title the task name, as today.

The plugin reaches the role catalog through a narrow port the application constructs
(`AgentRoleCatalogReader`, beside `AgentSessionDriver`), resolved per call. It never
holds the catalog.

**What the model sees.** The orchestrator guidance (`subagent_guidance`) lists the
spawnable roles:

- name and description;
- at most 16 roles, each description cut at 200 characters on a character boundary;
- sorted by name, so the request is stable.

The `spawn_agent` schema adds `agent_type` as a string with that list in its
description. This is model-visible, so the request goldens are regenerated on
purpose. Hanafubuki's goldens change too.

**Limits.** The `maximumRunningAgents` option (M1) bounds the running children, as
the constant does today.

### 3.6 Roles from packages (M6)

- **Our format** (`.hanafubuki-plugin/plugin.json`): an `agents` component path, by
  default `agents/`, holding role TOML files. They are read when the package is
  loaded.
- **Claude Code packages** (`.claude-plugin`): `agents/*.md`. The YAML frontmatter has
  `name`, `description`, `tools` and `model`; the body is the instructions.
  - `tools` is mapped through a fixed table of Claude tool names to ours:

    | Claude | Ours |
    | --- | --- |
    | `Read` | `read_file` |
    | `Grep` | `search_text` |
    | `Glob`, `LS` | `list_directory` |
    | `Edit`, `MultiEdit`, `Write` | `apply_patch` |
    | `Bash` | `exec_command` |
    | `WebFetch` | `fetch_web_page` |
    | `WebSearch` | `web_search` |
    | `mcp__<server>__<tool>` | the flattened name, unchanged |

    A name the table does not know is left out with a package issue. An absent
    `tools` means no narrowing.
  - `model: inherit` is honoured. Any other value is ignored with an issue, since
    Claude's aliases do not name this Agent's models.
  - Follow-up (2026-11-11): `disallowedTools` maps through the same table to the
    role's deny list (a specifier denies the whole tool; `mcp__<server>` denies
    that server's tools), and `permissionMode` maps to the cap (`plan` also denies
    Plan mode's write tools, and every MCP tool unless `tools` lists it). See
    writing-plugins `#claude-agents`.
- **Codex packages** have no roles; nothing to read.
- **Identity and lifetime.** A package role is `<plugin>:<role>`. It exists while the
  package is enabled. Two enabled packages from different marketplaces with the
  same plugin name: the first by package id keeps the name, and the other is listed
  as a duplicate.
- **Safety.** A package role can only narrow (§3, invariant 1). A package role cannot
  have `spawnable = false` overridden by a user role of another name; a user role
  that `extends` it can change it.
- **Documentation.** `docs/plan-plugin-system.md` §3.3 and §10 list Claude's
  `agents/` as not ported. When M6 lands, update those lines to point here.

### 3.7 Linked development packages (M7; decision D6)

KohakuTerrarium's editable installs (`<name>.link`) shorten the plugin author's loop.
The equivalent here:

- `hanafubuki-agent plugins link <dir>` records a package **by reference**:
  - `install.json` source `linked`;
  - no copy into the cache, no content hash pin.
- Linked packages are listed as such and are never auto-updated.
- The guest host watches the linked directory, debounced at 500 ms like the Skill
  watcher, and restarts the guest. kardia-loader's `registry().replace` is already
  the upgrade path.
- Content components (Skills, roles, hooks, MCP) are re-read on the same signal.
- A linked package is trusted exactly as an installed one: the same permission
  grants and the same sandbox profile.

## 4. Module lifecycle, end to end

The lifecycles below are additive. Nothing about the existing ones changes, except
M2's remount ordering.

| Module | Created | Configured | Changes while running | Removed | Failure |
| --- | --- | --- | --- | --- | --- |
| Row | instance start (manifest, unchanged) | `from_settings`: shared settings or `pluginOptions` (M1) | settings publication, then `reconfigure`: `on_update`, or a restart that drains leases | `disabledPlugins`, safe mode, shutdown | fault, then `RestartPolicy`; start failure: `Failed` until retry or a new config |
| Guest | enabled package with code | `[plugins."<id>"].settings`, validated against the lowered `configSchema` (M1) | `fiber.update`; linked source changed: restart (M7) | package disabled or uninstalled | as today |
| Role definition | settings or package publication, then `AgentRoleCatalog` rebuilt | its file / table | the catalog is rebuilt; sessions unaffected until re-selected | file deleted or package disabled: gone from the catalog, sessions keep their scope | unusable with why; never fails the Agent or a session |
| Session scope | `create_session(role)`, `create_child_session` (narrowed), fork (copied) | resolved role, then `normalize_session_config` | `set_session_role` between turns, durable | session deleted | invalid parts dropped at normalization |
| Per-step view | each sampling step's snapshots | scope filter applied at the existing snapshot points | none within a step (frozen plan) | step end | unchanged guards and bounds |

**Order of resolution at instance start.**

1. Settings load: plugin options decoded.
2. Packages read: package roles and lowered config schemas.
3. `AgentRoleCatalog` resolved.
4. Plugins started.
5. Sessions recovered, each with its persisted scope; no role lookup.

## 5. Extensibility: who extends what, and how

| Author | Adds | Mechanism | New in this plan |
| --- | --- | --- | --- |
| This repository | a capability | a row: plugin crate or facade plugin, plus a manifest row | a descriptor; options with a schema (M1); an explicit renderer `order` (M2) |
| A host (Hanafubuki) | host capabilities | `AgentHostPorts::plugins` rows | descriptors (D2); renderer `order` in the host band (M2); host roles later (D5) |
| A package author (code) | tools, sections, prompt, hooks, observers | guest `contributes` | `configSchema` validated, with a generated form (M1); roles (M6); linked dev loop (M7) |
| A package author (content) | Skills, MCP servers, hooks | content components | roles: ours or Claude Code's `agents/*.md` (M6) |
| The user | definitions and choices | `config.toml` layers, `agents/*.toml` | `[pluginOptions]` (M1); roles and `[agents]` (M3); `--role`, `/role` (M4); `spawn_agent` roles (M5) |
| A project (trusted) | project roles and options | `.hanafubuki-agent/` | `agents/` and `[agents]`, `[pluginOptions]` |

What stays closed on purpose:

- no transform hooks;
- no approval rules or turn admission from guests (plugin plan decision 5);
- no per-session widening;
- no dynamic rows.

## 6. Customization capabilities after the plan

| Want | Today | After |
| --- | --- | --- |
| Tune a built-in's limits | edit code | `[pluginOptions.<row>]`, or the generated form |
| Configure a guest | hand-edit `[plugins."<id>"].settings`, unchecked | a validated form with defaults and issues |
| A read-only research assistant | Plan mode, or hope | `--role explorer`, or a custom role with `tools.allow` |
| Subagents with different jobs | all equal to the parent | `spawn_agent { agent_type }` over spawnable roles |
| Share an agent definition | not possible | a role file in a package, or a Claude Code `agents/*.md` |
| Turn a plugin off for one kind of session | not possible (instance-wide only) | `plugins.off` in a role |
| Stable prompt cache across plugin restarts | broken by remount order | explicit renderer order |

## 7. Considered and not adopted

Each of these is KohakuTerrarium's, and each conflicts with a rule this repository
already holds.

- **Transform hook chains.** `pre_llm_call` / `post_llm_call` message rewriting,
  `pre_tool_execute` argument rewriting, `pre_tool_dispatch` tool renaming. They
  conflict with:
  - "the model API is the wire and nothing more";
  - the goldens, since what the model sees would depend on plugin code;
  - the deny-only combinations;
  - `invoke`'s approval re-check, since an argument rewritten after approval would
    run unapproved.
  Tool lifecycle hooks (deny, add context) cover the safe subset.
- **Fail-open hooks.** KohakuTerrarium logs and continues on any non-block exception,
  so a buggy policy plugin lets calls through. The Agent's rules fail closed and its
  user hooks have no say. Both are deliberate.
- **A context object with the whole agent** (`PluginContext.host_agent`,
  `controller._pending_injections`). Contracts-only access and narrow ports stay.
- **Priority-ordered linear chains as the general combination.** The Agent's
  per-point combinations are stricter and independent of registration order. Only
  the renderer order is adopted (M2).
- **Instantiating every discovered plugin at startup, disabled.** Rows are compiled
  in and packages are lazily mounted guests; nothing to gain.
- **Cooperative sandboxing** (`path_arguments`, a runner a tool may ignore). The
  kernel pipeline and the OS sandbox stay.
- **Dynamic built-in rows in `ModuleHost`.** Plugin plan decision 3 stands: the guest
  subtree covers runtime add and remove, and changing the host twin costs Hanafubuki.
- **Triggers, input and output modules, channels, terrariums, output wiring, the
  `compose` algebra.** These are ambient, unattended turns and multi-agent graphs.
  They need an approval model for turns nobody started, and a multi-session
  scheduler. `AgentSessionDriver` plus subagents covers today's delegation. Revisit
  with a real user.
- **Jinja templating of instructions and `${VAR}` interpolation.**
  KohakuTerrarium's templates render only under a condition and fall back to raw
  text silently. Roles concatenate plain text; nothing needs variables yet.
- **Per-session plugin enablement that turns on what the instance has off.** It
  widens. Only `plugins.off` exists.
- **Plugin-contributed slash commands and UI widgets, LLM preset and variation
  groups, Python-dependency installation, Studio code generation.** No user here
  yet; the Provider model policy covers model choice.
- **Background tool mode and mid-stream dispatch.** Architecture, "No streaming
  dispatch", still holds.

## 8. Milestones

Each milestone is shippable on its own, validated as `AGENTS.md`'s Validation
section says, documented in both languages with matching heading ids, and committed
separately. Order: M1 and M2 are independent; M3 then M4 then M5; M6 after M3; M7
after M1.

### M1: typed plugin options and row descriptors

**Work.**

- `crates/config/src/plugin_options/`:
  - the `PluginOptionSchema` vocabulary;
  - the tolerant decoder;
  - the `[pluginOptions]` table, layered per row and key;
  - `AgentConfig::set_plugin_options`;
  - `config get/set pluginOptions.<row>.<key>`.
- Lower a guest's `configSchema` in the packages crate. `check.rs` uses the lowered
  form, and guest settings are validated against it before `apply`.
- `crates/agent/src/host/descriptors.rs`: a descriptor per row. `AgentPluginSettings`
  carries the resolved options. Add the `subagents` options, with `on_update` in its
  plugin.
- `AgentPluginStatus` gains `summary` and `options`; `AgentPackageGuest` gains the
  lowered schema and values.
- TUI: the generic options form, reachable from `/plugins` and `/config plugins`.
  Extend the coverage test.

**Tests.**

- Decoder: each wrong type falls back with an issue; out-of-range values clamp.
- Layering per key.
- Strict writes.
- A row restarts, or absorbs the change, on an option write.
- A guest receives validated settings.
- The descriptor-per-row test.

**Model-visible:** no. **Goldens:** unchanged. **Twin:** none. `SettingsConfig` is
reused through a non-capturing closure; confirm that `hanafubuki-agent-host` has no
diff.

**Documents:** config-reference (`pluginOptions`), configuration, plugin-rules
("Host, configuration and status", "Writing a plugin" checklist: descriptor and
options), terminal-interface (`/plugins` options), writing-plugins (`configSchema` is
now enforced, and the supported keywords).

**Hanafubuki at its next pin:** its host rows supply descriptors (D2).

### M2: explicit context order

**Work.**

- `ContextRenderer.order`.
- Ordering by `(slot, order, kind)` in request assembly
  (`crates/kernel/src/service/sampling/request.rs`, and `prompt.rs` for fragments).
- Ranks for every built-in renderer, the kernel's in `kernel_context_renderers`
  (`crates/kernel/src/model/world_state.rs:26`).
- Guests: the 900 band.

**Tests.**

- All request goldens unchanged.
- A new test remounts a context plugin and gets a byte-identical request.
- Ties are broken by kind.

**Model-visible:** only after a remount. **Goldens:** must not change.
**Twin:** none (contracts is not a twin).

**Documents:** plugin-rules `#context-renderers`, architecture
`#sections-slots-and-renderers`, host-plugins (choose an order in the host band).

**Hanafubuki at its next pin:** every host renderer sets `order` in 500–599; its
goldens must not change.

### M3: roles: definitions, sources, resolution

**Work.**

- `crates/config/src/roles/`: the TOML reader (ours plus Codex aliases), `[agents]`
  in every layer, and discovery of the `agents/` directories (the user's, and a
  trusted project's).
- `crates/agent/src/roles/`:
  - sources and precedence;
  - inheritance and the merge table;
  - provenance;
  - `AgentRoleCatalog`, rebuilt on settings and package publication;
  - the built-in `default` and `explorer`.
- CLI `roles list` / `roles show`; `doctor` and `/warnings` issues.
- **No session uses roles yet.**

**Tests.**

- The merge table, field by field.
- Narrowing monotonicity: a child can never allow what its parent denies (a property
  test over random chains).
- Cycles and missing parents.
- Duplicates within a source; precedence across sources.
- Tolerant keys; Codex aliases.

**Model-visible:** no. **Twin:** none.

**Documents:** a new page `roles.mdx` / `roles.en.mdx` (Configuration group; add the
slug to `meta.json` and `meta.en.json` in the same position), config-reference
(`[agents]`), config-layers (roles per layer), cli-reference (`roles`).

**Hanafubuki:** none.

### M4: the session capability scope

**Work.**

- `AgentCapabilityScope` in the kernel model, inside `AgentSessionConfig`, with
  tolerant decoding and `normalize_session_config` canonicalization.
- The filter at each point in the §3.4 table:
  - `AgentToolCatalogContext.scope` and the planner;
  - context, turn admission, prompt and continuation snapshots by owner;
  - the permission-mode cap at the turn's freeze.
- Children narrowed in `create_child_session`; forks copy.
- Facade:
  - `create_session(role)` and `set_session_role`;
  - `AgentRuntime` and the host API take `role`;
  - CLI `--role` for interactive mode and `exec`;
  - TUI `/role`, plus `role` in `/status` and the status line.

**Tests.**

- A filtered tool is "no tool named", identically to an absent one.
- Kernel core tools and `tool_search` survive any scope.
- Approval rules and tool lifecycle hooks still apply under `plugins_off` naming
  their plugin.
- A deferred selection of a filtered tool is ignored.
- A child is never wider than its parent.
- Recovery keeps the scope after the role file is deleted.
- An old history opens with the empty scope.
- `set_session_role` is refused while a turn runs.

**Model-visible:** only for sessions with a non-default role. **Goldens:** the
default requests are unchanged. Add one golden for an `explorer` session.

**History format:** a new tolerated field. Re-record the corpora
(`HANAFUBUKI_AGENT_UPDATE_GOLDEN_HISTORY=1`) only if the persisted bytes of a default
session change. Prefer `skip_serializing_if = "is_default"` so they do not; the
existing corpora then keep proving that older history opens.

**Twin:** the history corpus is shared bytes with Hanafubuki; with
`skip_serializing_if` it stays identical.

**Documents:**

- plugin-rules: a new section, "Capability scope", with the invariants as rules;
  plus the never-filtered list.
- architecture: "Sessions and turns", "A plan per step".
- sessions, cli-reference, exec (`--role`), json-events (`role`),
  terminal-interface (`/role`), embedding (`role` on session creation).

**Hanafubuki at its next pin:** its session-creation calls compile with `role: None`.
Optionally it can expose role choice in its UI.

### M5: role-aware subagents

**Work.**

- `spawn_agent { agent_type }`.
- The `AgentRoleCatalogReader` port.
- Children created with the role's instructions, model and narrowed scope.
- The guidance lists spawnable roles (bounded, sorted).
- `maximumRunningAgents` from M1 if landed, else the constant.
- `list_agents` reports each agent's role.

**Tests.**

- An unknown or non-spawnable type is a field-named error listing the valid ones.
- A child's tools are the intersection.
- The model falls back to the parent's.
- The guidance is truncated and stable.

**Model-visible:** yes. Regenerate the request goldens on purpose
(`HANAFUBUKI_AGENT_UPDATE_GOLDEN=1 cargo test -p hanafubuki-agent --lib golden`) and
review the fixture diff.

**Documents:** plugin-rules ("The plugins today", `subagents` row), architecture
("Subagents"), the subagents section of sessions. Remove the `agent_type` note in
`crates/agent/src/plugins/subagents/tools.rs:9` (the code comment) when the argument
returns.

**Hanafubuki at its next pin:** its integration goldens change; regenerate and
review them.

### M6: roles from packages

**Work.**

- The `agents` component in our manifest format.
- The Claude Code `agents/*.md` reader with the tool-name table (packages crate). It
  needs no new dependency: `serde_yaml` is already a dependency of
  `crates/packages`, for frontmatter.
- Package roles enter the catalog as `<plugin>:<role>` while enabled.
- `AgentPackageStatus` lists them.
- `/plugins` shows a package's roles.

**Tests.**

- Frontmatter tolerance: a wrong type drops that key.
- Unknown Claude tools are left out with issues.
- `model: inherit` is honoured and other models are ignored.
- Enable and disable add and remove the roles.
- Duplicate plugin names across marketplaces.

**Model-visible:** only through M5's guidance when a spawnable package role exists.
The goldens have no packages, so they are unchanged.

**Documents:** architecture "Installed plugins", writing-plugins (the `agents`
component), roles page (package roles). Update `docs/plan-plugin-system.md` §3.3 and
§10.

**Hanafubuki:** none.

### M7 (D6): linked development packages

**Work.**

- `plugins link <dir>`: `install.json` source `linked`.
- A watcher in the guest host, built as a `BackgroundWorkerGroup::contained` worker,
  never a plain thread.
- Guest restart via kardia-loader `replace`; content re-read.

**Tests.**

- A change restarts the guest within the debounce.
- Linked packages are never auto-updated.
- Unlinking keeps the source directory.

**Documents:** writing-plugins (development loop), cli-reference (`plugins link`),
internals ("Plugin guests").

### Migration and compatibility

Nothing is renamed or removed, so there is no migration step for users. Every change
is additive and tolerant.

| Area | Change | Existing data / users | Older build reading new data |
| --- | --- | --- | --- |
| `config.toml` | new `[pluginOptions]`, `[agents]` tables (M1, M3) | absent tables: today's behaviour | unknown tables ignored (tolerant settings) |
| User/project files | new `agents/*.toml` directories (M3) | none exist: only built-in roles | not read |
| `[plugins."<id>"].settings` | now validated against the guest's `configSchema` (M1) | a value that does not fit falls back to its default, **with a warning**; nothing is rewritten | — |
| Session config in history | new `scope` field, `skip_serializing_if` default (M4) | old sessions open with the empty scope (`default` role); default sessions' bytes unchanged | a session with a scope opens, and the field is ignored: the older build runs it unscoped. Acceptable, since downgrading is not a supported security boundary; say so in the release notes |
| `install.json` | new source kind `linked` (M7) | unchanged | an unknown source reads as none, the entry's other fields kept (existing tolerant reader). As built, a linked package has no cache directory, so an older build does not list it at all (not "listed with an issue"), and that build's next write of `install.json` keeps the entry with `"source": null`; this build then no longer lists it either, until it is linked again. The directory is never touched |
| `ContextRenderer` (contracts) | new `order` field (M2) | built-in ranks reproduce the current order | — (compile-time API; host code must set it) |
| `spawn_agent` spec | new optional `agent_type` (M5) | calls without it behave as today | — |
| `exec --json` | new `role` field on the session object (M4) | additive under the stability rule | — |
| Host API | `create_session` gains `role`, `AgentHostPorts::plugins` gains descriptors (D2) | compile-time changes for Hanafubuki | — |

**Goldens and corpora per milestone.**

| Milestone | Request goldens | History corpora |
| --- | --- | --- |
| M1 | unchanged | unchanged |
| M2 | unchanged; that is its acceptance test | unchanged |
| M3 | unchanged | unchanged |
| M4 | unchanged, plus one new explorer golden | unchanged, thanks to `skip_serializing_if` |
| M5 | regenerated on purpose | unchanged |
| M6 | unchanged | unchanged |
| M7 | unchanged | unchanged |

**Hanafubuki, in pin order.**

1. M1: supply a descriptor with each host row (D2).
2. M2: set `order` in 500–599 on each host renderer, and check that its goldens are
   unchanged.
3. M4: pass `role: None` where it creates sessions.
4. M5: regenerate its integration goldens and review them.

No milestone changes a twin module (`hanafubuki-agent-foundation`, `-process`,
`-host`), and the shared history corpus keeps its bytes.

### Per-milestone checks

- **Model-visible changes regenerate goldens on purpose.** Only M5, plus M4's new
  explorer golden.
- **No `deny_unknown_fields`** is introduced, so `architecture/strict-decoding-policy.toml`
  is unchanged. Role files, options and Claude frontmatter are tolerant.
- **Module size.** New modules stay under 600 code lines. If
  `crates/config/src/settings/mod.rs` or the planner grows past its limit, report it;
  do not add an exception silently.
- **Panics.** The new code in `crates/agent/src/plugins` (subagents) is under
  `check-plugin-panics`, so it has no `unwrap` or `expect`.
- **Locks.** Any new shared state (the role catalog behind an `ArcSwap` or a
  `parking_lot::RwLock`) follows the lock rule. Compute, then assign.
- **Paths.** Role file and linked package paths go through `foundation::paths`
  (`ResolvedPath`): `native()` for messages and `key()` for duplicates.

## 9. Decisions

Each decision has a recommendation. **All were accepted as recommended
(2026-11-11)**; the alternatives are kept for the record.

1. **D1: adopt the narrowing-only invariant for every definition.** *Recommended:*
   yes. Roles, inheritance, children and package roles can only remove and tighten.
   *Alternative:* KohakuTerrarium-style widening roles (a role may enable a plugin
   the instance has off, or raise the permission mode). This makes package roles a
   privilege-escalation path and needs a grant model.
2. **D2: descriptors for host rows.** *Recommended:* `AgentHostPorts::plugins`
   carries a descriptor with each row, so Hanafubuki's rows show a summary and can
   have options. *Alternative:* optional descriptors, with a host row shown by name
   only.
3. **D3: move `web`'s settings into `[pluginOptions.web]`.** *Recommended:* no. They
   are shared with the network form and the CLI, and a clean move would rename
   user-visible keys for no capability gain. *Alternative:* move them, as the
   repository's "clean breaking change" preference would allow.
4. **D4: built-in roles.** Codex ships `default`, `explorer` and `worker`
   (`codex-rs/core/src/agent/role.rs`, `mod built_in`), with descriptions written as
   spawn guidance.
   - *Recommended:* `default` and `explorer`, with Codex's description adapted.
     `explorer` is read-only: `list_directory`, `search_text`, `read_file`,
     `view_image`, `web_search`, `fetch_web_page`; it is `spawnable`.
   - *Alternative:* also `worker`, which here would be `default` with Codex's
     ownership guidance as its description, since it narrows nothing.
   - *Alternative:* `default` only, leaving every role to the user and to packages.
5. **D5: host-provided roles** (`AgentHostPorts::roles`). *Recommended:* not now. Add
   it with Hanafubuki's first role. *Alternative:* add the port in M3.
6. **D6: linked development packages (M7).** *Recommended:* yes, after M1. It is the
   one KohakuTerrarium convenience plugin authors miss most, and it reuses the guest
   restart path. *Alternative:* leave plugin development to reinstalling.
7. **D7: the model-visible name of the subagent role argument.** *Recommended:*
   `agent_type`, Codex's, which models know. *Alternative:* `role`, which matches
   this plan's term.
8. **D8: role switching mid-session.** *Recommended:* allowed between turns
   (`set_session_role`), durable. *Alternative:* a role is fixed at creation (fork to
   change it), which is simpler and keeps one session one definition.
9. **D9: options table name.** *Recommended:* `[pluginOptions.<row>]`, separate from
   `[plugins."<id>"]`. Packages and rows are named differently and are enabled
   differently (plugin plan decision 7). *Alternative:* `[plugins."<row>"].options`
   for rows as well, one table for both, which mixes the package-id namespace with
   row names.

## 10. Not built (in this plan)

- Ambient triggers, inputs and outputs, channels, multi-agent graphs.
- Transform hooks of any kind.
- Plugin-contributed slash commands or UI widgets.
- Instruction templating or variables.
- Per-session options (options are instance-wide; a role only narrows).
- A `review` role replacing the review plugin's approval rule.
- Model presets and variation groups beyond the existing Provider model policy.
- Host-provided roles (`AgentHostPorts::roles`, D5), until Hanafubuki has a first role.
