# Configuration center handoff, 2026-11-11

The user first asked for work to stop after C3, then requested the C4–C6
continuation. This handoff records both the C3 boundary and the current
working-tree state.

## Completed and pushed

- `a6abcdb` — `docs(plan): build the configuration center`
  - Added `docs/plan-configuration-center.md`.
  - Added the C1–C6 task list at the top of `todo.md`.
- `b5b5086` — `feat(tui): open a searchable configuration center`
  - C1: `/config` opens the full-screen, searchable configuration center.
  - Added `ConfigState`, active-layer and warning summaries, direct section
    routing, and `/config quick` as the legacy quick editor.
- `76f9488` — `feat(tui): edit every config setting from the configuration center`
  - C2: whole-domain forms for General, Sandbox, Memory, Web, Compaction,
    Network/logging and Interface.
  - Added typed standalone writes through the running Agent instance, TUI
    preference saves, higher-layer warnings, credential-safe proxy display,
    and a scalar-setting coverage test.

The C3 changes in this commit complete the shared Provider model policy.

## C3 implementation

`/providers` now has a `p` policy stage. It edits the default Provider,
model, reasoning effort, image input, context window, automatic-compaction
switch and threshold, remote-compaction mode, and either the Agent defaults
or all eleven custom turn-budget fields.

Key files:

- `crates/tui/src/commands/providers/screen/policy.rs`
  builds the policy form and drafts it into `AgentModelPolicySettings`.
- `crates/tui/src/commands/providers/screen/mod.rs`
  owns the new `Policy` stage, keyboard routing, save state and revision
  conflict handling.
- `crates/tui/src/agent_bridge/providers.rs`
  adds `ProviderCall::WriteModelPolicy` and `ProviderReply::ModelPolicySaved`
  and calls the existing revision-gated facade write.
- `crates/agent/src/lib.rs`
  re-exports the typed policy values and turn-budget ceilings the TUI needs
  while preserving the rule that the TUI depends on the facade rather than
  internal crates.
- `crates/tui/src/app/tests/settings.rs`
  covers a full policy save (including all budget fields) and a save over a
  moved registry, which reloads the registry and keeps the user's entries.

A stale revision follows the Provider screen's existing behaviour: the
failure is shown in the form, the newer registry is installed, the user's
entries stay, and the next Save uses the new revision.

## Important correction made during C2/C3

The first standalone-settings bridge read `AgentRuntime::config()`. That is
the runtime's construction-time clone; a running Agent instance has its own
`AgentConfig`. The C2 implementation moved standalone reads and writes onto
the running Agent's facade view, so proxy, logging and TUI changes do not
diverge between the runtime and the live Agent. Preserve this boundary.

The C2 commit predated the required notice markers on new TUI files, so its
`cargo xtask check` did not pass. The markers are included in this C3
commit, and the full repository check passes now. Do not replay or cherry-
pick C2 alone into another branch without adding those markers.

## C3 remaining work, completed in the continuation

- C4 — MCP completeness:
  - Add `protocolMode`.
  - Add per-tool maximum concurrency.
  - Add per-tool approval and exposure policies.
  - Keep credential values secret and generations managed.
- C5 — Integrated editors and layers:
  - Improve direct navigation from the configuration center to specialized
    editors where useful.
  - Add the layers/files page.
  - Apply override warnings consistently across the remaining editors.
- C6 — Documentation and full validation:
  - Update the English and Chinese architecture, configuration,
    configuration-reference and terminal-interface pages.
  - Run the complete Rust and site matrices.
  - Check that every site page prerenders.

The current `/config` Providers row opens `/providers`; the Provider list's
`p` key opens the policy stage. A future C5 improvement can route directly
to that stage if the command gains a policy argument.

## Validation at the C3 stop

Completed:

