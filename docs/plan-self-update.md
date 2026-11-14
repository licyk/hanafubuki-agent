# Plan: the update check, the update prompt and self-update

Repositories, read from source on 2026-11-14:

- The Agent: `/root/code_workspace/hanafubuki-agent`, main `2726e6a`. Every
  `file:line` below is at that commit. A milestone still re-reads the symbol
  named beside each reference: M8 (`history_cell/tool/`) and L3
  (`commands/providers/`) are landing while this is written, in files this
  plan does not touch.
- Hanamori: `/root/code_workspace/Hanamori`, `cb6b08b`, for its self-update
  (`src/upgrade.rs`, `src/commands/upgrade.rs`, `src/locking.rs`,
  `src/file_usage/`, `src/upgrade/tests.rs`, `tests/cli/upgrade.rs`,
  `site/content/docs/upgrade.en.mdx`). It is GPL-3.0-only by the same author
  as this repository, which has the same licence: its code may be adapted
  without a notice.
- Codex: `/root/code_workspace/codex`, `57a808a392`, for the update check and
  the prompt (`codex-rs/tui/src/updates.rs`, `updates_cache.rs`,
  `update_versions.rs`, `update_prompt.rs` and its two snapshots,
  `update_action.rs`, `history_cell/notices.rs`, `tui/src/lib.rs:1285-1308`,
  `cli/src/main.rs:799-902`). Apache-2.0: a file ported from it carries the
  marker `architecture/notices-policy.toml:14-20` requires.
- The live release, read through the API on 2026-11-14: `v0.2.0` has eight
  assets, four archives and their `.sha256` files, **each with a `digest`
  (`sha256:<hex>`)** and a size (the archives 15.7–19.8 MB).

The user agreed to the decisions of §2 on 2026-11-14 and decided the
questions of §6, each as recommended: U0–U3 are the work; U4 stays an
optional later milestone, not started without a further decision (Q8).

The request has five parts. Numbers in brackets refer to them throughout:

- [1] **Self-update**, after Hanamori: the application replaces itself.
- [2] **An automatic check** when the terminal interface starts, never
  blocking its start or its use, and **a notice at the bottom** of the
  interface when a newer version exists.
- [3] **A confirmation screen at the next launch**, after Codex: update now,
  skip, or skip this version.
- [4] **A complete plan**: the check, the mechanism, versions, the
  interaction, the errors.
- [5] **Parallel work**: the milestones in lanes that can run at once.

**Scope.** `hanafubuki-agent update [--check] [--force]`; the check at the
terminal interface's launch, its cache and its opt-outs; the notice in the
footer; the prompt before the interface starts; the release's asset names as
a contract; the documentation. **Non-goals:** a `/update` slash command or
updating from inside a running session; package-manager channels (Homebrew,
Scoop, winget, cargo-binstall); pre-release channels, pinning or downgrades;
restarting running instances; installing without asking; delta downloads;
any change to the facade, so the embedded host (Hanafubuki, which has its
own updater) gets none of this; signatures in this round (U4, §6 Q8).

The `todo.md` section "Plan: the update check, the update prompt and
self-update" tracks the milestones U0–U3, and U4 after a further decision (§7). A
path this plan names that does not exist yet is a file its milestone
creates; it is marked "(new)".

## 0. Terms

- **Release**: a GitHub release of `licyk/hanafubuki-agent`, tagged
  `v<version>`. **The latest release**: what
  `GET /repos/licyk/hanafubuki-agent/releases/latest` answers (GitHub's
  newest release that is neither a draft nor a pre-release).
- **Target**: one of the four triples `release.yml` builds
  (`.github/workflows/release.yml:182-192`). **Archive**: the release asset
  `hanafubuki-agent-<version>-<target>.tar.gz` (`.zip` on Windows). **Entry**:
  the one file the updater takes from it,
  `hanafubuki-agent-<version>-<target>/hanafubuki-agent[.exe]`.
- **Installation**: the running executable's real path (symlinks resolved)
  and its **kind**: a released binary, a `cargo install` from crates.io, a
  build from a checkout, or one the updater cannot replace (§3.6).
- **The check**: the background release lookup at the terminal interface's
  launch. **The cache**: `update-check.json` in the data directory.
- **The notice**: the footer's line saying a newer version exists. **The
  prompt**: the screen before the interface starts.
- **Staging directory**: `.hanafubuki-agent-update-<random>/`, created beside
  the executable for one update. **Backup** (Windows only): the replaced
  executable, `<name>.exe.<ms>.outdated`, beside it.

## 1. The request against the code

| Premise | What the code says | Consequence |
| --- | --- | --- |
| The binary knows its version | `--version` is clap's (`crates/cli/src/cli.rs:9-16`: `name = "hanafubuki-agent"`, `version`), so it prints `hanafubuki-agent 0.2.0`; the version is `[workspace.package].version` (`Cargo.toml:8`); no `build.rs`, no git hash. | The current version is `env!("CARGO_PKG_VERSION")`; the staged binary's `--version` must print `hanafubuki-agent <expected>` exactly, as Hanamori checks (`src/upgrade.rs:245-288`). |
| Releases are archives | `release.yml:211-232` packages `dist/$name/` (the binary, `LICENSE`, `NOTICE`, `THIRD-PARTY-LICENSES.txt`) as `$name.tar.gz` (`COPYFILE_DISABLE=1 tar czf`) or, on Windows, `$name.zip` (`7z a -tzip`), plus `$archive.sha256`; `gh release create … dist/*` (`:276`), or on a resumed run `gh release upload … --clobber` (`:270`). No signature, attestation or codesigning. | The updater downloads an archive and takes one entry from it (D4). The digest comes from the API (D3). A resumed release replaces assets in place: a lookup during it may miss this target's archive or see a digest change under the download; both must fail safely (§4). |
| Four targets | `x86_64-unknown-linux-gnu` on ubuntu-22.04 ("glibc 2.35 and newer", `:183`), `x86_64-apple-darwin` (cross-built, not run, `:186,206-207`), `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`; `about.toml:20-25` lists the same four. | A compile-time table of four targets; any other build has no release to update from (D19). |
| One process image, re-executed | `helper_dispatch()` is the first call in `main` (`crates/cli/src/main.rs:29`); it records `std::env::current_exe()` as the sandbox helper at start (`crates/sandbox/src/helper.rs:37-45`) and sandboxed commands run that path. `current_exe` is used nowhere else in production. | On Windows a sandboxed command holds the image open (rename works, delete does not). A still-running old instance's later sandboxed commands execute whatever is at the recorded path, the **new** binary after an update; an unknown helper command line is refused (`helper.rs` `HelperRequest::parse`), so it fails closed (§4). This is true of any upgrade today. |
| The interface starts after the Agent | `interactive::run` checks for a terminal (`crates/cli/src/interactive.rs:43-48`), opens the standalone Agent (`:54`), **starts it** (`:61`, waiting up to 120 s, `crates/cli/src/agent.rs:20,83-88`), then calls `hanafubuki_agent_tui::run` (`:70`), which builds the bridge, preferences, theme, `tui::init()`, `Tui`, `ChatWidget`, `TuiState`, `App` (`crates/tui/src/interface.rs:85-171`). Opening reads the configuration (`agent.rs:66-70`) and takes no lock; the data directory's lock is taken by the start (`crates/agent/src/startup.rs:274`). | The prompt belongs between `open` and `start` (D13): the configuration is read, no Agent runs, no lock is held, no child holds the executable, and the screen does not wait for the plugins. |
| No screen before the interface | Every full-screen view is drawn inside `App` (onboarding is in the chat, the resume picker opens after the UI is up, `interface.rs:168-170`). `tui::init()` sets the modes and opens the inline viewport at the cursor (`crates/tui/src/tui/mod.rs:71-80`); `Tui::draw(height, …)` draws it (`:214`); `InputReader` sends keys as `AppEvent::Key` and a resize as `AppEvent::Redraw` (`crates/tui/src/tui/input.rs:74-94`); tests build `Tui` over `VT100Backend` (`crates/tui/src/tui/tests.rs:27-31`, `crates/tui/src/test_support.rs:27`). | A public entry point in the TUI crate that owns init, input and restore around one small screen, its loop reading a channel, testable on the vt100 terminal (§3.11). |
| Codex replaces nothing | Codex's "Update now" returns `UpdatePromptOutcome::RunUpdate(action)` (`update_prompt.rs:89-101`) and, after the TUI is torn down, the outer binary runs a package manager or installer (`cli/src/main.rs:799-866`), then prints "Please restart Codex". No action detected (`InstallMethod::Other`) → no prompt (`update_prompt.rs:58-60`). | The same shape here: the TUI returns a choice; the CLI runs the same install flow as `update`, in plain lines (D14). The TUI never links the engine (D2). |
| Codex's cache and timing | `version.json` `{latest_version, last_checked_at, dismissed_version}`; refreshed in the background when missing or older than 20 h, never awaited, **this run reading only the earlier value** (`updates.rs:27-58`); failures logged; the prompt respects the dismissal (`updates.rs:150-166`); the banner does not (`app/startup.rs:1292-1300`); pre-releases never newer (`update_versions.rs:1-7,73-77`); disabled in debug builds (`updates.rs:1`). | Taken, except that this run's own check also shows the notice when it finishes ([2]); the prompt still waits for the next launch ([3]). |
| A third-party request precedent | models.dev is off by default "because it is a request to a third party" (`crates/agent/src/model_discovery.rs:21-27`), with the User-Agent `hanafubuki-agent/<version>` and no credential (`crates/model-api/src/models_dev.rs:38,229-262`). | The user asked for the check, so it is on by default, with a setting, an environment variable and a statement of what is sent (D15, §6 Q1). |
| The User-Agent | `crates/foundation/src/user_agent.rs:11-12` holds only `BROWSER_USER_AGENT`; the named agent is a private `const` in `models_dev.rs:38`. | The update crate has its own `hanafubuki-agent/<version>` constant (Correction 1). |
| The proxy | `[network].proxy` is `AgentProxy` (`crates/config/src/network.rs:11-20`), re-exported by the facade (`crates/agent/src/lib.rs:130`) and read from `AgentStandaloneSettings.network` (`crates/config/src/standalone.rs:21-28`); "only the facade reads the configuration" (`xtask/src/dependencies.rs:361`); the facade maps it to `ModelHttpProxy` (`crates/agent/src/network.rs:16-22`). | The update crate takes its own `UpdateProxy`, built by the CLI from the setting's string form (`From<AgentProxy> for String`, `network.rs:49-57`). |
| Layers | Every member needs a row of `LAYERS` (`xtask/src/dependencies.rs:22-27`); the TUI's is `[FACADE]`, the CLI's `[FACADE, tui]` (`:174-175`). | A new row for `hanafubuki-agent-update` over the foundation; the CLI's row gains it; the TUI's does not change (D2). |
| Paths | `ResolvedPath` is the only canonicalizer (`crates/foundation/src/paths/resolved.rs:61-74`, `architecture/path-policy.toml`). | The executable is resolved with `ResolvedPath::resolve`; `system()` for file operations, `native()` in messages. |
| Standalone state | `tui-state.json` (`crates/tui/src/tui_state.rs:1-12`): tolerant decode, quarantine (`quarantine_file`, `crates/agent/src/lib.rs:98`), atomic writes off the loop (`write_atomic`, `FileAccess`, re-exported from `crates/config/src/file.rs:202` at `crates/agent/src/lib.rs:216-228`). | The cache follows it, in the CLI, which has the facade's primitives (D11). |
| The bottom of the screen | The bottom pane is the status indicator, the waiting messages, then the composer (`crates/tui/src/bottom_pane/mod.rs:669-695`); the footer is one row in every mode but the shortcut overlay (`bottom_pane/footer/mod.rs:39-81,154-180`): the status line or `? for shortcuts`, and `← for agents` appended without changing the row count (`composer_empty_line`, `:184`); its props are built in `bottom_pane/chat_composer/render.rs:55-68`. The status line is the person's list (`crates/config/src/tui.rs:150`, `crates/tui/src/appearance/items.rs:42-60`). | The notice goes on the footer's row, right-aligned, so it never adds a row or moves the composer (D12). A status-line item would be absent for whoever set `tui.statusLine`. |
| Asynchronous results | `AppEvent`s on one channel made before the Agent (`interface.rs:28-50`); `AppearanceEvent` is applied by `app/preferences.rs:60` and `chatwidget/appearance.rs:17`. | The notice arrives as an `AppearanceEvent` through a small public sender on `TuiEvents`. `app/mod.rs` (531 of 600) and `agent_bridge/mod.rs` (598 of 600) are not touched. |
| Settings | `[tui]` keys are `AgentTuiPreferences` fields with a `SettingSpec` in `section()` (`crates/config/src/tui.rs:27-117,233`); the settings center shows each; `cargo xtask check-config-reference` wants a row in both reference pages. Hanafubuki does not use `AgentTuiPreferences` (searched 2026-11-14). | `tui.checkForUpdates` is one field and one spec (D15). |
| Children the CLI starts | The CLI and the TUI already start helper programs directly and wait for them (`crates/cli/src/config.rs:210`, `crates/tui/src/external_editor.rs:93`). The Processes rule is about the Agent's processes (`ExecutorManager`). | The staged binary's `--version` and `cargo install` are the CLI's children, started when no Agent runs, waited for and killed at a timeout (D21). |
| Fake servers | The facade's test support serves HTTP from a raw `tokio::net::TcpListener` (`crates/agent/src/test_support/model_server.rs:18-23`). | The update crate's `test_support` serves a release JSON and its archives the same way, recording each request's headers. |

