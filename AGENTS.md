# Agent Notes

## Project

**hanafubuki-agent** is an AI coding and operations agent that runs in two
modes: standalone, as the `hanafubuki-agent` command-line and terminal (TUI)
application, and embedded, as a Rust library a host application builds on.
Hanafubuki is such a host. Dependencies point one way: this repository depends
on no Hanafubuki crate, from any source, and Hanafubuki depends on it
(`cargo xtask check-dependencies` enforces this). The Agent-specific rule
documents (plugins, lifecycle, architecture) arrive with the code under
`docs/rules/`; until then this file holds every rule.

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

## Workspace

- Crates live in `crates/<name>/` and are named `hanafubuki-agent-<name>`,
  except the embedding facade, `hanafubuki-agent`. Repository tooling is the
  `xtask` crate, run as `cargo xtask <command>`.
- `version`, `edition`, `license`, `repository` and `rust-version` are set once
  in the root `[workspace.package]`. Internal dependencies are declared once in
  the root `[workspace.dependencies]` with both `path` and `version`, and
  members use them with `.workspace = true`.
- The edition stays 2021 and rustfmt keeps its defaults.

## Validation

Run from the repository root, selecting scope by the change's impact:

- `cargo fmt --all --check`
- `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets --all-features`
- `cargo xtask check` (every repository check)
- `cargo test --workspace --all-features`

These are the steps `.github/workflows/ci.yml` runs on Linux, Windows and
macOS.

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
requirement (and dependency snippets in `README.md` and `docs/*.md` when the
requirement users write changes), then refreshes `Cargo.lock`. Commit the
result; `cargo xtask tag [--push] [--dry-run]` then makes the annotated
`v<version>` tag on a clean `HEAD`.

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

Module size is reviewed now and will be checked by `cargo xtask
check-module-size` against a policy file once it arrives with the code. Counts
are code lines, so comments never count.

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
