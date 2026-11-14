# Sandbox completion

Requested on 2026-11-14; baseline `c766d450`, reference Codex `57a808a392`.

## Scope

- Complete Windows elevated setup, separate online/offline sandbox identities,
  operating-system network enforcement, restricted command runner and readiness.
- Connect setup and permission presets to the actual backend, with actionable
  failure reasons. Never silently run a sandboxed request without isolation.
- Add the Linux namespace/bubblewrap execution path alongside the documented
  Landlock compatibility path; protect repository/configuration directories.
- Compare macOS Seatbelt policy and initialization with Codex; retain native
  validation for filesystem, network, PTY and nested-sandbox behavior.
- Update both documentation languages, packaging if needed, and native tests.

## Validation

Exercise allowed reads/writes, denied writes, protected metadata, network on/off,
pipe and PTY IO, interruption and child cleanup. Run repository checks and scoped
Rust tests locally; use native Windows/macOS CI for OS claims. A cross-compile
alone is not native acceptance. Keep setup credentials out of arguments and logs.

## Progress

- Investigation: Windows setup is a placeholder; disabled backend is collapsed to
  a boolean in the picker. Elevated mode is explicitly refused. Linux only uses
  Landlock; macOS policy is already derived from the same Codex revision.
- Implemented bundled bubblewrap with explicit legacy Landlock compatibility,
  macOS overlapping-root protection and nested detection, Windows UAC/account/WFP
  setup and command runner, and TUI setup/readiness integration.
- Dedicated worktree: /tmp/hanafubuki-sandbox; GitHub branch:
  test/sandbox-platforms. Only the Sandbox platforms workflow runs on pushes.
- Commit 4e02ba9d passed Sandbox platforms run 43629054590 on Linux, Windows
  and macOS: scoped check/clippy/native sandbox tests, Linux legacy Landlock,
  Windows repeated elevated setup, filesystem boundaries, native TCP network
  on/off, stdin/stdout/stderr and environment inheritance.
  https://github.com/licyk/hanafubuki-agent/actions/runs/43629054590
- Local cargo fmt --all --check, cargo xtask check, sandbox tests and Windows
  cross-target clippy passed during implementation. No whole-workspace test
  suite was requested or run.
- Native PTY fidelity, MSYS compatibility and interruption remain acceptance
  limits; elevated stdio is a pipe bridge and does not yet create an inner ConPTY.
  Do not describe this implementation as complete Codex terminal parity.
- Kept on the dedicated test branch; no main merge or release. The passed
  sandbox suite is not evidence of full-workspace or interactive-terminal parity.
