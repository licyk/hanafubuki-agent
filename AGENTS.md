# Agent Notes

## Project

**hanafubuki-agent** is an AI coding and operations agent that runs in two
modes: standalone, as the `hanafubuki-agent` command-line and terminal (TUI)
application, and embedded, as a Rust library a host application builds on.
Hanafubuki is such a host. Dependencies point one way: this repository depends
on no Hanafubuki crate, from any source, and Hanafubuki depends on it
(`cargo xtask check-dependencies` enforces this).

This file holds the rules every change needs; the documents below hold the
rest, each the only copy of its rules. Read the ones a change touches before
changing behavior, and update them when the design changes.

| Document | Read before changing |
| --- | --- |
| [docs/rules/plugins.md](docs/rules/plugins.md) | Any tool, context section, approval rule, continuation hook or observer; the plugin host, its manifest and the plugin configuration; the crate boundaries; the host interface a host embeds the Agent through; the goldens. |
| [docs/architecture.md](docs/architecture.md) | The Agent as a whole: crates, plugin host and faults, tool planning and pipeline, context, Skills, MCP, memory, persistence, and why the less obvious choices were made. |
| [docs/spec.md](docs/spec.md) | The Agent as built in detail: composition, storage, contracts, plugins and tools, sessions and turns, approval, execution, history, memory, MCP, models, Skills, presentation, shutdown. |
| [docs/cli.md](docs/cli.md), [docs/config.md](docs/config.md) | The `hanafubuki-agent` command line (its commands, `exec`'s output formats, approvals and exit codes); the standalone configuration (locations, layers, keys). |
| [docs/release.md](docs/release.md) | The release workflow (`release.yml`), the crates' packaging, the binary archives and their third-party license file (`about.toml`, `about.hbs`). |
| [docs/extraction-plan.md](docs/extraction-plan.md) | The plan this repository was extracted from Hanafubuki by, and its remaining phases (standalone mode, the TUI, the release). |

## Development Principles

- Internal APIs and persisted formats are unstable. Prefer clean breaking
  changes when they improve the design; unless compatibility is requested,
  remove obsolete structures and update current callers, tests, and documents
  rather than preserving aliases, migrations, or deprecated paths. A format may
  change, but its reader never deletes user data: it decodes tolerantly and
  quarantines what it cannot use.
- Implement the smallest coherent design satisfying the requirement and these
  contracts. Do not add speculative flexibility, redundant phases/state,
  hypothetical workflows, extension points, or stricter process.
- Avoid excessive checks and overly strict field validation. Validate only what
  is necessary for correct operation, and tolerate unknown fields in
  configuration files and upstream data instead of rejecting otherwise usable
  input.
- Do not add complexity for speculative, extremely unlikely race conditions or
  security edge cases. If you believe such complexity is necessary, first
  explain exactly what could go wrong, how likely it is, and how much
  additional code it would require. Proceed only after the user approves. Do
  not silently expand the scope of changes.
- Validate external data only once, at the entry boundary. Pass values of known
  types into internal code, and do not repeat defensive checks internally.
- Keep the main function focused on the normal flow. Put validation and
  implementation details in small helper functions below it.
- Write comments only for non-obvious constraints and counterintuitive
  behavior. Do not comment on self-explanatory code.
- Do not manually edit generated files. Change the source and regenerate them.
- Test the real implementation. Do not duplicate business logic in tests.
  Minimize the use of mocks.
- Do not prematurely extract functions that are used only once. Extract a
  function only when it will be reused, encapsulates truly complex logic, or
  represents a concept that can be clearly named.
- Prefer explicit Rust APIs and typed structs. Raw JSON is acceptable only at
  controlled patch/override boundaries, validated in Rust before use or persistence.

## Rules every change needs

- **Locks** a contained panic reaches do not poison. Every panic in the
  Agent unwinds on a contained thread, so the production code of every crate
  uses `parking_lot::{Mutex, RwLock, Condvar}`, never `std::sync::Mutex` or
  `RwLock`: a std lock poisoned by one panic fails every later caller, and a
  drop guard taking it while unwinding aborts the process (`cargo xtask
  check-locks`, outside tests and test support). Every critical section
  leaves its state consistent at any panic point: compute, then assign.
- **Paths** go through `hanafubuki-agent-foundation::paths`
  (`ResolvedPath`, `PathKey`), the only code that calls `fs::canonicalize`:
  `system()` for operating system calls, `native()` for anything a person,
  a model, a log line or a record reads, `key()` for every comparison. Never
  render `system()`, compare resolved `PathBuf`s directly or strip `\\?\`
  (`cargo xtask check-path-boundaries`, with its audited exceptions in
  `architecture/path-policy.toml`).
- **Processes** the Agent starts are the Agent's own: its `ExecutorManager`
  (`hanafubuki-agent-process`) reserves an owner before a spawn, mutates a
  process only with its exact receipt, and keeps owner, receipt and
  metadata until termination is confirmed; nothing is terminated and
  forgotten. Each Agent instance's processes belong to its owner group, a
  stop sweeps that group, and the groups a run leaves are recorded in
  `process-groups.json` and reaped at the next start. A host's processes are
  the host's ([spec.md](docs/spec.md)).
- **Decoding by input source**: how strictly a value is decoded is decided
  by who produced it, not by its type. A request a host's frontend sends
  over IPC rejects unknown fields (`deny_unknown_fields`), and so does an
  integrity manifest we produce and check by digest; our persisted data
  (settings, history, memory, state files) ignores unknown fields, falls
  back per field and quarantines what it cannot use; external producers
  (model providers, MCP servers, the web) are read tolerantly, one bad item
  never failing the rest; model tool arguments are normalized against their
  schema ([plugins.md](docs/rules/plugins.md#model-tool-arguments)).
  `deny_unknown_fields` appears in production code only on a type listed in
  `architecture/strict-decoding-policy.toml` with its category and reason
  (`cargo xtask check-strict-decoding`). Approval, path containment and
  identities stay strict whatever the source.
- **Plugins** are kardia plugins that the kernel reaches only through the
  `hanafubuki-agent-contracts` registries; a registration is an effect of
  the calling plugin, removed when it stops; plugins return errors and never
  panic (`cargo xtask check-plugin-panics`); and the goldens pin what the
  model sees, so a model-visible change regenerates them on purpose
  ([plugins.md](docs/rules/plugins.md)).

## Agent Content Logging

Diagnostic logs, including raw wire bodies (`hanafubuki-agent-model-api`
`trace_wire_body`), may include wire/model content, reasoning, tool
arguments, and committed tool results only at `trace`, truncated to
`MAX_TRACE_CONTENT_BYTES` on a UTF-8 boundary by
`hanafubuki_agent_model_api::bounded_trace_content`, when the user explicitly
selects `trace` through the log level. Never log headers, credentials,
signing tokens, or approval-sensitive fields, or emit content at `debug` or
below. Do not insert diagnostic/raw wire records into conversation history.
Canonical conversation items, including tool calls and committed results,
are persisted through the typed Agent history contract independently of
diagnostic log level; this does not authorize persisting credentials or
approval-sensitive fields.

## Workspace

- Crates live in `crates/<name>/` and are named `hanafubuki-agent-<name>`,
  except the embedding facade, `hanafubuki-agent`. Repository tooling is the
  `xtask` crate, run as `cargo xtask <command>`.
- `version`, `edition`, `license`, `repository` and `rust-version` are set once
  in the root `[workspace.package]`. Internal dependencies are declared once in
  the root `[workspace.dependencies]` with both `path` and `version`, and
  members use them with `.workspace = true`.
- The edition stays 2021 and rustfmt keeps its defaults.
- Every crate but `xtask` is published: it has a `description` and no
  `publish = false`. The facade and the CLI take the repository's
  `README.md` as their `readme`.

## Site

The documentation site is `site/`, the one member of a Bun workspace at the
root (`package.json`, whose `packageManager` pins Bun, and `bun.lock`):
Fumadocs on TanStack Start, prerendered to static files and built for
GitHub Pages under `/hanafubuki-agent/` (`VITE_BASE_PATH`). It is zh-CN by
default with English: every page under `site/content/docs` is `x.mdx` plus
`x.en.mdx`, and `meta.json` and `meta.en.json` list the same slugs in the
same order (`i18n-content.test.ts`). The Markdown under `docs/` moves there
in phase D2 of [docs/plan-thinking-and-site.md](docs/plan-thinking-and-site.md).

- `bun run site:dev|check|test|build|preview` run the site's scripts;
  `bun run format:site` formats `site/` and the root `package.json` and
  `.prettierrc.json` with prettier. Markdown is not formatted
  (`.prettierignore`).
- `patches/` holds a Bun patch of `@tanstack/start-static-server-functions`,
  which fetches its prerendered results from the origin root; the patch
  prefixes Vite's base. Bumping that package means regenerating the patch
  (`bun patch`): without it, first loads under the base still work but
  client-side navigation fails.
- `.github/workflows/site.yml` checks and builds the site; `ci.yml` ignores
  the site's paths.

## Validation

Run from the repository root, selecting scope by the change's impact:

- `cargo fmt --all --check`
- `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets --all-features`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` (an
  `#[allow]` names its `reason`)
- `cargo xtask check` (every repository check: dependencies and layers,
  source layout, module size, strict decoding, model-api context sections,
  path boundaries, locks, plugin panics, the third-party notices)
- `cargo test --workspace --all-features`

These are the steps `.github/workflows/ci.yml` runs on Linux, Windows and
macOS.

- For site changes: `bun install --frozen-lockfile`, `bun run
  format:check:site`, `bun run site:check`, `bun run site:test` and
  `VITE_BASE_PATH=/hanafubuki-agent/ bun run site:build` (the steps
  `site.yml` runs), then check that every page was prerendered.

- For documentation-only changes, check the diff and referenced paths;
  builds and tests are unnecessary.
- Add meaningful tests for changed behavior/contracts, not tests mirroring
  reversible, low-impact edits. Once required checks pass, broaden or repeat
  only for new changes, failures, or unresolved concerns.
- Formatting, static checks, and tests are independent: a check must not
  format, and a test must not format or run static checks.
- Report exactly which validation commands ran and which relevant checks were
  skipped or blocked. Do not claim CI equivalence without reproducing the current
  workflow, or Windows or macOS coverage from Linux results.

### Releases

`cargo xtask version <patch|minor|major> [--dry-run]` raises
`[workspace.package].version` and every internal `[workspace.dependencies]`
requirement (and dependency snippets in `README.md`, `docs/*.md` and the
site's `.mdx` pages when the requirement users write changes), then refreshes `Cargo.lock`. Commit the
result; `cargo xtask tag [--push] [--dry-run]` then makes the annotated
`v<version>` tag on a clean `HEAD`. The tag starts `release.yml`
(crates.io, the binaries, the GitHub release); the checklist, the first
release's crates.io rate limit and how to resume a stopped release are in
[docs/release.md](docs/release.md). `about.toml` lists the licenses the
binary's dependencies may have; a dependency under another one fails the
release's `licenses` job until it is accepted there.

### Rust source and test layout

`cargo xtask check-source-layout` (part of `cargo xtask check`) enforces the
mechanical rules below; keep the rest in review.

- `lib.rs` holds docs, attributes, module declarations, and re-exports only
  (an inline module may serve as a re-export namespace; proc-macro entry
  points stay in `lib.rs`); types, functions, impls, constants, and macros
  live in named modules. A `mod.rs` may hold its own module's code.
- A module with production child modules is `foo/mod.rs`. A module whose only
  children are tests is `foo.rs` with `foo/tests.rs` or `foo/tests/mod.rs`; a
  multi-file test directory is rooted at `tests/mod.rs`.
- `src/` contains no `#[path]` attribute: the module tree is the directory tree.
- Snapshot files (`insta`) live in the `snapshots/` directory beside the tests
  that assert them; a `snapshots/` directory is not a module.
- Re-exports name their items (`pub use module::{Item, Other};`); `src/` has no
  glob re-export (`pub use module::*;`) at any visibility.
- Production files contain no inline tests or `#[test]`; declare only an external
  `#[cfg(test)] mod tests;`.
- Private white-box tests belong in adjacent `src/<module>/tests.rs` or
  `src/<module>/tests/*.rs`; do not widen production visibility just to move tests.
- Public API, cross-crate, and architecture/dependency contracts belong in the
  owning crate's `tests/`. A single-file suite is `tests/<suite>.rs`; a
  multi-file suite is `tests/<suite>/main.rs` with plain `mod` children, and
  helpers shared by several suites live in `tests/support/mod.rs`, reached with
  `#[path = "../support/mod.rs"] mod support;`. Split large
  suites by behavior/invariant while preserving a shared white-box scope when
  private fixtures require it.
- Test helpers: private fixtures are a `fixtures` module inside a `tests`
  module; helpers other crates' tests use are a public `test_support` module
  gated by `#[cfg(any(test, feature = "test-support"))]`, enabled only from
  `[dev-dependencies]`. Test-only accessors to private state are `#[cfg(test)]`
  items beside that state.

### Module size

Module size is checked by `cargo xtask check-module-size` against
`architecture/module-size-policy.toml`. Counts are code lines, so comments
never count; `cargo xtask module-size-report` lists the largest files and
functions.


- Limits: 600 code lines for a production file, 900 for a test file, and 200
  for a production function. Test functions are not limited.
- A limit asks for a review, not a target. Split along responsibilities, never
  mechanically (`part1`/`part2`) or only to get under the number.
- A cohesive module over a limit (an exhaustive dispatch, a data table, a set
  of contract types) keeps an exception whose reason says why and whose
  `ceiling` is the size it was reviewed at. Growth past the ceiling fails, and
  an exception that no longer applies must be removed.
- Adding an exception or raising a ceiling is reported in the handoff, never
  done silently to get an unrelated change through. When a change pushes a
  module you were not asked to restructure over its limit, report it rather
  than refactoring it on the side.