```powershell
cargo fmt --all --check
$env:RUSTFLAGS='-D warnings'; cargo check -p hanafubuki-agent -p hanafubuki-agent-tui --all-targets
cargo clippy -p hanafubuki-agent -p hanafubuki-agent-tui --all-targets -- -D warnings
cargo xtask check
cargo test -p hanafubuki-agent model_policy -- --nocapture
cargo test -p hanafubuki-agent-tui provider -- --nocapture
cargo test -p hanafubuki-agent-tui policy -- --nocapture
```

Related C2 tests also passed before the C2 commit:

```powershell
cargo test -p hanafubuki-agent-config --all-features
cargo test -p hanafubuki-agent the_standalone_settings_are_read_and_written_through_the_running_agent
cargo test -p hanafubuki-agent-tui config_ -- --nocapture
cargo test -p hanafubuki-agent-tui form_saves -- --nocapture
cargo test -p hanafubuki-agent-tui every_known_scalar_setting_has_an_editor
cargo test -p hanafubuki-agent-tui availability_follows_the_spec
```

Not run at this stop: the full `cargo test --workspace --all-features`, the
site checks/build, and non-Windows CI. Do not claim CI or platform
equivalence from this local Windows run.

## C4–C6 continuation

Completed in the working tree; not committed or pushed.

### C4 — MCP completeness

- The `/mcp` server form edits `protocolMode`, a concurrent-call limit for
  each tool, and per-tool approval and exposure overrides in Advanced.
- Invalid protocol, limit, approval or exposure values stay in the form with
  the field's error; a round-trip test covers every new map and an invalid
  input test covers the parsers.
- The form says `mcp.toml` is unlayered and owner-only. Credential handling
  is unchanged: values stay secret and their generations stay managed by the
  MCP store.

Key files: `crates/tui/src/commands/mcp/form.rs`,
`crates/tui/src/commands/mcp/form/tests.rs`, and the
`McpProtocolMode` re-export in `crates/agent/src/lib.rs`.

### C5 — Integrated editors and layers

- `/config providers` now routes directly to `/providers policy`; `/providers
  policy` opens the policy stage without first rendering the Provider list.
- The Interface form links `/theme`, `/statusline`, `/title`, `/keymap`,
  `/presets` and `/tui`, and replays the chosen editor's command.
- Enter on `/config layers` opens a layers page listing the user, profile,
  project and `-c` sources in read order, trust and not-created markers, and
  every configuration warning.
- Scalar `[tui]` preference writes now join keymap entries and presets in the
  higher-layer shadow warning.

Key files: `crates/tui/src/config_center/layers.rs`,
`crates/tui/src/config_center/mod.rs`,
`crates/tui/src/config_center/forms/mod.rs`,
`crates/tui/src/app/preferences.rs`,
`crates/tui/src/commands/providers/mod.rs` and
`crates/tui/src/commands/providers/screen/mod.rs`.

### C6 — Documentation and validation

- Both languages describe the configuration center in Architecture,
  Configuration, Configuration reference, Terminal interface, MCP and
  Providers & models. Heading ids match between each pair, and `meta.json` and
  `meta.en.json` are unchanged.
- `architecture/notices-policy.toml` records the two new original TUI files.

Validation completed on Windows:

```powershell
cargo fmt --all --check
$env:RUSTFLAGS='-D warnings'; cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo xtask check
cargo test --workspace --all-features
bun install --frozen-lockfile
bun run format:check:site
bun run site:check
bun run site:test
$env:VITE_BASE_PATH='/hanafubuki-agent/'; bun run site:build
```

`cargo test --workspace --all-features`, `bun install --frozen-lockfile` and
`bun run site:test` first hit Windows sandbox limitations (`CreateFileMapping
... Win32 error 5`, Bun temporary-directory `EPERM`, and Vite `spawn EPERM`);
they passed when rerun outside the sandbox. The build prerendered all 34
documentation pages; a script compared every source slug with
`site/.output/public/docs`.

Not run: commit, push, CI, and Linux or macOS coverage. Do not claim platform
equivalence from this local Windows run.