### 1.1 Corrections to the direction given

1. *"`foundation/src/user_agent.rs` builds the User-Agent"*: it holds only a
   browser string. The named agent is private to `models_dev.rs:38`; the
   update crate defines the same string for itself.
2. *"Network code needs the proxy types"*: only the facade may read the
   configuration, so the engine cannot take `AgentProxy`; it takes its own
   `UpdateProxy`, which the CLI makes from `network.proxy`'s string form.
3. *"A crate used by the CLI and the TUI"*: the TUI need not link it. As in
   Codex, the TUI renders a notice and a screen from plain data and returns
   a choice; the CLI owns the cache, the check and the install. The TUI's
   layer stays `[FACADE]`.
4. *"Shown before the main UI … with `Tui` before `App::new`"*: before
   `standalone.start()` instead. Before `App::new` the Agent already runs
   (plugins, MCP servers, the lock), and the screen would wait for its
   start.
5. *"The `.outdated` rename"*: a Windows need. On Unix one `rename(2)` over
   the real path replaces the file atomically while the running process
   keeps its inode: no backup, no moment without an executable.
6. *"The API `digest` and/or the `.sha256` asset"*: the digest is on every
   v0.2.0 asset, and the sidecar comes from the same release and the same
   uploader, so it adds no assurance; the digest alone is required.
7. *"This run uses only the previously cached value"* (Codex): the request
   asks for a notice when an update "is available" at launch, so the check
   that finishes during this session also shows it; only the prompt waits
   for the next launch.
8. *Running instances*: after any upgrade (cargo, by hand, or this one) a
   running old instance's sandboxed commands start the new binary as their
   helper (`helper.rs:42`). It fails closed on an incompatible command
   line; the helper's flags become a cross-version contract (§4, §8).
9. Smaller: `app/mod.rs` is at 531 code lines, not 524; `DEFAULT_STATUS_LINE`
   is at `crates/config/src/tui.rs:150` (the wake plan says `:118`); the
   prefixes D, P and S are also taken in `todo.md`.
10. *Cargo installs, "show it and offer to run it"*: on Windows a running
    executable cannot be overwritten or deleted, and the process that would
    run `cargo install` is that executable, so cargo is expected to fail
    there (to confirm on Windows CI in U1; if cargo copes, the Windows split
    goes). Offered on Unix, shown on Windows (D7, §6 Q2).

## 2. Decisions

Each gives what is decided, why, and the alternative rejected. Those that
were also questions for the user are marked and listed, answered, in §6.

- **D1 — A real self-update from the GitHub release, Hanamori's
  mechanism adapted.** Look up the latest release, take this target's
  archive, check its size and SHA-256, take the entry, run its `--version`,
  then put it in place; never downgrade; the next start runs it. *Why:* the
  request [1] names Hanamori, whose flow is already tested on every failure
  path. *Rejected:* Codex's model of only running an installer (there is no
  installer, and a released binary has no package manager to run).
- **D2 — One new crate, `hanafubuki-agent-update` (`crates/update`), over
  the foundation only.** It holds the release lookup, the version rules, the
  installation kinds and the engine; nothing persistent. The CLI drives it
  (`update`, the check, the prompt's install); the TUI renders a notice and
  a screen from plain data and links nothing of it; the facade never
  depends on it, so the embedded host cannot get it. *Why:* the engine is
  testable against a fake server without a terminal or an Agent; the lanes
  of §7 touch disjoint files; the TUI stays presentation. Its one consumer
  is the CLI; the crate boundary is for the tests (a public API over a fake
  server, which a binary crate's integration tests can reach only through
  the process) and for the layer check, which then proves the facade never
  links it. *Rejected:* a module in `crates/cli` (§6 Q12); modules in the
  TUI (a terminal crate doing downloads, and its row would grow).
- **D3 — Integrity: HTTPS, the API's `digest`, the size, then the staged
  `--version`.** The archive's SHA-256 is taken from the asset's `digest`
  (`sha256:<64 hex>`), compared while streaming, with the size the API gives
  (more bytes than it, or fewer, fail). A release asset without a digest is
  refused: "gives no SHA-256 digest … nothing was changed". The entry must
  then run `--version` within 10 s and print exactly `hanafubuki-agent
  <version>`. *Why:* the digest and the download URL come from the same
  TLS-protected answer; the version check catches a wrong architecture, a
  too-old glibc and a mislabelled archive before anything moves.
  *Rejected:* the `.sha256` sidecar as a fallback or second check (same
  release, same uploader, one more request); trusting the size alone;
  signatures now (no key exists; U4).
- **D4 — Extract from the existing archives; `release.yml` unchanged.**
  The updater reads exactly one entry by its exact path, a regular file, at
  most 16 times the archive's size uncompressed (a release binary is about
  3 times its archive), and never writes any other. Dependencies, each
  only on the platform that needs it: `tar` (default features off: no
  `xattr`) and `flate2` (already in the lock) on Unix, `zip` with only its
  Deflate reader through `flate2` on Windows; `test-support` also writes
  them, for the fake release. *Why:* the licence files travel with the
  binary as the archives intend; v0.1.0 and v0.2.0 already have this
  layout, so `update --check` and the lookup work against real releases
  now; no workflow change to rehearse. *Rejected:* adding bare binaries to
  releases (four more assets without the licence texts, a workflow change,
  the archive kept anyway; it would save one decompression); a single
  checksums file (adds nothing over D3) (§6 Q4).
- **D5 — Replacement: one atomic rename on Unix, move-aside on Windows,
  never a write in place.** Unix: the staged file takes the current file's
  mode, is fsynced, and `rename`s over the **resolved** path; a symlink
  pointing at it stays a symlink. Windows: the running executable is
  renamed to `<name>.exe.<unix-ms>.outdated`, the staged file is renamed
  into place, and if that fails the backup is renamed back; if that fails
  too, the message names both paths to rename by hand. No process is ever
  closed or killed to free a file. *Why:* Unix keeps the old inode for every
  running process and has no moment without an executable; a fresh inode
  also keeps macOS's code-signature cache from killing the new process,
  which a write into the existing file would. Windows lets a running image
  be renamed but not replaced or deleted. *Rejected:* Hanamori's two renames
  everywhere (a window without an executable on Unix, and a backup to clean
  up); its Restart Manager closing the backup's users (it could close
  another of the person's Agent sessions).
- **D6 — One update at a time, and an update already made is noticed.** A
  lock file beside the executable, `.<name>.update.lock`, taken without
  waiting (`File::try_lock`, as `DataDirLock` does,
  `crates/agent/src/data_dir_lock.rs:29-56`); held → "another update of …
  is running; try again when it ends". It stays after the update (removing
  it races a second process that opened it). Under the lock, the installed
  file is asked its `--version`; if it already reports the latest release,
  nothing is downloaded: "… is already 0.3.0 (another process updated it)".
  *Why:* two instances may prompt at once; the second's update becomes a
  no-op instead of a second download that replaces a backup in use.
  *Rejected:* a blocking lock (a stuck update would hang the next); a lock
  in the data directory (two data directories can share one executable).
