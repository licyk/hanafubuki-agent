# Plan: a configuration center for every Agent setting

`/config` already exists, but it edits only five values and links to the
other editors. This plan turns it into the terminal interface's
configuration center: one searchable screen over every setting the Agent
documents as user-configurable, backed by the typed configuration model and
the existing atomic writers. It does not add a raw TOML editor or a second
configuration parser.

## 1. What exists today

- `/config` is registered in `crates/tui/src/commands/mod.rs`. Its panel,
  `crates/tui/src/commands/config.rs`, edits `tui.rawOutput`,
  `tui.statusLineColors`, `tui.fullscreenTranscript`,
  `tui.alternateScreen` and `compaction.strategy`, then opens `/providers`,
  `/mcp`, `/memories`, `/theme`, `/statusline`, `/title`, `/keymap`,
  `/presets` and `/tui`.
- `hanafubuki-agent-config` already has typed settings, layered reads,
  tolerant recovery, in-place writes and atomic publication:
  `AgentSettings`, `AgentSettingsPatch`, `AgentStandaloneSettings`,
  `AgentTuiPreferencesPatch`, `AgentConfig::write_settings` and
  `AgentConfig::write_tui_preferences`.
- The facade exposes a full `AgentSettingsView` and `write_settings`.
  Writes validate the resulting settings, reconfigure plugins and signal a
  memory scan. The terminal interface additionally owns
  `AgentTuiPreferencesPatch` and holds the standalone `AgentRuntime`, so it
  can read and write standalone-only settings without widening the embedded
  Agent API.
- Provider and MCP configuration already have revision-gated editors. The
  Provider editor covers a Provider's connection fields and the default
  model, but not the whole shared model policy. The MCP editor covers most
  of a server but not its protocol mode or per-tool policy maps.
- Plugins, installed packages, marketplaces, hooks, themes, status lines,
  terminal titles, keymaps and presets already have dedicated interfaces.
  The configuration center should be their common entry point, not a copy
  of them.

## 2. Scope

“All settings” means the known, user-configurable keys documented in the
configuration reference, plus the editable records in `providers.toml` and
`mcp.toml`. Internal bookkeeping is displayed read-only or omitted:
registry revisions, generated Provider and MCP ids, and credential
generations are never hand-edited. Unknown keys remain preserved by the
configuration writer but are not exposed as settings.

### 2.1 `config.toml`

- General: `permissionMode`, `disabledPlugins` through the plugin manager,
  and `logLevel`.
- Sandbox: `sandboxMode`, `sandboxWorkspaceWrite.writableRoots`,
  `sandboxWorkspaceWrite.networkAccess` and `windows.sandbox`.
- Memory: all eleven `memory.*` fields.
- Web tools: `webSearchProvider`, `searxngBaseUrl` and
  `webFetchUseJinaReader`; the SearXNG provider and endpoint are saved as
  one validated group.
- Compaction: `compaction.strategy`, both background fields and all
  fifteen hierarchical fields.
- Standalone network and logging: `network.proxy` and `logLevel`.
- Terminal interface: the four scalar preferences in a form, with the
  theme, status line, terminal title, keymap and preset editors opened as
  dedicated pages.
- Plugins, packages and marketplaces: summaries and entry into `/plugins`;
  lifecycle calls keep owning their writes.
- Hooks: an entry into `/hooks`. Hook definitions are not `config.toml`
  settings and do not gain a creator here.
- Layers: user, profile, trusted project, `-c` and flag layers are shown;
  writes continue to change only the user's file.

### 2.2 `providers.toml`

The Provider record editor keeps its current fields. A new model-policy
stage edits the default Provider and model, reasoning effort, image input,
context window, automatic compaction, remote compaction and the complete
optional turn budget. Saves use `AgentModelPolicyWriteRequest` with the
registry revision the screen read.

### 2.3 `mcp.toml`

The server form gains `protocolMode`,
`toolMaximumConcurrentCalls`, `toolApprovalPolicies` and
`toolExposurePolicies`. Existing transport, credential, timeout, tool-list
and global policy fields remain. Credential values can be replaced through
secret fields; their generation stays managed by the MCP store.

## 3. Interface design

`/config` opens a full-screen configuration center, like `/agents`. The
first page is searchable and lists one row per configuration domain with a
short current value. Its header names the active layers and warning count.
`Enter` opens a domain; `Esc` goes back or closes; `/` focuses search.

A domain is a `FormView`: text, secret text, choices, toggles, numbers,
nullable text, single-column lists and name/value rows. Rarely used values
stay under “Advanced”. A domain is saved as a whole: local edits are
drafted into a typed patch, checked, written atomically, and only then
replace the screen's state. Invalid input stays in the form with field
errors.