- **D7 — The installation's kind decides the action** (Codex's
  `UpdateAction`, with this project's channels; §3.6). A released binary in
  a writable directory: self-replace. A `cargo install` from crates.io:
  `cargo install --locked hanafubuki-agent-cli` (with `--root` when the
  root is not cargo's default), run for the person on Unix, only named on
  Windows, where cargo is expected not to replace the running executable
  (confirmed or dropped in U1). A build from a checkout, a `cargo install --path`/`--git`, an
  unwritable directory, or a platform without a release: no action; the
  notice and `update --check` name the manual step. *Why:* replacing a
  cargo-managed file leaves cargo's records stale, and replacing a
  checkout's `target/release` binary with a download is never what a
  developer wants. Wrong guesses fail safely (§3.6). *Rejected:*
  self-replacing every kind; never running cargo (§6 Q2).
- **D8 — Versions: semver precedence, stable only, never down.** A release
  is newer when its tag is `v<semver>` without a pre-release part and its
  version is greater than this build's by precedence (build metadata
  ignored). Equal is up to date; greater here is "newer than the latest
  release": nothing happens. `--force` reinstalls an equal version only.
  *Why:* Hanamori's and Codex's rule; a later patch of an older line that
  becomes "latest" is never installed over a newer build. *Rejected:*
  offering pre-releases (no channel exists; releases are never marked
  pre-release, `release.yml:274`).
- **D9 — After an update, the process ends; the next start runs the new
  version.** "Updated to 0.3.0. The next start of hanafubuki-agent runs it."
  *Why:* Hanamori's rule; re-executing is easy on Unix (`exec`) and not on
  Windows (a child process, its exit code and Ctrl-C forwarded, the old
  image kept mapped); after "Update now" the person is at a shell prompt
  and starts it again. *Rejected:* re-executing with the same arguments
  (§6 Q3).
- **D10 — The check: at the interface's launch, on a thread of the CLI,
  silent, at most every 20 hours.** `interactive::run`, after its terminal
  check, starts a thread `update-check` when the check is allowed (D15,
  D16) and the installation is not a checkout build. It looks up the latest
  release when the cache is missing, older than 20 h, or dated in the
  future (a clock set back), writes the cache, and, when the release is
  newer, sends the notice to the interface. A failure leaves the cache as
  it was (the next launch tries again), is logged at `debug` with its kind
  only, and is never shown. No token is sent. *Why:* [2]; the thread
  overlaps the Agent's start and never touches the interface's loop.
  *Rejected:* the Agent's runtime (the check is not the Agent's, and it
  would have to wait for its start); every launch (§6 Q6).
- **D11 — The cache, `update-check.json` in the data directory, owned by
  the CLI** (§3.9). Tolerant per field, unknown keys kept, a file that is
  not an object quarantined, an unreadable one never written, atomic
  writes. *Why:* the `tui-state.json` pattern, with the facade's primitives;
  the embedded host never has it. *Rejected:* the configuration directory
  (it is state, not configuration); the TUI crate (the check is the CLI's).
- **D12 — The notice is the right side of the footer's row** (§3.10).
  `Update available: 0.3.0 · hanafubuki-agent update`, shortened to
  `Update available: 0.3.0`, omitted when even that does not fit beside the
  row's own content, which it never cuts. It has no key and stays for the
  session. *Why:* persistent and unobtrusive; no row is added, so nothing
  moves when the check lands mid-typing; the `← for agents` suffix is the
  precedent. *Rejected:* a status-line item (absent for whoever set
  `tui.statusLine`, and R8's file); an extra row (moves the composer up
  when it appears); an info line in the transcript (scrolls away) (§6 Q5).
- **D13 — The prompt: Codex's screen and keys, before the Agent starts**
  (§3.11). Shown when the cache's version is newer, not the dismissed one,
  and the installation has an action; on `hanafubuki-agent` and `resume`
  alike; never when a prompt argument was given (`hanafubuki-agent "fix
  it"`, `resume ID PROMPT`), when the check is off, or without a terminal.
  "Skip until next version" records that exact version. *Why:* [3]; the
  person who typed a task wants it run. *Rejected:* skipping it on
  `resume` (it is one keypress, and resuming is how many people start).
- **D14 — "Update now" runs the `update` flow in the terminal.** The screen
  is cleared and the terminal restored; the CLI prints the same lines as
  `hanafubuki-agent update` and ends (D9). A failure prints the error,
  says the current version is unchanged, and exits with 1; the next launch
  prompts again. *Why:* one install path, one set of messages, tested once;
  nothing of the Agent is running. *Rejected:* progress drawn in the TUI
  (a second renderer, and the TUI would link the engine); continuing into
  the interface after a failure (the error would vanish under Fullscreen).
- **D15 — On by default, with two ways off.** `tui.checkForUpdates`
  (boolean, default `true`, applies at the next launch) and
  `HANAFUBUKI_AGENT_NO_UPDATE_CHECK` (any non-empty value turns the check,
  the notice and the prompt off, whatever the setting). `update` works on
  demand either way. What is sent is documented (§3.13). *Why:* the user
  asked for the check; the request carries the version and nothing of the
  person. *Rejected:* off by default like models.dev (§6 Q1).
- **D16 — A debug build never contacts GitHub.** In a debug build the
  release API is `HANAFUBUKI_AGENT_TEST_RELEASE_URL`, and without it the
  check does not run and `update` says "a debug build looks up releases
  only at HANAFUBUKI_AGENT_TEST_RELEASE_URL". A release build compiles the
  variable out (`cfg(debug_assertions)`) and always asks
  `api.github.com`. Plain `http` is accepted only for a loopback address,
  only through that variable. *Why:* every test is a debug build, so no
  test can reach the real API by forgetting a setting; a release binary has
  no redirection an environment could plant. *Rejected:* an override in
  release builds (§5); Codex's compiling the whole feature out (it could
  then not be tested end to end) (§6 Q10).
- **D17 — `GITHUB_TOKEN`, optional, only for `update`, only to the API.**
  Sent as `Authorization: Bearer …` (a sensitive header value) to the
  release lookup of `update` and the prompt's install; never to the asset
  download, which goes to another host by redirect; never by the
  background check. *Why:* Hanamori's rule; the check needs one anonymous
  request a day (GitHub allows 60 an hour per address), and a token the
  person keeps for other work is not sent unasked.
- **D18 — Leftovers are removed cheaply.** Under the lock, every update
  first removes stale staging directories beside the executable (one held
  by a live update cannot exist: it is created only under the lock) and,
  on Windows, every `<name>.exe.*.outdated` it can (one still mapped by a
  running process fails and is left). The check's thread does the same
  `.outdated` sweep on Windows at each launch (one directory read). *Why:*
  a Ctrl-C or crash leaves a staging directory; the backup cannot go until
  its process ends. *Rejected:* a marker file recording leftovers (state
  for what one directory read finds).
- **D19 — The asset names are a contract**, checked. A table of four cfgs
  gives the target; `asset_name(version, target)` and `entry_path` follow
  `release.yml:216-229`; a test reads `release.yml` and `about.toml` from
  the repository and fails when either drifts from the table (§3.5).
  `releasing` says so. *Why:* the updater of version N reads the assets of
  N+1. *Rejected:* a `build.rs` exporting `TARGET` (a build script for one
  string, and it would accept targets with no release).
- **D20 — One error type, one sentence each, saying what was kept**
  (`UpdateError`, §4): every failure before the final rename ends with
  "nothing was changed" or names the version still installed. The
  background check logs `update check: <kind>` at `debug`, never a body,
  header, URL with credentials or token. *Why:* the person must know the
  binary is intact; AGENTS.md's logging rule.
- **D21 — The updater's children are the CLI's.** The staged and the
  installed executable's `--version` (stdin null, stdout piped, stderr's
  first line kept up to 1 KiB, `kill_on_drop`, 10 s, `CREATE_NO_WINDOW` on
  Windows) and `cargo install` (inherited stdio, waited for) are started
  only when no Agent runs (`update`, and the prompt before `start`). *Why:*
  the Processes rule covers the Agent's `ExecutorManager`; starting the
  Agent to update it would hold the lock and its children. The existing
  direct spawns are the precedent (§1).