`/config` accepts an optional section name for direct entry and tests:
`general`, `sandbox`, `memory`, `web`, `compaction`, `network`,
`interface`, `providers`, `mcp`, `plugins`, `hooks` and `layers`.

The overview displays effective values and defaults. After a save, if the
resolved value differs from the value written to the user's file, the form
says that a profile, project or override above it applies. Providers and
MCP identify themselves as unlayered owner-only files.

## 4. Backend design

- Add a terminal-interface-only `ConfigState`: the facade's
  `AgentSettingsView`, the standalone settings, layer sources and
  configuration warnings. The Agent facade itself does not gain a
  standalone-settings write API.
- Replace the `/config` reply that carries only the compaction strategy
  with one carrying `ConfigState`.
- Add `SettingsCall::SetAgentSettings(AgentSettingsPatch)` and migrate the
  compaction and memory saves onto it.
- Add `AgentStandaloneSettingsPatch` and
  `AgentConfig::write_standalone_settings`, used by the TUI through its
  runtime. A network write triggers the existing settings-reconfiguration
  path so MCP sees the new proxy; log level changes apply at the next
  launch.
- Keep terminal preferences on `AgentTuiPreferencesPatch`,
  `write_tui_preferences` and `App::apply_preferences`. Extend the existing
  shadowed-value check from keymap entries and presets to scalar fields.
- Extend `/providers` with a model-policy stage and `/mcp` with the missing
  server fields. Both continue to use their revision-gated facade calls.

Paths shown to people use the foundation path policy's native form and are
normalized lexically when existence is not required. Credentials are never
rendered or logged: Provider tokens, MCP secrets and userinfo in a proxy
URL are redacted, and debug implementations do not expose drafts or values.

## 5. Code layout

- `crates/tui/src/config_center/`: the full-screen overview, search,
  navigation, render and state.
- `crates/tui/src/commands/config/`: thin `/config` dispatch plus typed
  section forms and draft-to-patch conversion.
- Small, needed additions to `bottom_pane/form`: number, nullable text and
  single-column list controls. No speculative schema renderer.
- Provider model-policy and MCP field additions stay in their existing
  command modules, split further only when a file reaches its reviewed
  module-size limit.
- A completeness test enumerates the serialized default `AgentSettings`,
  `AgentStandaloneSettings`, model policy and MCP server documents and
  fails when a known user-editable leaf has no form coverage. Explicitly
  delegated and internal fields are listed exceptions.

## 6. Phases

### C1 — State, navigation and the center

- Add `ConfigState`, extend the settings bridge reply, create the
  full-screen overview, `/config <section>` routing, search and layer
  summaries.
- Keep the existing quick interface settings available through the
  interface section.

### C2 — `config.toml` domains

- Add forms for general/permissions, sandbox, all memory fields, web tools,
  all compaction fields, standalone network/logging and interface scalar
  preferences.
- Save each domain through the typed patch APIs and refresh its state.
- Preserve comments and unknown keys; report higher-layer overrides.

### C3 — Provider model policy

- Add the model-policy stage to `/providers`, including the optional
  complete turn budget.
- Handle stale registry revisions with the screen's existing reload-and-
  retry behaviour.

### C4 — MCP completeness

- Add protocol mode and per-tool concurrency, approval and exposure maps.
- Keep credential replacement secret and generation-managed.

### C5 — Integrated editors and layers

- Link plugins, hooks and every specialized interface editor.
- Add the layers/files page and post-save override warnings across
  domains.

### C6 — Documentation and validation

- Update the English and Chinese architecture, configuration,
  configuration-reference and terminal-interface pages.
- Run formatting, static checks, repository checks and workspace tests;
  run the site matrix because published documentation changes.

## 7. Validation

From the repository root:

```sh
cargo fmt --all --check
RUSTFLAGS='-D warnings' cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo xtask check
cargo test --workspace --all-features
```

For the site:

```sh
bun install --frozen-lockfile
bun run format:check:site
bun run site:check
bun run site:test
VITE_BASE_PATH=/hanafubuki-agent/ bun run site:build
```

The final handoff names exactly which commands ran and which were blocked;
it does not claim platform coverage the local run cannot establish.

## 8. Decisions

- No raw TOML editor: typed forms keep validation and effects coherent.
- No unknown-key editing: tolerant decoding and in-place writes keep those
  keys, but they are not Agent settings.
- No profile editor in this work: profiles select launch-time layers and
  are displayed as layers, not silently written as if they were runtime
  settings.
- Plugin and marketplace lifecycle calls remain the only writers for their
  records.
- Hook definitions remain outside `config get/set` and outside this center.
- Standalone logging and proxy settings are written by the standalone TUI,
  not added to the embedded facade.