- **D22 — Rules of the new code.** `lib.rs` declarations and named
  re-exports only; `parking_lot` if a lock is ever needed (none is
  planned); `ResolvedPath` for the executable; external data decoded once
  at the boundary (`Release`), tolerantly (an asset that does not decode is
  skipped, unknown fields ignored); the crate is published with a
  `description`. New files under `crates/tui/src` carry "Modified from
  OpenAI Codex 57a808a392 (Apache-2.0)." (the prompt is ported, the notice
  types follow Codex's notice); the CLI's cache and check are written anew
  from this plan, so no `[[derived]]` entry is needed (if a milestone ports
  Codex's code instead, it adds one and the NOTICE line).

## 3. Design

### 3.1 The flows [1][2][3]

```text
hanafubuki-agent [PROMPT] / resume …            (interactive::run)
  terminal check ─ Standalone::open (configuration read, no lock)
  ├─ allowed? (D15, D16, not a checkout build)
  │    ├─ cache newer, not dismissed, action exists, no PROMPT
  │    │      └─ the prompt (TUI screen) ─┬─ Update now ─► the install flow ─► exit
  │    │                                  ├─ Skip
  │    │                                  └─ Skip until next version (cache written)
  │    └─ thread update-check: due? lookup ─► cache ─► notice (if newer)
  ├─ Standalone::start (lock, plugins)
  └─ hanafubuki_agent_tui::run (notice from the cache at once; later from the thread)

hanafubuki-agent update [--check] [--force]      (no Agent, no lock)
  Standalone::open (proxy; a broken configuration only warns)
  installation ─ lookup ─ compare ─ report (--check ends here)
  └─ the install flow: lock ─ cleanup ─ installed version? ─ staging ─ download
     ─ digest, size ─ entry ─ --version ─ replace ─ exit
```

### 3.2 The crate and its API [1][4] (U0, U1)

`crates/update` (new), `hanafubuki-agent-update`: "The hanafubuki-agent
binary's self-update: the release lookup, the version rules, the
installation's kind, and the download, verification and replacement of the
executable". Row `("hanafubuki-agent-update", &[FOUNDATION])`; the CLI's row
becomes `[FACADE, "hanafubuki-agent-tui", "hanafubuki-agent-update"]`.
Dependencies: the foundation, `reqwest` (`rustls`, `socks`, `stream`,
`system-proxy`, as model-api), `semver` (moved to `[workspace.dependencies]`;
`crates/packages` uses it too), `serde`, `serde_json`, `tokio` (`rt`,
`time`; U1 adds `fs`, `io-util`, `process`), `tempfile`, `sha2`, `hex`,
`log`; U1 adds the archive crates (D4).

The surface U0 lands, which U2 codes against while U0 is in review:

```rust
pub use semver::Version;

pub fn current_version() -> Version;                 // CARGO_PKG_VERSION
pub fn is_newer(latest: &Version, current: &Version) -> bool; // D8

pub const RELEASES_PAGE: &str = "https://github.com/licyk/hanafubuki-agent/releases";
pub fn release_notes_url(version: &Version) -> String; // …/releases/tag/v<version>

pub const RELEASE_TARGETS: [&str; 4];                // release.yml's matrix
pub fn this_target() -> Option<&'static str>;        // the cfg table, D19
pub fn asset_name(version: &Version, target: &str) -> String;
pub fn entry_path(version: &Version, target: &str) -> String;

pub enum UpdateProxy { System, Disabled, Explicit(String) } // Debug redacts the URL
impl UpdateProxy { pub fn from_setting(value: &str) -> Self; } // "system" | "disabled" | URL

pub struct ReleaseClient { /* reqwest client, API URL, optional token */ }
impl ReleaseClient {
    /// GitHub in a release build; the test URL (or an error) in a debug build (D16).
    pub fn for_this_build(proxy: &UpdateProxy, token: Option<String>) -> Result<Self, UpdateError>;
    pub async fn latest(&self) -> Result<Release, UpdateError>;
    pub fn latest_blocking(&self) -> Result<Release, UpdateError>; // own current-thread runtime
}
pub struct Release { pub version: Version, pub assets: Vec<ReleaseAsset> }
impl Release { pub fn asset(&self, name: &str) -> Option<&ReleaseAsset>; }
pub struct ReleaseAsset { pub name: String, pub url: reqwest::Url, pub size: u64, pub sha256: Option<[u8; 32]> }

pub struct Installation { /* executable: ResolvedPath, kind */ }
impl Installation {
    pub fn current() -> Result<Self, UpdateError>;      // current_exe, resolved
    pub fn at(path: &Path) -> Result<Self, UpdateError>; // tests
    pub fn executable(&self) -> &ResolvedPath;
    pub fn kind(&self) -> &InstallKind;
    pub fn writable(&self) -> bool;                      // creates and drops a temp file beside it
}
pub enum InstallKind { Release, Cargo { root: PathBuf, default_root: bool }, Checkout }

// What can be done for this installation (§3.6): the notice's wording (U2),
// the prompt's first option (U3) and `update` (U1) all read it.
pub enum UpdateAction { Replace, Cargo { command: Vec<String> }, Manual(ManualReason) }
pub enum ManualReason { Checkout, Unwritable, NoReleaseForPlatform, CargoOnWindows }
impl Installation { pub fn action(&self) -> UpdateAction; }

pub enum UpdateError { /* one variant per row of §4 */ } // Display: §4's sentence; kind(): a log label

#[cfg(any(test, feature = "test-support"))]
pub mod test_support; // FakeRelease: a release JSON and assets on 127.0.0.1, requests recorded
```

An `Installation` is resolved once, by `update` at its start and by the
interface's launch before the prompt, and the install flow takes it as
given: a Linux process whose executable was replaced reads its own path as
`<path> (deleted)` (checked on 2026-11-14: `readlink /proc/<pid>/exe` after
a `mv` over the running file), so a later resolution would fail exactly
when another process has just updated.

U1 adds the engine:

```rust
pub enum Progress { Downloading { done: u64, total: u64 }, Verifying, Checking, Installing }
pub struct Installed { pub version: Version, pub backup: Option<PathBuf>, pub already: bool }
pub async fn install(client: &ReleaseClient, installation: &Installation, release: &Release,
                     force: bool, progress: &mut dyn FnMut(Progress)) -> Result<Installed, UpdateError>;
pub fn install_blocking(…) -> Result<Installed, UpdateError>;
pub fn run_cargo(installation: &Installation, command: &[String]) -> Result<(), UpdateError>;
pub fn sweep_outdated(installation: &Installation); // D18, best effort, Windows only
```

### 3.3 The release lookup [1][2] (U0)

`GET https://api.github.com/repos/licyk/hanafubuki-agent/releases/latest`,
headers `Accept: application/vnd.github+json`, `User-Agent:
hanafubuki-agent/<version>`, and `Authorization` only per D17. Connect
timeout 10 s, the whole lookup 30 s, the body at most 1 MiB. The proxy is
`UpdateProxy` (`System`: reqwest's system proxy and the proxy variables).
Statuses: 404 → no release; 401 → the token was rejected; 403 or 429 → the
rate limit; any other non-2xx → "GitHub answered N". The body is read
tolerantly: an object with `tag_name` (required), `draft`, `prerelease`
(missing counts as `false`), `assets` (missing counts as none); each asset
with `name`, `browser_download_url`, `size` and `digest`, an asset that does
not decode being left out; unknown fields ignored. Then: a draft or
pre-release, or a tag that is not `v<semver>` without a pre-release part, is
"not a release this version understands". The asset URL must be `https`
(loopback `http` only through D16's variable). Nothing in the answer is
shown as given: the version is printed from the parsed `Version`, the
release notes URL is built from it, an asset name only after it matched the
expected name exactly.

### 3.4 Versions [4] (U0)

| This build | Latest release | Result |
| --- | --- | --- |
| 0.2.0 | 0.3.0 | newer: notice, prompt, `update` installs |
| 0.2.0 | 0.2.0 | up to date; `update --force` reinstalls |
| 0.2.0+local | 0.2.0 | up to date (build metadata ignored) |
| 0.3.0-rc.1 (a local build) | 0.3.0 | newer |
| 0.3.0 | 0.2.1 (a later patch of an old line) | "newer than the latest release": nothing; `--force` refuses |
| 0.2.0 | tag `v0.3.0-beta.1`, or `nightly` | not understood: the check ignores it, `update` says so |

The cache keeps the latest stable version found; after an update to it, the
cache and the binary agree, so nothing is shown (§3.9).

### 3.5 The asset contract [1][4] (U0)

| Target (cfg) | Archive | Entry |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` (`linux`, `x86_64`, `gnu`) | `hanafubuki-agent-<v>-x86_64-unknown-linux-gnu.tar.gz` | `hanafubuki-agent-<v>-x86_64-unknown-linux-gnu/hanafubuki-agent` |
| `x86_64-apple-darwin` (`macos`, `x86_64`) | `hanafubuki-agent-<v>-x86_64-apple-darwin.tar.gz` | `…/hanafubuki-agent` |
| `aarch64-apple-darwin` (`macos`, `aarch64`) | `hanafubuki-agent-<v>-aarch64-apple-darwin.tar.gz` | `…/hanafubuki-agent` |
| `x86_64-pc-windows-msvc` (`windows`, `x86_64`, `msvc`) | `hanafubuki-agent-<v>-x86_64-pc-windows-msvc.zip` | `…/hanafubuki-agent.exe` |

Any other build has `this_target() == None`: no release for this platform
(an x86_64 build under Rosetta is x86_64 and updates as such, as Hanamori
keeps the running architecture). `crates/update/tests/release_contract.rs`
(new) reads `../../.github/workflows/release.yml` and `../../about.toml`
from `CARGO_MANIFEST_DIR` and returns without asserting when they are absent
(the crate as published); it checks that the matrix's targets, `about.toml`'s
`targets` and `RELEASE_TARGETS` are the same set, and that the packaging
step still has `name="hanafubuki-agent-$VERSION-$TARGET"`, the
`binary=hanafubuki-agent` / `hanafubuki-agent.exe` lines, `$name.zip` on
Windows and `$name.tar.gz` otherwise, and the copy into `dist/$name/`.
`releasing` gains a section: the names, the layout and the stable flag are
read by every released updater; changing them strands those versions'
self-update; the `--clobber` resume replaces assets under running lookups,
which fail safely.

### 3.6 The installation's kind and action [1] (U0)

From the executable's real path `P` (`ResolvedPath::resolve(current_exe)`,
once, §3.2; a missing file, as Linux reports a replaced one with
` (deleted)`, is "cannot find the running executable"):

1. **Checkout** when `P`'s directory holds both `deps/` and `.fingerprint/`
   (a cargo profile directory: `target/debug`, `target/release`,
   `target/<triple>/release`).
2. **Cargo** when `P`'s directory is named `bin` and its parent `R` holds
   `.crates2.json` whose `installs` has a key `hanafubuki-agent-cli
   <version> (<source>)` whose `bins` lists `P`'s file name, the source
   being a registry (`registry+…` or `sparse+…`). cargo records crates.io as
   `registry+https://github.com/rust-lang/crates.io-index` with the sparse
   protocol too (seen in a `~/.cargo/.crates2.json` on 2026-11-14). A
   `path+` or `git+` source is a **Checkout** (`cargo install --path
   crates/cli`). `default_root` is whether `R` is `$CARGO_HOME`, else
   `~/.cargo`. The file is cargo's, read tolerantly: anything unreadable
   means "not cargo".
3. **Release** otherwise.

The action: Release in a writable directory (`writable()` creates and
drops `.hanafubuki-agent-update-probe-<random>` there) → Replace; Release
without `this_target()` → Manual(NoReleaseForPlatform); Release in an
unwritable directory → Manual(Unwritable); Cargo on Unix → Cargo with
`cargo install --locked hanafubuki-agent-cli` and `--root R` unless
`default_root`; Cargo on Windows → Manual(CargoOnWindows); Checkout →
Manual(Checkout).

Wrong guesses fail safely. A released binary someone put in `~/.cargo/bin`
is not in `.crates2.json`: it is replaced like any released binary. A
cargo-installed copy replaced later by hand still lists as cargo: running
cargo reinstalls it, which is harmless. A checkout misread as a release
would need `deps/` and `.fingerprint/` missing from its directory, which
cargo always creates. A read-only location (a Nix store, a system
directory) is Unwritable.

### 3.7 The engine [1] (U1)

In order, everything before step 8 leaving the installed file untouched:

1. **Look up** (§3.3), **compare** (§3.4). Up to date and no `--force`, or
   newer here: stop. Find this target's asset; none → "release 0.3.0 has no
   archive for <target> …"; no digest → refuse (D3).
2. **Lock** (D6). **Clean** stale staging directories and, on Windows,
   backups (D18).
3. **Ask the installed file** its `--version` (D6); already the latest and
   no `--force` → `Installed { already: true }`.
4. **Stage**: `tempfile::Builder::new().prefix(".hanafubuki-agent-update-")
   .tempdir_in(<executable's directory>)` (failure: the directory is not
   writable). Same filesystem, so the final rename is atomic; never `/tmp`.
5. **Download** the archive into it, streaming: SHA-256 and the byte count
   as it comes, more bytes than the size stops at once, fewer is
   "ended after X of Y bytes"; connect timeout 10 s, 60 s without a byte
   fails; no total limit (19 MB at a slow link). `Progress::Downloading`
   at most every 100 ms. Then the digest compare (`Verifying`).
6. **Extract** the entry (§3.5) to `<staging>/hanafubuki-agent[.exe]`:
   exactly that path, a regular file, at most 16 times the archive's size;
   on Unix the current executable's mode; fsync.
7. **Check** the staged file (`Checking`): `--version`, 10 s, D21; the
   output must be `hanafubuki-agent <latest>`.
8. **Replace** (`Installing`, D5). On Windows, if a rename fails with a
   sharing violation (an antivirus scan of the new file), it is retried up
   to ten times over two seconds before the rollback (§6 Q7).
9. The staging directory is removed on every path (its `Drop`); a process
   killed meanwhile leaves it to step 2 of the next update.

`run_cargo` (Unix): resolves `cargo` from the absolute entries of `PATH`
(none → "cannot find cargo on PATH; run `cargo install --locked
hanafubuki-agent-cli` yourself"), runs it with the person's environment, in
a new temporary directory (so no project's `.cargo/config.toml` or
`rust-toolchain.toml` applies, as Codex runs its updater from `/`), stdio
inherited, and waits. Ctrl-C reaches both; whether cargo moves a finished
file into place (so either version is left whole) is confirmed in U1
before the docs say so.

### 3.8 The `update` command [1] (U1)

`hanafubuki-agent update [--check] [--force]` (`--check` and `--force`
conflict: usage error, 2). It opens the standalone runtime for
`network.proxy` and does not start the Agent, so it works while another
Agent runs; when the configuration cannot be read it warns
(`warning: the configuration could not be read (…); the update uses the
system proxy`) and goes on, since an update may be how a broken setup is
fixed. `GITHUB_TOKEN` per D17. The report goes to standard output, progress
to standard error; exit 0 when it did what was asked or there was nothing
to do, 1 on any failure (`error: …`, `crates/cli/src/exit.rs`).

`update --check`:

```text
Current version  0.2.0
Latest release   0.3.0
Executable       /home/me/.local/bin/hanafubuki-agent
Installed as     a released binary
Release notes    https://github.com/licyk/hanafubuki-agent/releases/tag/v0.3.0

An update is available: run `hanafubuki-agent update` to install 0.3.0.
```

`Installed as` is `a released binary`, `a released binary (its directory is
not writable)`, `cargo install (in /home/me/.cargo)` or `a build from a
checkout`. The last line, by case:

| Case | Last line |
| --- | --- |
| Up to date | `hanafubuki-agent 0.3.0 is the latest release.` |
| Newer here | `This build (0.4.0-rc.1) is newer than the latest release (0.3.0); nothing to install.` |
| Cargo | ``An update is available: run `cargo install --locked hanafubuki-agent-cli` to install 0.3.0.`` |
| Checkout | `This executable was built from a checkout (/home/me/src/hanafubuki-agent/target/release): update the checkout and build it again.` |
| Unwritable | `An update is available, but /usr/local/bin is not writable: install 0.3.0 by hand (https://licyk.github.io/hanafubuki-agent/docs/installation/), or run the update as a user who can write there.` |
| No release for the platform | `An update is available, but there is no released binary for this platform: install 0.3.0 the way you installed this one.` |

`update` (standard error until the last line):

```text
Updating hanafubuki-agent 0.2.0 → 0.3.0 at /home/me/.local/bin/hanafubuki-agent
Downloading hanafubuki-agent-0.3.0-x86_64-unknown-linux-gnu.tar.gz: 12.3 of 18.9 MiB (65%)
Verifying the SHA-256 digest: ok
Checking the new executable: hanafubuki-agent 0.3.0
Installing: done
Updated to 0.3.0. The next start of hanafubuki-agent runs it; running ones keep 0.2.0 until they restart.
```

On a terminal the download line is rewritten in place (`\r`); otherwise only
its first state and `… 18.9 MiB: done` are printed. On Windows a line
follows: `The previous executable is kept as
hanafubuki-agent.exe.1763107200000.outdated until it no longer runs; the
next update or start removes it.` Other endings: up to date
(`hanafubuki-agent 0.3.0 is the latest release; nothing to do.`), already
updated by another process (`hanafubuki-agent at … is already 0.3.0
(another process updated it).`), Cargo on Unix (`Running cargo install
--locked hanafubuki-agent-cli` and cargo's own output, then the same
`Updated to …` line), the manual cases (their `--check` sentence, exit 1).

### 3.9 The check and the cache [2][4] (U2)

`<data dir>/update-check.json`:

```json
{
  "latestVersion": "0.3.0",
  "checkedAtMs": 1763107200000,
  "dismissedVersion": "0.3.0"
}
```

| Field | Written by | Read as |
| --- | --- | --- |
| `latestVersion` | a successful check | a stable semver, else absent |
| `checkedAtMs` | a successful check (milliseconds since the epoch, the repository's convention) | a number, else absent (the check is due) |
| `dismissedVersion` | "Skip until next version" | a semver, else absent |
| any other key | a later version | kept as it is when the file is rewritten |

A file that is not a JSON object is moved to `.recovery/`
(`quarantine_file`) and read as empty; a field that does not read is
absent, and the file is moved aside before it is next written, as
`tui-state.json` does; a file that exists but cannot be read is never
written, and the check does not run. Writes are `write_atomic(…,
FileAccess::Private)`, re-reading the file just before so a dismissal
another instance wrote meanwhile is kept (two instances racing a write keep
the last; a lost dismissal only means one more prompt). The check is due
when `checkedAtMs` is absent, more than 20 h ago, or after now. A cache
written by a newer version reads the same; one whose `latestVersion` is not
newer than this build shows nothing.

`crates/cli/src/update_check/` (new): `mod.rs` (whether the check is
allowed: the setting, the variable, a debug build without the test URL, a
checkout build; `start(…)` spawning the thread), `cache.rs`, `check.rs`.
`interactive::run` reads the cache before `start`, passes the notice it
implies in `TuiOptions`, and starts the thread with the interface's
notifier.

### 3.10 The notice [2] (U2)

The TUI's public data (`crates/tui/src/update_notice.rs` (new)):
`TuiUpdateNotice { version: String, action: String }` (the CLI formats both
from typed values), `TuiOptions::update_notice: Option<TuiUpdateNotice>`
(the cache's, at once), and `TuiEvents::update_notifier() ->
TuiUpdateNotifier` (`Clone + Send`, `notify(notice)`, sending
`AppearanceEvent::UpdateNotice`). The composer keeps it beside the status
line (`set_update_notice`, as `set_status_line`, `bottom_pane/mod.rs:497`),
and `FooterProps` gains `update_notice: Option<&TuiUpdateNotice>`.

`render_footer` draws it right-aligned on the footer's row, two columns from
the right edge, in the single-row modes (empty composer, a draft, the Esc
hint, the quit reminder), never in the shortcut overlay or in place of the
history search's line. It is laid out after the row's own content, with a
gap of three columns:

| Fits beside the row's content | Shows |
| --- | --- |
| the full form | `Update available: 0.3.0 · hanafubuki-agent update` |
| the short form | `Update available: 0.3.0` |
| neither | nothing |

The action, by kind: `hanafubuki-agent update` (Replace); `cargo install
--locked hanafubuki-agent-cli` (Cargo, both systems); `hanafubuki-agent
update --check` (every manual case: it explains). `Update available:` and
the action are dim, the version takes the accent colour. At 120 columns
beside the default status line it shows in full; at 80, short; at 50,
usually not at all. It is shown for a dismissed version too (Codex's
banner rule; §6 Q9), and is not dismissed by a key. R8's
`background-tasks` item lives inside the status line, on the left, and its
waiting line in the status indicator above the composer: neither competes
with the notice, which yields first when the row is full.

### 3.11 The prompt [3] (U3)

`crates/tui/src/update_prompt.rs` (new), ported from Codex's
`update_prompt.rs`: `pub fn update_prompt(prompt: TuiUpdatePrompt) ->
Result<TuiUpdateChoice, String>` with `TuiUpdatePrompt { current, latest,
release_notes, update_now }` (strings the CLI formats) and
`TuiUpdateChoice::{UpdateNow, Skip, SkipVersion}`. It calls `tui::init()`,
starts an `InputReader` on a channel of its own after dropping what the
terminal already holds (an Enter typed while the program started must not
choose "Update now"), draws in the inline viewport at the cursor at the
screen's height (in Fullscreen and Scrollback alike: it is drawn before the
interface chooses its mode), handles `Key` and `Redraw` (resize, focus),
ignores paste and mouse, and on a choice stops the input, clears the
viewport (`Tui::leave`) and restores the terminal. The loop takes the
receiver, so tests drive it with a channel and a `VT100Backend`.

At 80 columns:

```text

  Update available · 0.2.0 → 0.3.0
  Release notes: https://github.com/licyk/hanafubuki-agent/releases/tag/v0.3.0

› 1. Update now (downloads 0.3.0 and replaces this executable)
  2. Skip
  3. Skip until next version

  enter continue · esc skip

```

For a cargo installation (Unix) the first option is ``Update now (runs
`cargo install --locked hanafubuki-agent-cli`)``, with `--root …` when it is
passed. At 40 columns, option 3 highlighted (the snapshot is the
reference):

```text

  Update available · 0.2.0 → 0.3.0
  Release notes:
  https://github.com/licyk/hanafubuki-
  agent/releases/tag/v0.3.0

  1. Update now (downloads 0.3.0 and
     replaces this executable)
  2. Skip
› 3. Skip until next version

  enter continue · esc skip
```

Keys: Up or `k`, Down or `j` (wrapping); `1`, `2`, `3` choose at once;
Enter chooses the highlighted row (initially 1); Esc, Ctrl-C and Ctrl-D are
Skip; key releases are ignored. The heading and the hint are drawn as
Codex's are, the rows with `selection_popup_common`'s row renderer and
`render_menu_surface` (`bottom_pane/selection_popup_common.rs:67`); the URL
is an OSC 8 link where the terminal supports one. On a screen too short for
everything, the highlighted row and the hint stay visible (Codex's 28×12
test).

`crates/cli/src/update_check/prompt.rs` (new) decides (D13), builds the
strings, writes `dismissedVersion` on SkipVersion (a failed write is logged
at `warn` and not shown), and on UpdateNow runs the install flow of
`crates/cli/src/update/install.rs` with a fresh lookup (the latest may be
newer than the cache's: `Updating 0.2.0 → 0.3.1`).

### 3.12 Settings, the environment and what is sent [2][4] (U2)

`tui.checkForUpdates`: `AgentTuiPreferences::check_for_updates: bool`,
default `true`; `SettingSpec::new("tui.checkForUpdates", "Check for
updates", SettingKind::Boolean)`, `SettingApply::NextLaunch`, described "At
launch, asks GitHub at most once a day whether a newer release exists". Rows
in both `config-reference` pages (`#tui`).

`HANAFUBUKI_AGENT_NO_UPDATE_CHECK` (U2), `GITHUB_TOKEN` (U1) and, for
development, `HANAFUBUKI_AGENT_TEST_RELEASE_URL` (U0) in `environment#read`.

What the check sends, for `terminal-interface#updates` (U2): "When the
terminal interface starts, at most once every 20 hours, it sends one HTTPS
request, `GET https://api.github.com/repos/licyk/hanafubuki-agent/releases/latest`,
with `Accept: application/vnd.github+json` and `User-Agent:
hanafubuki-agent/<version>`, through `network.proxy`. No token, cookie or
identifier is sent, and nothing of your configuration, sessions or
projects. GitHub sees the request's address and the version, as it would a
download. The answer's version is kept in `update-check.json`. Turn it off
with `tui.checkForUpdates = false` or `HANAFUBUKI_AGENT_NO_UPDATE_CHECK=1`;
`hanafubuki-agent update` still works when you run it." `update`'s own
requests (the lookup, `GITHUB_TOKEN` if set, and the archive from GitHub's
download host) are listed in `cli-reference#update`.

### 3.13 Documentation, both languages

| Page | Milestone | Change |
| --- | --- | --- |
| `architecture` `#crates-and-layers` | U0 | `update` in the Interfaces row, used only by the CLI |
| `plugin-rules` `#crates-and-dependencies` | U0 | the crate's row |
| `releasing` | U0 | the asset contract section and the stable flag; a release that adds a crate creates it (minutes) |
| `internals` | U1 | the sandbox helper's command line is read by later versions: flags are only added (§8) |
| `environment` `#read` | U0, U1, U2 | the test URL; `GITHUB_TOKEN`; the opt-out variable (rows apart, so the lanes do not conflict) |
| `cli-reference` | U1, U3 | `#update` (new) and `#agent-start` (`update` does not start the Agent); `#interface`: the prompt |
| `installation` | U1 | "Upgrading" becomes "Updating": `update`, cargo, by hand, the interrupted Windows update, the backup; `cargo install --locked` (§6 Q11) |
| `exit-codes` | U1 | 1 includes an update that could not be made |
| `network-logging` `#proxy-scope` | U1, U2 | a row for `update`, another for the check |
| `terminal-interface` | U2, U3 | `#layout`'s footer item; `#updates` (new): the check, the notice, what is sent, the prompt |
| `config-reference` `#tui` | U2 | `tui.checkForUpdates` |
| `data-directory` `#contents` | U2 | `update-check.json` |

## 4. Errors and edge cases

Every message is the whole `error:` line; each before the replacement ends
with "nothing was changed" or names the version still installed. The check
shows none of them.

| Case | What happens and what the person reads |
| --- | --- |
| Offline, DNS or TLS failure, a proxy needed | `could not reach api.github.com (the connection failed); check the network and network.proxy. Nothing was changed.` (`timed out` for a timeout). The check: nothing, retried at the next launch. |
| Rate limit (403, 429) | `GitHub refused the release lookup (403): its rate limit was probably reached. Try again later, or set GITHUB_TOKEN.` |
| A rejected token (401) | `GitHub rejected GITHUB_TOKEN (401); unset it or use a valid token.` |
| No release (404) | `the repository has no published release yet (404).` |
| Another status | `GitHub answered 502 to the release lookup; try again later.` |
| Malformed JSON, a draft, a pre-release, a tag that is not a version | `GitHub's answer is not a release this version understands (the tag "nightly" is not v<version>); nothing was changed.` |
| No archive for this target (a release still uploading, a resumed release) | `release 0.3.0 has no archive for x86_64-unknown-linux-gnu (hanafubuki-agent-0.3.0-x86_64-unknown-linux-gnu.tar.gz); if it was just published, try again in a few minutes.` |
| No release for this platform | `there is no released binary for this platform; update it the way you installed it.` |
| No digest | `release 0.3.0 gives no SHA-256 digest for <archive>; nothing was changed.` |
| Digest mismatch (corruption, an asset replaced during the download) | `the download of <archive> does not match its SHA-256 digest; nothing was changed. Try again.` |
| Truncated or overlong download | `the download of <archive> ended after 12.3 of 18.9 MiB; nothing was changed. Try again.` / `…is longer than the release says…` |
| Disk full, a write error | `could not write the download beside the executable (/home/me/.local/bin): No space left on device; nothing was changed.` |
| Unwritable directory | `--check`'s Unwritable sentence (§3.8); exit 1. No prompt; the notice points at `update --check`. |
| No entry, a symlink entry, an entry over 16 times the archive | `<archive> does not contain hanafubuki-agent-0.3.0-<target>/hanafubuki-agent as a file; nothing was changed.` |
| The new executable does not run (wrong architecture, glibc older than 2.35) | `the new executable does not run here (version 'GLIBC_2.35' not found); hanafubuki-agent 0.2.0 is unchanged. The Linux build needs glibc 2.35 or newer.` (the glibc sentence only on Linux) |
| It reports another version, or nothing in 10 s | `the new executable reports "hanafubuki-agent 0.2.9", not hanafubuki-agent 0.3.0; nothing was changed.` / `…did not answer --version within 10 s…` |
| Another update holds the lock | `another update of /home/me/.local/bin/hanafubuki-agent is running (.hanafubuki-agent.update.lock); try again when it ends.` |
| Two instances launch at once and both prompt | Both may show the prompt; each resolved its installation at launch, before either update. Updating in both: the second either is told another update runs (the lock does not wait) or, after the first finished, finds the installed file at the latest version and says so (D6). |
| The executable moved or deleted before this process resolved it | `cannot find the running executable (it was moved or deleted); nothing was changed.` The installation is resolved once, at `update`'s start or the interface's launch (§3.2), so this happens only when the file went before then. |
| The executable replaced while running (cargo, by hand, another update) | Unix: it keeps running its old inode; the next start runs what is there. The path resolved at launch is still the installed file's, which D6 asks its `--version`: already the latest, the update is a no-op. |
| Running from a symlink | The real file is replaced; the link stays a link and now points at the new version (in a versioned directory, the directory's name then lies). |
| A cargo installation | Unix: `cargo install --locked hanafubuki-agent-cli` runs (§3.7); cargo fails → ``cargo install --locked hanafubuki-agent-cli failed (exit status: 101); hanafubuki-agent 0.2.0 is unchanged.`` Windows: the command is named, nothing runs. |
| A checkout build, a debug build | No check, no notice, no prompt (a debug build also needs D16's variable for `update`); `update --check` says to rebuild. |
| A package-manager copy | None exists today; a future channel joins `InstallKind`. One in an unwritable prefix is Unwritable already. |
| Windows sharing violation (an antivirus scan, an open handle) | Retried for two seconds (§6 Q7), then `cannot move /…/hanafubuki-agent.exe aside: another program holds it open (os error 32); close it and try again. Nothing was changed.` |
| The new file cannot be put in place (Windows) | `cannot put the new executable at C:\…\hanafubuki-agent.exe (…); the previous one was put back.` If the rollback fails too: `…, and the previous one could not be put back (…). Rename C:\…\hanafubuki-agent.exe.1763107200000.outdated to C:\…\hanafubuki-agent.exe by hand.` |
| Killed during the download (Ctrl-C, a crash) | The installed file was never touched; the staging directory stays until the next update removes it (D18). |
| Killed between the renames (Windows only, milliseconds) | `hanafubuki-agent.exe` is missing and the backup holds it: `installation` says to rename the newest `.outdated` back. Unix has no such moment. |
| A backup left behind (Windows) | Removed by the next update or the next interface start once no process maps it. |
| Running instances after an update | They keep their version. Their later sandboxed commands start the new executable as the helper (`helper.rs:42`), which refuses a command line it does not read (exit 126): the helper's flags must stay readable across versions (§8). `update` says running ones keep the old version until they restart. |
| `--force` against an older release | `refusing to replace 0.3.0 with the older release 0.2.1; --force only reinstalls the same version.` |
| Clock skew | A `checkedAtMs` in the future makes the check due; nothing else depends on the clock. |
| The cache written by a newer version, or after an update | Read the same; nothing shown when its version is not newer than this build. |
| The cache unreadable, not JSON, a field broken | §3.9: quarantined or left, never fatal; at worst one more request. |
| The data directory read-only | The cache is not written (logged at `debug`); the check runs at each launch. |
| Ctrl-C on the prompt | Skip; the interface starts. |
| A terminal too small for the prompt | The highlighted row and the hint stay visible; resizing redraws. |
| The check finishes after the person quit | The thread ends with the process; the write is atomic. |
| The check finishes before the interface is up | The event waits in the channel and is applied at the first loop. |
| `exec`, other commands, no terminal, the embedded host | No check, no notice, no prompt: only `interactive::run` (after its terminal check) starts it, and the facade has none of it. |

## 5. Security

The design's honest limit: **nothing is signed.** The integrity of an
update is GitHub's: TLS to `api.github.com` and to its download host, and
the digest GitHub computed for the uploaded file.

| Attacker | Could | The design |
| --- | --- | --- |
| On the network (a hostile Wi-Fi, a proxy) | Read or change traffic only with a certificate the system trusts; otherwise block or delay | TLS for the lookup and the download (`https` required, D16); the digest from the TLS-protected answer; a blocked lookup means no update, and never an older one (D8). A freeze (always answering an old release) keeps the person on their version. |
| Controlling a release (a stolen maintainer account or token, a compromised workflow) | Publish a malicious archive with a matching digest, reaching everyone who updates | Nothing in this round beyond what a manual download has. The staged `--version` is a sanity check, not a defence. Decided for later (U4, §6 Q8): sign each archive in `release.yml` with a minisign (Ed25519) key kept as a secret, and verify it with the public key compiled into the binary (`minisign-verify`, one small dependency). GitHub's artifact attestations are the alternative; verifying them in-process is far heavier. Enabling immutable releases would forbid the `--clobber` resume path (`releasing#resuming-a-release-that-stopped`). |
| A local user who can write the executable's directory | Replace the executable at any time, with or without the updater | Staging is a private, randomly named directory inside the executable's own directory (never `/tmp`), so the updater adds no window another user can use where the directory is the person's own. A world-writable install directory is already compromised. |
| A local user who can write the data directory | Change `update-check.json` to announce any version | The prompt and notice show only a parsed semver (no terminal escapes); "Update now" looks the release up again and installs what GitHub says, verified. At worst a misleading prompt. |
| The environment | Redirect the lookup | A release build has no endpoint variable (D16); proxies come from `network.proxy` and the system, as every other request. |
| The release answer itself | Inject text, paths or links | Versions parsed; names matched exactly; the entry read by its exact path (no path traversal, no symlink, a size bound against decompression bombs); the release-notes URL built, not taken. |
| Logs | Leak a token | The token is a sensitive header, never logged or printed; logs carry error kinds at `debug` only (D20). |

macOS and Windows specifics: a file the updater writes carries no
quarantine attribute (only browsers and similar applications set
`com.apple.quarantine`) and no Mark of the Web, so Gatekeeper and
SmartScreen do not assess it at first run; this is the same as `curl` or
`cargo install` today. The arm64 binary keeps the linker's ad-hoc signature
(the archive's bytes are written unchanged), and the rename gives the new
file its own inode (D5). Nothing is codesigned or notarized; the docs keep
saying so.

## 6. The questions, answered

The user answered every question on 2026-11-14, each as recommended. Each
entry keeps the question and records the decision; none is open.

- **Q1 — The check on by default?** It sends one request a day to GitHub (a
  third party, like models.dev, which is off by default). The user asked for
  an automatic check. **Decided by the user on 2026-11-14, as recommended:
  on**, with the setting, the variable and the statement of §3.12.
- **Q2 — Cargo installations.** Run `cargo install --locked
  hanafubuki-agent-cli` for the person (Codex runs its package manager), or
  only name it? On Windows cargo is expected not to replace the running
  executable (U1 confirms it on Windows CI; if it does replace it, cargo
  runs there too); moving it aside first (as D5 does) and putting it back if
  cargo fails would work but adds the Windows rename path and a Ctrl-C
  window around a build of several minutes. **Decided by the user on
  2026-11-14, as recommended: run it on Unix, name it on Windows** (no
  prompt there, the notice and `update --check` name it).
- **Q3 — After an update, exit or re-execute?** **Decided by the user on
  2026-11-14, as recommended: exit** (D9).
- **Q4 — Bare binaries or a checksums file in releases?** **Decided by the
  user on 2026-11-14, as recommended: neither** (D4, D3); `release.yml` is
  unchanged.
- **Q5 — The notice's place.** The footer's right side, a status-line item,
  an info line in the transcript, or a row of its own? **Decided by the user
  on 2026-11-14, as recommended: the footer's right side** (D12).
- **Q6 — The interval.** Every launch, 20 h (Codex), or a day? **Decided by
  the user on 2026-11-14, as recommended: 20 h**; a person who starts it
  daily sees a release within a day, and it is one request a day.
- **Q7 — Retrying a Windows rename on a sharing violation.** An antivirus
  scan of the file just written commonly holds it for a moment (the error is
  32, "used by another process"); without a retry the update fails and the
  person retries by hand. About ten lines: up to ten attempts over two
  seconds, only for errors 5 and 32, only on Windows. **Decided by the user
  on 2026-11-14, as recommended: yes.**
- **Q8 — Signatures (U4).** **Decided by the user on 2026-11-14, as
  recommended: later**, minisign over each archive (§5); not a blocker for
  U0–U3. It needs a key pair, a secret and a release, and the first version
  that verifies can only check the release after it. U4 stays optional and
  is not started without a further decision.
- **Q9 — The notice for a dismissed version.** **Decided by the user on
  2026-11-14, as recommended: shown** (Codex), only the prompt respects the
  dismissal.
- **Q10 — Debug builds never contact GitHub** (D16), so `update --check`
  from a debug build needs the test URL. **Decided by the user on
  2026-11-14, as recommended: yes**; the release build is the one people
  run.
- **Q11 — `cargo install --locked`** in the updater and in `installation`.
  **Decided by the user on 2026-11-14, as recommended: yes**: the released
  crate's lock file, as the binaries are built (`release.yml:205`).
- **Q12 — A crate, or a module of the CLI?** The engine has one consumer. A
  module in `crates/cli` saves a crate on crates.io and a layer row, but its
  tests would be white-box modules of a binary crate or reach it only
  through the process, and nothing would prove the facade never links it.
  **Decided by the user on 2026-11-14, as recommended: the crate** (D2).

## 7. Milestones

Each is one commit on main with green CI on Linux, Windows and macOS, and
records in `todo.md`: what landed and any deviation, the validation commands
run and skipped, the CI run's URL (no Windows or macOS claim from a Linux
run), whether goldens changed (none: nothing reaches a model request), what
Hanafubuki must change at its next pin (nothing: the facade is untouched,
and it uses neither the TUI nor `AgentTuiPreferences`), and any module-size
exception (none expected). Every milestone runs AGENTS.md's matrix
(`cargo fmt --all --check`; `RUSTFLAGS="-D warnings" cargo check --workspace
--all-targets --all-features`; `cargo clippy --workspace --all-targets
--all-features -- -D warnings`; `cargo xtask check`; `cargo test --workspace
--all-features`) and, since each changes pages, the site's (`bun install
--frozen-lockfile`, `bun run format:check:site`, `bun run site:check`, `bun
run site:test`, `VITE_BASE_PATH=/hanafubuki-agent/ bun run site:build`,
every page prerendered).

### Lanes

```text
now ──┬── lane A: U0 ──────────► U1 ───────────────┐
      │                                            ├──► U3 ──► (U4, if decided)
      └── lane B: U2 (TUI, config, docs) ─► U2 (CLI, after U0) ┘
```

- **Launch now, in parallel: U0 (lane A) and U2 (lane B).** U2 starts with
  what needs nothing of U0 (the setting, the notice's data and rendering,
  the footer, the docs) and writes its CLI part against §3.2's signatures;
  it commits only after U0 is on main, rebased onto it. U0 is a few hundred
  lines and lands first.
- **When U0 lands: U1** (lane A), in parallel with the rest of U2.
- **When U1 and U2 have landed: U3.**
- **U4** only after a further decision (Q8: later, optional).

U0 takes every shared edit, so the lanes do not meet: the workspace
`Cargo.toml` and `Cargo.lock` entry of the new crate, the `semver` move, the
`LAYERS` rows (both the new crate's and the CLI's: they are adjacent lines),
and the CLI's dependency on the crate. After it, U1 and U2 share only: the
module list in `crates/cli/src/main.rs` (`mod update;` and `mod
update_check;`, adjacent: the second to land rebases), `Cargo.lock` (U1's
archive crates; U2 adds none), and documentation tables where each adds a
row in a different place (`environment`, `network-logging`).

- **U0 — the crate, the release lookup, versions, installation kinds and
  actions**
  (§3.2–§3.6, D2, D3, D8, D16, D19).
  - Files: `Cargo.toml` (the workspace dependency `hanafubuki-agent-update`
    with `path` and `version`; `semver = "1"`), `Cargo.lock`,
    `crates/packages/Cargo.toml` (`semver.workspace`),
    `crates/update/Cargo.toml` (new), `crates/update/src/{lib.rs, error.rs,
    version.rs, target.rs, client.rs, release.rs, install.rs, action.rs,
    test_support.rs}` (new) with `src/{version,release,install,action}/tests.rs`
    (new), `crates/update/tests/{lookup.rs, release_contract.rs}` (new),
    `crates/cli/Cargo.toml` (the dependency; `test-support` in
    dev-dependencies), `xtask/src/dependencies.rs` (the rows).
  - Tests: versions (§3.4's table, whitespace, build metadata);
    `this_target` and the names (§3.5); the contract test; over the fake
    release: the request's headers (`Accept`, the User-Agent, a token only
    when given), unknown fields ignored, a bad asset skipped, each status's
    error, a draft, a pre-release, a bad tag, a non-https asset URL, a body
    over 1 MiB, a lookup timeout; `for_this_build` in a debug build without
    the variable errs and makes no request; `UpdateProxy::from_setting` and
    its redacted `Debug`; the kinds (§3.6) from temporary trees: a profile
    directory, a `.crates2.json` with a registry source, with a path
    source, without this binary, unreadable; a symlink resolves to its
    target; `writable` true, and false in a read-only directory (Unix,
    skipped as root); each kind's action (§3.6), Unwritable,
    NoReleaseForPlatform and CargoOnWindows included, and the cargo command
    with `--root` only off the default.
  - Docs: `architecture`, `plugin-rules`, `releasing` (the contract),
    `environment` (the test URL), both languages.
  - Acceptance: `cargo xtask check-dependencies` passes with the new rows;
    the lookup decodes the live v0.2.0 answer saved as a fixture; no test
    reaches the network but the fake server; U2 can word the notice from
    `Installation::action()` alone.
  - Hanafubuki: nothing (a new crate it does not use).
- **U1 — the engine and `hanafubuki-agent update`** (§3.7, §3.8, D4–D7,
  D9, D17, D18, D20, D21).
  - Files: `crates/update/src/{updater.rs, download.rs, archive.rs,
    verify.rs, lock.rs, cleanup.rs, cargo.rs}` and
    `crates/update/src/replace/{mod.rs, unix.rs, windows.rs}` (new) with
    their tests, `crates/update/src/{lib.rs, error.rs, test_support.rs}`
    (the engine's errors; archives for the fake release),
    `crates/update/tests/running_copy/main.rs` (new, `harness = false`), `crates/update/Cargo.toml` (`tar`, `flate2`,
    `zip`, per platform; `tokio` features), `Cargo.lock`;
    `crates/cli/src/{cli.rs, main.rs}` (`Command::Update { check, force }`
    and its arm), `crates/cli/src/update/{mod.rs, install.rs}` (new: the
    command; the install flow with its progress lines, which U3 reuses),
    `crates/cli/tests/update.rs` (new).
  - Tests (the crate, Unix fixtures with a shell script as the executable
    where a real binary is not needed): checks never download or write;
    `--force` never downgrades and reinstalls an equal version; a held lock
    stops before the download; an installed file already at the latest is
    not downloaded; metadata errors before any download; digest mismatch,
    short and long bodies, a missing, symlinked or oversized entry, a
    failing, silent or wrong-version executable, each leaving the original
    and no staging directory; the token reaches the API and never the
    asset host; through a symlink the link stays and the target changes,
    its mode kept; stale staging removed under the lock; `run_cargo` with a
    fake `cargo` on `PATH` records its arguments and a working directory
    outside the project. The running copy, on all three systems
    (`tests/running_copy/main.rs`, Hanamori's `running_copy_child` pattern
    in a target of its own): its executable answers `--version` as
    `hanafubuki-agent <version>`; the test copies it into a directory with
    spaces and non-ASCII (`工具 bin/custom name[.exe]`), serves an archive of
    it, and runs the copy in its child mode, where it updates itself through
    the library with `force`: the copy is replaced while it runs (on
    Windows, the `.outdated` backup is left), the next `--version` works,
    and a second update removes the earlier backup. Not the CLI binary:
    without a `[profile]` section a debug `hanafubuki-agent` is about 1.1 GB
    (measured in `target/debug` on 2026-11-14), too heavy to archive, serve
    and hash in a test on three systems; U1 records the test target's size.
    The CLI suite (`crates/cli/tests/update.rs`): `update --help` and
    `--check --force` (2) with no side effects; `update --check` against a
    release claiming 99.0.0 prints §3.8's report; a debug build without the
    test URL fails with D16's sentence; on Unix, a fake cargo installation
    (`bin/` and `.crates2.json`) with a fake `cargo` on `PATH` is updated
    through it. On Windows, U1 also tries `cargo install` over a running
    copy and records whether it fails (D7, Q2).
  - Docs: `cli-reference` (`#update`, `#agent-start`), `installation`,
    `exit-codes`, `environment` (`GITHUB_TOKEN`), `network-logging`
    (`update`'s row), both languages.
  - Docs also: `internals` (the helper's flags are only added).
  - Acceptance: a running copy replaces itself on the three CI systems and
    every failure leaves the original intact; a release rehearsal
    (`releasing#rehearsing`) passes the `licenses` job with the new
    dependencies.
  - Hanafubuki: nothing.
- **U2 — the check, the cache, the setting and the notice** (§3.9, §3.10,
  §3.12, D10–D12, D15).
  - Files: `crates/config/src/tui.rs` (the field, the default, the spec)
    and its tests; `crates/tui/src/update_notice.rs` (new),
    `crates/tui/src/{lib.rs, interface.rs}` (re-exports;
    `TuiOptions::update_notice`, `TuiEvents::update_notifier`),
    `crates/tui/src/appearance/mod.rs` (`AppearanceEvent::UpdateNotice`),
    `crates/tui/src/app/preferences.rs` and
    `crates/tui/src/chatwidget/appearance.rs` (applying it),
    `crates/tui/src/bottom_pane/mod.rs` (`set_update_notice`),
    `crates/tui/src/bottom_pane/chat_composer/{mod.rs, render.rs}` (the
    field and the props), `crates/tui/src/bottom_pane/footer/{mod.rs,
    tests.rs}` and `footer/snapshots/` (rendering);
    `crates/cli/src/update_check/{mod.rs, cache.rs, check.rs}` (new) with
    `update_check/{cache,check}/tests.rs`, `crates/cli/src/main.rs` (`mod
    update_check;`), `crates/cli/src/interactive.rs`.
  - Tests: the setting's default, decoding and place in the settings
    center, and both reference rows (`check-config-reference`); footer
    snapshots at 120, 80 and 50 columns with the default status line
    (full, short, none), with a draft, with `← for agents`, and the
    shortcut overlay without it; the bottom pane's height is the same with
    and without the notice; a notice sent through the notifier appears in
    the footer (an `app/tests` case in a new file, `app/tests/mod.rs` gains
    one line); the cache (§3.9: each field alone, unknown keys kept, not an
    object quarantined, unreadable never written, due at 20 h and in the
    future, a dismissal written meanwhile kept); the check over the fake
    release (due: one request, cache written, notice sent when newer; not
    due: none; a failure, a pre-release, an older release: cache unchanged,
    nothing sent; no `Authorization` even with `GITHUB_TOKEN` set); whether
    it is allowed (the setting, the variable, a debug build, a checkout).
  - Docs: `terminal-interface` (`#layout`, `#updates`), `config-reference`,
    `data-directory`, `environment`, `network-logging`, both languages.
  - Acceptance: at launch the interface draws its first frame without
    waiting for the network; a newer release found by this launch's check
    appears in the footer without moving the composer; with the setting or
    the variable off, no request is made.
  - Module size: `bottom_pane/mod.rs` 549 → about 553;
    `chat_composer/mod.rs` 509 → about 515; `chat_composer/tests.rs` (869
    of 900) takes nothing: the footer's tests are in `footer/tests.rs`.
  - Hanafubuki: nothing. `AgentTuiPreferences` (in `hanafubuki-agent-config`,
    which it depends on) gains `check_for_updates`, defaulted; it builds no
    such value (searched 2026-11-14).
- **U3 — the prompt and "Update now"** (§3.11, D13, D14, D18's sweep at
  launch).
  - Files: `crates/tui/src/update_prompt.rs` (new) with
    `update_prompt/tests.rs` and `update_prompt/snapshots/`,
    `crates/tui/src/lib.rs`, `crates/tui/src/tui/input.rs` (dropping
    pending input at start, if `InputReader::start` cannot be reused as
    is); `crates/cli/src/update_check/{mod.rs, prompt.rs (new), check.rs}`
    (the decision, the dismissal, the Windows sweep in the thread),
    `crates/cli/src/interactive.rs`, `crates/cli/src/update/install.rs` (if
    the prompt's call needs a seam).
  - Tests: snapshots at 80×12 (a released binary; a cargo installation)
    and 40×12 with option 3 highlighted; keys (Enter on the default, Down
    Down Enter, Up wrapping, `j`/`k`, `1`/`2`/`3`, Esc, Ctrl-C, Ctrl-D, a
    release ignored); the loop over a channel and a `VT100Backend`: a
    `Redraw` redraws, a key queued before the screen is dropped, the
    viewport is clear afterwards; the decision (shown; not when dismissed,
    with a prompt argument, without an action, with the check off) and the
    dismissal keeping the other keys; on Windows, a `.outdated` beside a
    copied executable removed by the launch sweep.
  - Docs: `terminal-interface#updates` (the prompt), `cli-reference`
    (`#interface`), both languages.
  - Acceptance: with a newer version cached, the next `hanafubuki-agent`
    shows the prompt before the Agent starts; Skip and Esc start the
    interface; "Skip until next version" stops the prompt until a newer
    version; "Update now" prints `update`'s lines and ends, and the next
    start runs the new version (rehearsed by hand, §9).
  - Hanafubuki: nothing (the terminal interface is not its).
- **U4 — signed archives** (optional, later: Q8). `release.yml` signs each archive
  (`<archive>.minisig`), the crate verifies it with the compiled-in key
  before extracting, `releasing` documents the key and its rotation,
  `installation` the manual check. Not scheduled.

## 8. Risks

- **The first real self-update is the next release's.** A bug in version
  N's updater can only be fixed by updating N by hand once. The CLI suite
  replaces a running copy on all three systems in CI, and §9's rehearsal
  runs the whole path before the release that ships it.
- **GitHub's answer changes** (the `digest` field, the URL scheme): the
  updater refuses and says why; the manual path stays.
- **Windows file locks** beyond the retry: a clear message, nothing
  changed.
- **Running instances and the helper.** After any upgrade a running
  instance's sandboxed commands use the new binary as their helper; it
  fails closed on a command line it cannot read. From U1 on, the helper's
  flags are append-only across versions (noted in `internals`); a version
  that must break them says so in its release notes.
- **Privacy perception** of an on-by-default request: documented exactly,
  two ways off.
- **The new dependencies' licences**: accepted only when a rehearsal's
  `licenses` job passes; another licence would need `about.toml`.
- **crates.io**: the release that adds the crate creates one new crate;
  within the burst crates.io allows, so minutes, not hours.
- **Text-matching `release.yml`** in the contract test is brittle by
  design: it fails on any edit of the packaging lines, which is the point.

## 9. How to test by hand

The real path needs a release newer than the build. Until then, a local
rehearsal (Linux and macOS shown; Windows the same with PowerShell and
`.zip`):

1. Build "the new version" in a scratch worktree: set
   `[workspace.package].version` to `9.9.9` (not committed), `cargo build
   --release -p hanafubuki-agent-cli`, and package it as `release.yml:216-232`
   does: `hanafubuki-agent-9.9.9-<target>/hanafubuki-agent` in
   `hanafubuki-agent-9.9.9-<target>.tar.gz`.
2. Beside it write `latest.json`: `{"tag_name": "v9.9.9", "draft": false,
   "prerelease": false, "assets": [{"name": "<archive>", "size": <bytes>,
   "browser_download_url": "http://127.0.0.1:8765/<archive>", "digest":
   "sha256:<sha256sum>"}]}`, and serve the directory: `python3 -m
   http.server 8765 --bind 127.0.0.1`.
3. Take "the old version", a debug build of main, out of `target/` (a
   checkout build is never replaced): `install -m 755
   target/debug/hanafubuki-agent /tmp/hfa/bin/`.
4. With `HANAFUBUKI_AGENT_HOME=/tmp/hfa/home` and
   `HANAFUBUKI_AGENT_TEST_RELEASE_URL=http://127.0.0.1:8765/latest.json`:
   `/tmp/hfa/bin/hanafubuki-agent update --check` reports 9.9.9; the first
   interface start shows the notice once the check lands; the second shows
   the prompt; Skip until next version, then a third start shows none;
   delete `dismissedVersion` from `update-check.json`, start again, choose
   Update now: the lines of §3.8, then `/tmp/hfa/bin/hanafubuki-agent
   --version` prints `hanafubuki-agent 9.9.9`. On Windows, check the
   `.outdated` backup and that the next start removes it.
5. Break it on purpose: a wrong digest, a truncated file (`head -c`), the
   server stopped, `chmod 555 /tmp/hfa/bin`, a second instance holding the
   lock: each says what §4 says and leaves `--version` unchanged.

A release build cannot be pointed at a fake (D16). Against the real API,
`update --check` from a release build of main reports `0.2.0` as the latest
today; after the first release that contains the updater, the release after
it is the one to try `update` with, from a downloaded copy of the earlier
one.

## 10. Fit with other work

- **In flight:** M8 (`history_cell/tool/`, `diff_render`) and L3
  (`commands/providers/`, `settings_center/`) share no file with U0–U3.
- **O1–O3** (kernel, the facade's subagents, `/subagents`): no shared file.
- **R1–R8:** R2 adds a setting in `crates/config` and the TUI's waiting
  line; R8 touches `appearance/{items,mod}.rs` (U2 adds one
  `AppearanceEvent` variant there), `crates/config/src/tui.rs` (R8 changes
  `DEFAULT_STATUS_LINE`; U2 adds a field and a spec), `config-reference#tui`
  (different rows) and `terminal-interface` (`#status-line`, `#layout`).
  Whichever lands second rebases; no design conflict: R8's item is inside
  the status line and its lines are above the composer, the notice is the
  footer row's right side (§3.10).
- **The queue:** the U milestones need nothing from the others and can run
  beside them; the user places them. `agent_bridge/mod.rs` (598 of 600) and
  `app/mod.rs` (531) are not touched.
