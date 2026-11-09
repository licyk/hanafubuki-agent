# Plan: Codex's new terminal interface, its commands, and configuration inside the interface

Repositories: the Agent (`/root/code_workspace/hanafubuki-agent`, main
`feba8a1`, 0.2.0) and Codex (`/root/code_workspace/codex`, checked out at
`57a808a392`, 2026-11-07). Hanafubuki embeds the Agent through the facade
and does not depend on the TUI or CLI crates, so it is affected only by
facade, history and golden changes. Each phase lists its notes for
Hanafubuki.

The request has five parts:

1. Port Codex's redesigned TUI.
2. Port 34 slash commands with their full backends, as independent modules
   on Kardia, with fault isolation and usable from the library.
3. Add a provider configuration screen.
4. Add an MCP configuration screen.
5. Port Ctrl+T (transcript) and Esc (edit an earlier message).

## 1. What happens today (verified)

### 1.1 Codex's new TUI

- **Fullscreen mode is a second rendering path, not a separate program.**
  - Codex calls it "owned transcript", or Fullscreen. It lives inside the
    same `App` and `ChatWidget` as the legacy interface.
  - In Terminal mode, the legacy one that this repository ported, finished
    history cells go into the terminal's native scrollback (`insert_history`).
  - In Owned mode the app runs on the alternate screen. Every frame it
    repaints the retained `transcript_cells` through a viewport widget,
    `TranscriptView`.
  - `TranscriptView` handles scrolling, mouse and keyboard selection, F3
    search, F4 disclosure of activity, copy, "follow latest", and loading
    older history pages.
- **Choosing the mode.**
  - `transcript_mode.rs`: `TranscriptMode::resolve(fullscreen_transcript, alt_screen)`.
  - `[tui].fullscreen_transcript` defaults to true.
  - `[tui].alternate_screen` is `auto|always|never`; `auto` turns it off over SSH.
  - `/tui` writes the setting, which takes effect at the next launch.
  - The mode is fixed for the life of the process.
- **Revision.** None of this is in a stable release:
  - `app/owned_transcript.rs` is absent at `rust-v0.152.0`, the latest
    stable tag.
  - It is present at `rust-v0.169.0-alpha.2`, 59 commits before the
    checkout.
  - This repository's TUI is marked "Modified from OpenAI Codex
    rust-v0.93.0" (`architecture/notices-policy.toml`).
- **Layers a port adds to the legacy interface**, in dependency order, with
  approximate line counts excluding tests:

  | | Layer | Size | Notes |
  | --- | --- | --- | --- |
  | A | Terminal extensions | ~3k | `tui.rs` owned-screen state; `tui/{alternate_screen,keyboard_modes,event_stream,frame_requester}.rs`; mouse capture; OSC 8 hyperlinks in `custom_terminal.rs`. Needs ratatui's `unstable-widget-ref`. |
  | B | The `HistoryCell` trait | | Gates C. Adds `display_lines_for_mode`, hyperlink lines, `activity_ids`, `activity_disclosure`, `copy_source` and animation ticks, plus `ChatWidget`'s live-tail accessors. |
  | C | `transcript_view*` | ~7.9k | Anchored positions, layout cache, live tail, follow, selection, copy mode, select mode, search, disclosure. |
  | D | Owned app shell | ~3.2k | `app/owned_transcript.rs` (paint, event priority), `history_pagination`, `app/input.rs`, footer selection, link hover. |
  | E | Clipboard | | `arboard`, with OSC 52, tmux and WSL fallbacks; `markdown_copy`. |
  | F | Keymap | ~9.9k | Only Ctrl+T, F3, F4, select and the leader key matter to the new TUI; the rest serves `/keymap`. |
  | G | Pager and backtrack | ~3k | `pager_overlay/transcript.rs` and `app_backtrack*`. |
  | H | Workspace panes | ~3.1k | Sidebar, side pane, HUD, pane motion. Behind Codex's `tui_workspace` feature, which is off by default. |

- **Ctrl+T.**
  - Owned mode switches the same view to the detailed presentation in place
    (`app_backtrack.rs` `open_transcript_overlay`).
  - Terminal mode opens the alternate screen with a `TranscriptOverlay`
    over a clone of the cells. Each draw syncs in the live tail. New history
    is held in `deferred_history_lines` and written out on close.
- **Esc to edit.**
  1. The first Esc on an empty composer primes and shows a hint.
  2. The second Esc browses user prompts: ←/→ move between prompts, Ctrl+T
     shows details, Esc goes back.
  3. Enter sends `thread/revert {before_turn_id}`. That is an in-place
     truncation of durable history; files are not restored.
  4. After the revert the composer is refilled with the prompt and the cells
     are cut.
- **Not to port:** accounts, cloud, daemon, pets, voice, analytics,
  updates, the Desktop handoff, guardian and safety buffering, and the
  `ui_tree` footer extensions.

### 1.2 This repository

- **TUI** (`crates/tui`, about 23.8k lines):
  - 12 commands (`model mode skills new resume compact diff mention status mcp quit exit`).
  - Dispatch is a set of flat `match` statements:
    - `slash_command.rs`;
    - `chatwidget/commands.rs::dispatch_command`;
    - `AgentCommand` in `app_event.rs`;
    - `agent_bridge/commands.rs::run_command`;
    - `TurnUpdate`;
    - `ChatWidget::handle_command_update`;
    - `app.rs::handle_event`.
  - Commands take no arguments.
  - Ctrl+T is the legacy pager (`pager_overlay.rs`).
  - No backtracking, no syntax highlighting, keys hard-coded, no `[tui]`
    configuration.
- **TUI start.**
  - `interface.rs:77` opens a session before `tui::init()`.
  - `create_session` fails without a default provider
    (`agent/src/service/sessions.rs:39`), so the interface never draws.
  - `AgentBridge` and `ChatWidget` assume a session.
  - Every start creates a session (todo.md).
  - `providers add` never sets a model, and an empty `modelPolicy.model`
    gives a session that cannot run a turn.
- **Kardia.**
  - `kardia-core` and `kardia-macros` 0.2.0.
  - The nine registries in `hanafubuki-agent-contracts` all face the model
    (tools, sources, context, renderers, admission, prompt, approval rules,
    continuation hooks, observers). There is no extension point for an
    operation a host or the TUI calls.
  - A row can still provide a service under its own name
    (`Context::provide_arc_as`).
  - The facade can resolve that service per call with
    `ModuleHost::service::<T>(name)` (`crates/host/src/host/lookup.rs`). The
    call returns "the <name> plugin is unavailable: <reason>" while the row
    is pending, failed or disabled.
  - Nothing uses this yet.
  - A panic in plugin code is caught and restarts its row on the restart
    policy. A kernel panic rebuilds the instance.
- **Panic containment.**
  - The hook aborts on any panic that is not contained
    (`foundation/src/panic/hook.rs:56`).
  - The TUI's main thread is not contained, so today a panic in TUI code
    ends the process.
  - `hanafubuki_agent_foundation::panic::catch_panic` and
    `catch_panic_async` already enter a guard scope (a contained region)
    and catch the unwind (`foundation/src/panic/mod.rs:60`). The facade
    does not re-export them.
- **Standalone plugins.** `AgentRuntimeHost` (`agent/src/standalone/mod.rs:62`)
  takes no plugin rows, so a standalone feature plugin must be in the
  facade's manifest (`agent/src/host/manifest.rs`).
- **Already in the facade (`AgentApplicationService`):**
  - Sessions: `rename_session`, `archive_session`, `restore_session`,
    `delete_session_confirmed`, `fork_session_at_head`, and
    `fork_session_before_turn`, which returns the restored prompt and images.
  - History: `list_sessions_page`, `open_session`, `read_transcript_page`,
    `read_turn_page`.
  - Background terminals: `list_background_terminals` and
    `stop_background_terminals_confirmed`. Stopping is all or nothing.
  - Memory: `memory_status`, `reset_memory`, session contribution, and the
    `memory.*` settings through `write_settings`.
  - Providers: `provider_registry_snapshot`, `write_provider`,
    `delete_provider`, `set_provider_enabled`, `write_model_policy`,
    `list_provider_models`.
  - MCP: `mcp_snapshot`, `mcp_write_server`, `mcp_delete_server`,
    `mcp_write_credentials`, connect, disconnect and refresh, and
    `mcp_oauth_*`. Writes apply live.
  - Plugins: `plugin_status`, `set_plugin_enabled`, `retry_plugin`.
- **Partly there.**
  - **Per-session permission mode.** The kernel takes it per turn
    (`AgentTurnInput.permission_mode`), but the facade overwrites it from
    the settings (`agent/src/service/turns.rs:69,135`). The modes are
    `Manual`, `AutoReview` and `FullAccess`.
  - **Auto-review denials.** The kernel's approval auto-review emits
    `PermissionReviewCompleted { status: Denied, .. }`
    (`kernel/src/model/event.rs:112`). That is the input `/approve` needs.
  - **Working directory.** `StandaloneWorkspace` is fixed for the run.
    `WorkspaceFsPolicy` is built once in `open`. `exec_command` without
    `workdir` runs in `.` (`execution/src/shell.rs:68`), which is the `-C`
    bug in todo.md.
  - **One-shot structured model call.** It exists only inside the titles
    plugin (`agent/src/plugins/titles/generation.rs`, which uses
    `ModelStructuredOutputCapability` and falls back to forcing a tool).
- **Absent:** sandboxing, subagents, goals, user lifecycle hooks, code
  review, the worktree manager, themes and syntax highlighting, keymap,
  status-line, terminal-title and `[tui]` settings, and an OAuth loopback
  listener for MCP.
- **Tokens.** Provider tokens are kept in `providers.toml`, readable by the
  owner only. MCP credentials are kept in `mcp.toml` `[credentials]`. The
  keyring holds only MCP OAuth tokens.

### 1.3 The commands: where each one's work lives

The tier decides where a command's code goes (§2.1).

| Command | Codex backend | Here today | Tier |
| --- | --- | --- | --- |
| `/rename` | `thread/name/set`, plus a suggested title from a one-shot structured turn | `rename_session`; the suggestion is missing | B; suggestion in P8 |
| `/archive` | `thread/archive`, which also archives subagent children | `archive_session` (cascade comes with P10) | B |
| `/delete` | `thread/delete`, which also deletes subagent children | `delete_session_confirmed` | B |
| `/resume` | `thread/list`, `thread/read`, `thread/resume`; `/resume <id\|name>` | picker exists; no argument, no cwd or branch in summaries | B |
| `/fork` | `thread/fork`; can pick a worktree first | `fork_session_at_head` | B |
| `/clear` | clear the screen, `thread/start` | `/new` exists | C |
| `/copy` | local clipboard (arboard, OSC 52) | none | C |
| `/export` | Markdown from the hydrated thread | none | B (Markdown in the facade) |
| `/raw` | `[tui].raw_output_mode` | none | C |
| `/tui` | `[tui].fullscreen_transcript` | none | C |
| `/ps`, `/stop` | `thread/backgroundTerminals/list` and `clean` | facade has both | B |
| `/pwd`, `/cd` | `/cd`: idle check, rebuild configuration, trust, then fork into the new cwd | `/pwd` only needs the roots; `/cd` needs a mutable workspace | C / B+ |
| `/worktree` | `worktree` crate (`git worktree add --detach`), then `/cd` and fork | none | A |
| `/init` | a fixed user prompt (`tui/assets/prompt_for_init_command.md`) | none | C |
| `/recap` | one-shot structured turn (`context-fragments/src/recap_prompt.rs`) | none | B (P8) |
| `/review` | review sub-agent with `prompts/templates/review/rubric.md` and JSON findings | none | A |
| `/goal` | `ext/goal`: tools, continuation turn, budgets, SQLite | none; `ContinuationHookRegistry` has no user yet | A |
| `/side` (`/btw`) | ephemeral fork plus injected side policy (`context-fragments/src/side_conversation.rs`) | fork exists; ephemeral sessions do not | A |
| `/subagents`, `/agents` | multi-agent v2 tools (`core/src/tools/handlers/multi_agents_v2/`), thread tree, command center | none | A |
| `/hooks` | `hooks` crate (14 events, command/MCP/prompt handlers, trust by hash) and a browser | none | A |
| `/permissions` | presets of approval × sandbox, session only | three modes, but the facade overrides them | B |
| `/approve` | guardian denials, then inject an "approved action" note | denial events exist; no note | B |
| `/setup-default-sandbox` | Windows elevated sandbox setup | no sandbox at all | A (decision 4) |
| `/memories` | toggles and reset | facade has all of it | B |
| `/config` | `[tui]` preferences panel, opens the other editors | none | C |
| `/keymap` | `[tui.keymap.<context>.<action>]` | none | C |
| `/theme` | syntect, two-face, `.tmTheme` files, `[tui].theme` | none | C |
| `/title`, `/statusline` | `[tui].terminal_title` and `[tui].status_line` items, OSC title | none | C |
| `/presets` | `[tui.presets."0-9"]` `{input, label}`; replayed through the slash parser | none | C |
| `/warnings` | client-side list of warnings with copy and dismiss | sources exist: config recoveries, plugin warnings, MCP diagnostics, recorded panics | C |

## 2. Architecture of the port

### 2.1 Three tiers

- **A: an Agent capability.** The code is a Kardia plugin row in the
  facade's manifest. The facade exposes typed methods for it (§2.2).
  - The model-facing side uses the existing registries:
    - goal: tools and a continuation hook;
    - subagents: tools;
    - hooks: approval rules, continuation hooks and observers, plus a new
      extension point for tool events (§3.13);
    - review and side: prompt contributors.
  - When the plugin fails, only its feature is unavailable.
  - Hosts reach it through the facade, like everything else.
- **B: an operation the facade already owns or should own.** It is a method
  on `AgentApplicationService`, not a plugin. It needs no extension point,
  because it is the application's own state: sessions, history, settings,
  background terminals.
  - These are the "independent modules" of the facade:
    `service/<area>.rs` files.
  - Each method runs on the Agent runtime, which is contained.
- **C: presentation in the terminal.** It is a module in `crates/tui`.
  - "Kardia plugin" does not apply: the TUI is a client of the facade, and
    rule 6 keeps kardia out of it.
  - What is shareable still goes through the facade:
    - the `[tui]` preferences are configuration keys the facade reads and
      writes (§2.5);
    - the Markdown export is a facade method.
  - So another frontend over the library can use them too.
- **Standalone only, on purpose.** Two features exist only in a standalone
  run, because an embedded host owns what they change:
  - `AgentRuntime::set_working_directory` (`/cd`, `/worktree`): the host
    supplies the workspace through its own `WorkspaceRootsPort`;
  - `/tui`'s launch mode: it chooses how the terminal interface draws.
  Every other feature is reachable through `AgentApplicationService`
  wherever the Agent is embedded.

### 2.2 Feature plugins and the facade's feature calls

- **The pattern**, documented in plugin-rules "Host, configuration and
  status" and architecture "Faults":
  - A tier-A plugin provides one service under its row name
    (`ctx.provide_arc_as(NAME, Arc<FeatureService>)`, declared in its
    `provides`).
  - The facade resolves it per call through
    `ModuleHost::service::<T>(NAME)`. It never keeps the `Arc`, because a
    retried row provides a new instance.
  - A call goes through one helper, `AgentApplicationService::feature::<T>`.
    It maps unavailability to `AgentError::FeatureUnavailable { feature, reason }`
    and runs the service call under `PluginFaults::guard_async`. A panic
    then becomes that row's fault and restart, and the caller gets an
    error. The rest of the Agent is untouched.
- **Where a plugin's code lives.**
  - A plugin that needs the application's own services lives in
    `crates/agent/src/plugins/<name>/`, as titles, memory and skills do.
    Examples are a session driver, history or settings. It reaches the
    kernel through a narrow port the application constructs, in the
    style of `AgentTitleApplication`.
  - A self-contained domain is a domain crate plus a `crates/plugin-<name>`
    crate. Hooks and worktrees are of this kind.
  - Service traits a `crates/plugin-*` crate provides are declared in
    `hanafubuki-agent-contracts`, because plugins may not depend on the
    facade.
- **Panic check.** `cargo xtask check-plugin-panics` is extended to
  `crates/agent/src/plugins/` (open item in todo.md D2). Every new plugin
  is covered either way.
- **Rule 4.** A new contracts registry is added only with its first user.
  The one this plan needs is tool lifecycle events for hooks (§3.13).

### 2.3 TUI command modules and their isolation

- **Registry.** A `commands/` module tree replaces the flat matches. Each
  command is one module implementing `SlashCommandModule`, which provides:
  - a spec: name, aliases, description, whether it takes inline arguments,
    and its availability (during a task, without a session, in a side
    conversation);
  - `dispatch(&mut CommandContext, args)`;
  - optionally, the views it pushes.
- **What the registry produces.** The popup, `find`, and the
  argument-taking dispatch (Codex's `slash_dispatch.rs` split) come from
  the registry. `AgentCommand` and `TurnUpdate` keep one generic variant
  per facade call family, not one per command.
- **Isolation.**
  - The facade re-exports the foundation's existing `catch_panic` as
    `hanafubuki_agent::panic::catch_panic`. The foundation twin does not
    change.
  - The TUI wraps three things in it: each command's dispatch, each pushed
    view's input handling, and each pushed view's render.
  - When a module panics:
    - its view is popped;
    - an error cell names the command;
    - the module is turned off for the rest of the run;
    - the session, the composer and the other commands keep working.
  - The chat widget's own core (composer, transcript, streaming) is not a
    module. A panic there still ends the process, after restoring the
    terminal. This limit is stated on the terminal-interface page.

### 2.4 Starting without a session, and first-run setup

- **No session at start.**
  - `AgentBridge` and `ChatWidget` hold `Option<SessionId>`.
  - The session is created by the first submitted message, or by a command
    that needs one.
  - The interface draws before any session exists.
  - This also closes "every start creates a session" in todo.md.
- **No provider or no model.**
  - With no default provider, or an empty `modelPolicy.model`, the
    interface opens the provider setup screen (§3.4) instead of failing.
  - The composer stays usable. Submitting a message reopens the setup
    screen.

### 2.5 `[tui]` preferences in the configuration

- **The section.**
  - `hanafubuki-agent-config` gets a typed `[tui]` section, in camelCase
    like the other keys:
    - `fullscreenTranscript` and `alternateScreen`;
    - `rawOutput`;
    - `theme`;
    - `statusLine` and `statusLineColors`;
    - `terminalTitle`;
    - `keymap.<context>.<action>`;
    - `presets."0".."9"` as `{input, label}`;
    - the `/config` toggles (§3.8).
  - It is read tolerantly per field like every persisted setting.
  - Unknown keys under `[tui]` are ignored, not rejected (AGENTS.md
    "Decoding by input source").
- **Facade access.**
  - The facade gains `tui_preferences()` and
    `write_tui_preferences(patch, expected_revision)`.
  - This is needed because only the facade may depend on config.
  - `config get|set` reaches the keys through the existing
    `AgentConfig::value` and `set_user_value` allow-list, extended to
    `tui.*`.
  - Custom themes live in `<home>/themes/*.tmTheme`.

### 2.6 Codex revision, notices, third-party crates

- **Pinned revision.** Ported code comes from Codex `57a808a392` (decision 1).
  - New and rewritten files carry a second marker, "Modified from OpenAI
    Codex 57a808a392 (Apache-2.0).", added to
    `architecture/notices-policy.toml`.
  - Untouched legacy files keep the `rust-v0.93.0` marker.
  - NOTICE names both.
  - Ported prompt assets carry the same attribution in a header comment
    where their format allows one. Otherwise they are listed in NOTICE.
- **New crates**, each checked against `about.toml` and listed in
  `THIRD-PARTY-LICENSES.txt` at release:
  - `syntect` and `two-face` (themes, highlighting; MIT/Apache);
  - `arboard` (clipboard);
  - `toml_edit` (comment-preserving writes, if the config crate does not
    already provide them).
  - `image` is not added: inline image previews are out of scope.
- **Module size.** Codex's files are 1–10k lines. Every port splits along
  responsibility, never into `part1`/`part2`. These are the likely
  exceptions, each a cohesive table or dispatch, reported in the handoff
  with its ceiling:
  - the keymap's default bindings table;
  - `transcript_view/text.rs`;
  - `render/highlight.rs`'s language table.

## 3. Design per area

### 3.1 The new TUI (P2)

- **What is ported.** Layers A–G of §1.1 go into `crates/tui`. Owned mode
  becomes the default whenever the alternate screen is available
  (decision 2). Terminal mode stays: `--no-alt-screen`,
  `alternateScreen = "never"`, SSH under `auto`, or `/tui`.
- **Couplings removed.** The pets and image protocol coupling in
  `transcript_view` is removed, and so is the `ui_tree` footer extension.
- **Ordering.**
  1. The `HistoryCell` trait extension goes first. Every existing cell
     (`history_cell/`, including the thinking cell) implements the new
     methods.
  2. The pager, `TranscriptView`, and both modes are built on it.
- **Older history.** Older pages load through `read_transcript_page`, which
  is already cursor-based. A replayed tool call still lacks its subject
  (todo.md, 5b), which is unchanged here.
- **Layer H.** The workspace panes (sidebar, side pane) are ported with
  `/side` in P9, not here (decision 10).

### 3.2 Ctrl+T (P2)

- **Owned mode.** Ctrl+T toggles the detailed presentation of the same
  view. Esc or Ctrl+T returns.
- **Terminal mode.** The existing pager is replaced by Codex's
  `TranscriptOverlay`: a `TranscriptView` over the cells, with the live
  tail synced each frame and history held back until close. This closes
  "the transcript pager's live tail" in todo.md.

### 3.3 Esc to edit an earlier message (P3)

- **Interaction.** It is Codex's (`app_backtrack*`): prime, browse, then
  Enter, in both modes.
- **The mapping.** The visible user-prompt ordinal is mapped to a turn id
  from `read_turn_page`. Prompts that are not user turns are skipped:
  compaction, continuation, and the hidden review and side items of later
  phases.
- **Backend: decision 3.** The recommended path needs no history format
  change:
  1. Call `fork_session_before_turn(turn_id)`.
  2. Switch the interface to the fork.
  3. Refill the composer from `restored_prompt` and `restored_images`.
  4. Archive the original, which `/resume` can still restore.
- **Refusals.** It is refused while a turn runs, as in Codex. Files are not
  restored, which is also Codex's behaviour, and the hint says so.
- **Facade.** A host gets the same operation as `edit_from_turn(session, turn_id)`,
  which forks, archives and returns the fork with its restored prompt.

### 3.4 Provider configuration screen (P1)

- **Commands.**
  - `/providers`, also reachable from `/config` → Providers, and at first
    run (§2.4).
  - `/model` keeps choosing the session's model.
- **List view.** Shows the default, enabled state, protocol, base URL and
  whether a token is set. Keys:
  - Enter edits;
  - `a` adds;
  - `d` deletes, after a confirmation;
  - `e` enables or disables;
  - `*` makes the provider the default.
- **Form view.** Fields:
  - display name;
  - protocol (`responses | chat_completions | anthropic_messages`);
  - base URL;
  - token, masked as `•••x`, with its own kill buffer so a secret cannot be
    yanked into another field;
  - request headers, which also fixes `providers add` taking no headers
    (todo.md 4b).
- **Test.** The form has a "Test" action that lists the models of the
  unsaved draft. It shows the model count, or the provider's error.
- **After saving.** A model picker from `list_provider_models` writes
  `write_model_policy` with the default provider and the model. That
  removes the empty-model trap.
- **Writes.** Every write sends `expected_revision` from the snapshot taken
  when the screen opened. A conflict reloads the snapshot and keeps the
  form's values.
- **Facade addition.** `test_provider_draft(AgentProviderDraft) -> AgentProviderModelCatalog`,
  the same probe as `list_provider_models` on a draft that is never saved.
- **Widgets ported.**
  - single-line and masked text fields (`textarea/single_line.rs`, and
    `onboarding/provider_setup.rs`'s `FieldDisplay`);
  - a multi-field form view (`app/provider/editor.rs` and `editor/form.rs`);
  - `SelectionSecondaryAction` on `ListSelectionView`.
  - `bottom_pane`'s `push_view` becomes public to the command modules.
- **Tokens.** They stay in `providers.toml` (decision 8).

### 3.5 MCP configuration screen (P1)

- **Commands.**
  - `/mcp` opens the manager (decision 9).
  - `/mcp verbose` prints today's report.
  - `/mcp login <id>` starts OAuth.
- **List view.** Shows phase, tool count and last error. Keys: add, edit,
  delete, enable or disable, connect or reconnect (`mcp_connect` or
  `mcp_refresh`), OAuth login and logout.
- **Form view.**
  - Transport: stdio (program, arguments, cwd) or streamable HTTP (URL,
    OAuth policy).
  - Environment or header rows, each marked public or secret. A secret row
    writes a credential under a generated key `<server>-<NAME>` and
    references it from `credentialEnvironment` or `credentialHeaders`.
  - Common fields: enabled, required, approval policy, exposure policy,
    enabled and disabled tools, timeouts.
  - Saving calls `mcp_write_server`, then `mcp_write_credentials` with the
    whole set changed by one key. Both apply live.
- **Facade additions.**
  - `mcp_set_credential` and `mcp_remove_credential`: read, change, write.
  - `mcp_oauth_login(server, open_url)`, which:
    1. binds `127.0.0.1:0`;
    2. calls `mcp_oauth_start` with that redirect;
    3. hands the URL to `open_url`, which the TUI prints and tries to open;
    4. waits for the callback, with cancellation and a timeout;
    5. calls `mcp_oauth_complete`.
  - The listener is ported from Codex's `rmcp-client/src/perform_oauth_login.rs`
    and runs on the Agent runtime. This closes "OAuth login is edited in
    `mcp.toml`" (todo.md 4b), and the CLI gains `mcp login`.
  - Server ids are validated on read, which fixes the todo.md D2 item.

### 3.6 Session commands (P4, tier B)

- **`/rename [name]`.** A prompt view filled with the current title; an
  argument renames directly. The suggestion comes in P8.
- **`/archive` and `/delete`.** Each asks for confirmation. After it, the
  interface returns to a fresh composer with no session (§2.4). The cascade
  to child sessions comes in P10.
- **`/resume [id|title]`.**
  - The picker gains search, the archived filter (Ctrl+A archives or
    restores), the Ctrl+T preview, and the cwd and branch columns.
  - `AgentSessionSummary` gains `cwd` and `branch`, recorded at session
    creation as tolerant new fields. That closes the todo.md 5b item.
- **`/fork [name]`.** Uses `fork_session_at_head`. The worktree choice
  comes in P7.
- **`/clear [name]`.** Clears the terminal and drops to no session.
- **`/export [path]`.**
  - The facade gains `export_session_markdown(session) -> String`, built
    from the transcript pages. The format is Codex's
    (`app/transcript_export.rs`).
  - The TUI copies the result to the clipboard or saves it to
    `<cwd>/hanafubuki-agent-session-<id>.md` or to a given path.
- **`/copy`.** Copy targets from the last answer: the whole answer, each
  code block, each quote. In owned mode it uses selection copy instead.
- **`/ps` and `/stop`.** `/ps` lists terminals with recent output. `/stop`
  (alias `/clean`) calls `stop_background_terminals_confirmed`.
- **`/pwd`** (alias `/cwd`). Prints the workspace roots.
- **`/init`.** Submits the ported init prompt, which writes AGENTS.md and
  does not overwrite an existing one.

### 3.7 `/raw` and `/tui` (P2, tier C)

- `/raw [on|off]` sets `HistoryRenderMode::Raw`. It reflows the transcript
  and saves `tui.rawOutput`.
- `/tui` offers Scrollback or Fullscreen and saves
  `tui.fullscreenTranscript`, which applies at the next launch.

### 3.8 Preferences (P5, tier C over §2.5)

- **`/theme`.**
  - `render/highlight.rs` comes with syntect and two-face's grammars and
    themes.
  - Live preview, restored on cancel.
  - Custom `.tmTheme` files are read from `<home>/themes`.
  - This also gives the markdown and diff renderers syntax highlighting,
    closing the todo.md 5b item.
- **`/keymap [debug]`.**
  - The action catalogue is cut down to the actions this interface has.
  - It uses Codex's resolution (context, then global, then default),
    conflict detection, chords and the leader key.
  - The hard-coded keys (`pager_overlay.rs`, `app.rs`,
    `bottom_pane/textarea/keys.rs`) move into the keymap.
- **`/title [items|--clear]` and `/statusline [items] [--colors|--no-colors]`.**
  - Each has a setup view with a live preview, and arguments apply to this
    session only.
  - The items are model, reasoning effort, current directory, git branch,
    context remaining or used, tokens, session id, session title and task
    progress. Codex's account and rate-limit items are dropped.
  - The title is set with OSC 0/2 after stripping control and bidi
    characters (`terminal_title.rs`).
- **`/presets`.**
  - Slots 0–9 run their input through the normal slash parser.
  - Keys: `n` new, `s` save from the current settings (`/model`,
    `/permissions`, `/title`, `/statusline`), `e` edit, `d` delete.
  - Input is non-empty, at most 8,000 bytes, and does not start with `!`.
- **`/config`.**
  - Toggles and choices for `[tui]`: animations, tooltips, paste-burst,
    status-line colors, copy-on-select, resume cwd.
  - Entries open the other editors: providers, MCP, memories, theme, status
    line, title, keymap, TUI mode.
- **`/warnings`.**
  - A list of warnings, each with copy and dismiss. The sources are:
    - `AgentConfig::recoveries()`;
    - the plugins' `warning`s;
    - MCP diagnostics;
    - `recorded_panics()`;
    - the TUI's own warnings, including a module turned off by §2.3.
  - Configuration warnings from the first read are kept for the interface
    (part of the todo.md D2 item).
- **`/memories`.**
  - Toggles for use and generate (`memory.use_memories`,
    `memory.generate_memories`).
  - The session's contribution mode.
  - Reset (`reset_memory`), after a confirmation.

### 3.9 Permissions (P6, tier B)

- **Per-session mode.** The facade gains `set_session_permission_mode`,
  held in the session's runtime state like `set_session_model`. Turns use
  it before the settings (`turns.rs:69,135`). This closes the todo.md 5b
  item.
- **`/permissions [mode]`.**
  - Offers Manual, Auto-review and Full access. Full access asks for a
    confirmation.
  - The choice lasts for the session only, as in Codex. The configured
    default stays `config set permissionMode`.
- **`/approve`.**
  - The TUI keeps the last 10 `PermissionReviewCompleted { Denied }` events
    with their summaries.
  - Choosing one calls the facade's `approve_denied_action(session, review_id)`.
  - That commits an "approved by the user" context note for the next
    sampling step, as a `ContinuationContext`-style kernel item, and starts
    no turn.
  - The retry still goes through auto-review, which now sees the note.
  - Model-visible: the note's text, so the goldens change (§5).
- **`/setup-default-sandbox`.** See decision 4 and P14.

### 3.10 Working directory and worktrees (P7)

- **Mutable workspace.**
  - `StandaloneWorkspace` becomes mutable behind a `parking_lot` lock.
  - `WorkspaceFsPolicy` is rebuilt from the new roots, not by an instance
    restart, so live sessions stay live.
  - `exec_command` without `workdir` runs in the primary root. This fixes
    the `-C` bug (`shell.rs:68`).
  - The facade gains `AgentRuntime::set_working_directory(path)`. It is
    refused while a turn, a compaction or a background terminal runs.
  - Embedded hosts own their `WorkspaceRootsPort` and are unchanged.
- **`/cd [path]`** (default `~`).
  - Resolves the path through `foundation::paths`.
  - Applies the project trust level of the new directory. Trust prompts use
    the existing `projects.*.trustLevel`.
  - The session continues. Its summary's `cwd` is updated, and a context
    note tells the model the directory changed.
- **Worktrees.**
  - A new crate `hanafubuki-agent-worktree` provides create, list, owner,
    bind and remove, under `<home>/worktrees`.
  - It runs `git worktree add --detach --no-checkout`, then
    `reset --hard`.
  - Every git process goes through the Agent's `ExecutorManager`
    (AGENTS.md, Processes).
  - The crate is wrapped by a `plugin-worktree` row, so a failure costs
    only `/worktree`.
- **`/worktree`.**
  - Continue the current conversation (fork, then `/cd` into a new
    worktree).
  - Start a new one there.
  - Browse: resume the owner, copy the path, remove.
  - `/fork` offers the same worktree choice inside a git repository.

### 3.11 One-shot structured requests: `/recap` and the title suggestion (P8)

- **Shared request.** The titles plugin's request moves into a shared
  `structured_request` module, with the native schema or forced-tool
  fallback, a timeout and cancellation. The titles plugin, recap and
  rename all use it.
- **`/recap`.**
  - The facade gains `recap_session(session) -> AgentRecap { summary, next_action }`.
  - The input is the last 8 user and assistant exchanges, with no tool
    output, byte-bounded.
  - The prompt is ported from Codex's `recap_prompt.rs`.
  - It runs in a `recap` plugin row, so a failure costs only the command.
  - The TUI shows a loading cell, then the recap cell.
  - Automatic recaps are left out (decision 13).
- **Title suggestion.** The facade gains `suggest_session_title(session)`,
  shown in `/rename`'s prompt as it arrives.

### 3.12 Child sessions, `/side`, subagents, `/agents`, `/review` (P9–P11)

- **Child sessions** (P9, the foundation for side, review and subagents):
  - A session record gains `parent: Option<SessionId>` and `origin`
    (`side | subagent | review`). Both are tolerant fields; an old record
    has no parent.
  - A session can be ephemeral: kept in memory and never written to the
    history store.
  - A narrow port the application constructs, `AgentSessionDriver`, lets a
    plugin create a child, submit a turn, wait for it, interrupt it, and
    read its last answer.
  - Archive and delete cascade over children.
- **`/side [message]`** (alias `/btw`).
  - Creates an ephemeral fork with Codex's side policy, a developer
    instruction plus a boundary message, both model-visible.
  - Only one side conversation per parent.
  - Shown in the side pane in owned mode (layer H, ported here), and as a
    full switch with a return key in terminal mode.
  - Closing it interrupts the side session and drops it.
- **Subagents** (P10).
  - An agent plugin `subagents` registers Codex's v2 tools: `spawn_agent`,
    `send_message`, `followup_task`, `wait_agent`, `list_agents`,
    `interrupt_agent` (decision 6).
  - The tools run on `AgentSessionDriver`, with Codex's orchestrator prompt
    as a prompt contributor.
  - Depth and count limits are taken from Codex.
  - The tools respect the parent's permission mode.
  - `/subagents` shows the status feed and switches the interface to a
    child.
  - `/agents` is the command center: sessions and their children, status
    filters, and new, resume, fork, archive, delete and interrupt, all over
    facade calls.
- **`/review [instructions]`** (P11).
  - A `review` plugin starts a child review session.
  - The target is uncommitted changes, a base branch (merge-base), a
    commit, or custom instructions.
  - Base instructions are Codex's `rubric.md`. Tools are limited to
    read-only effects, and mutating calls are denied by an approval rule
    scoped to review sessions.
  - Findings are parsed as JSON into `AgentReviewOutput` and rendered as a
    review cell. Codex's exit messages go back into the parent.
  - The git queries (branches, recent commits, merge-base) are a small
    module in the worktree crate, through the executor.
  - The facade gains `start_review(session, target)`.

### 3.13 `/goal` and `/hooks` (P12–P13)

- **Goal** (P12, an agent plugin `goal`).
  - The goal state is objective, status (`active | paused | blocked |
    budget_limited | complete`), token and time budget, and usage.
  - It is stored as session metadata in the history store (decision 5),
    so fork, archive and delete carry it.
  - Model-facing parts:
    - tools `get_goal`, `create_goal`, `update_goal`;
    - a context contributor for the user's goal;
    - a continuation hook that keeps an active goal going within budget.
      It is the first user of `ContinuationHookRegistry`.
  - The templates are Codex's `continuation`, `budget_limit` and
    `objective_updated`.
  - Facade: `goal(session)`, `set_goal`, `clear_goal`, `pause_goal`,
    `resume_goal`.
  - `/goal [objective|clear|pause|resume|edit]` comes with a status-line
    item.
- **Hooks** (P13).
  - A domain crate `hanafubuki-agent-hooks` reads Codex's hook
    configuration: `[hooks]` in `config.toml` layers and `hooks.json` in
    the home and the project (decision 12).
  - Trust is by hash of the hook's definition, in `hooks.state.<key>`.
  - Handlers are command, MCP and prompt.
  - Commands run through the `ExecutorManager`, bounded by a timeout.
  - A `plugin-hooks` row maps the events:
    - `PreToolUse` to a new contracts extension point,
      `ToolLifecycleHookRegistry`, deny-only and ordered like approval
      rules. This is the extension point's first user.
    - `PostToolUse` and `Stop` to continuation hooks or the same registry.
    - `UserPromptSubmit` and `SessionStart` to turn admission and context.
    - Notifications to observers.
  - Hook progress is a new `AgentEventKind::Hook{Started,Completed}`
    (`ContinuationHook*` already exists and is reused where it fits).
  - Facade: `hooks_list(cwd)`, `set_hook_enabled`, `trust_hooks`.
  - `/hooks` is Codex's browser: by event, with source, trust and handler
    type, and enable, trust and trust-all actions.
  - Untrusted hooks are reported at start through `/warnings`, not by a
    blocking prompt.

### 3.14 Sandbox (P14, decision 4)

- **Proposed.** Codex's `sandboxing`, `linux-sandbox` (landlock and bwrap),
  macOS seatbelt and `windows-sandbox-rs` are ported as
  `hanafubuki-agent-sandbox`.
- **The execution plugin** spawns through it when a session's preset asks
  for a sandbox.
- **`/permissions` presets** then combine a mode with a sandbox: read-only,
  workspace-write, or none.
- **`/setup-default-sandbox`** appears on Windows when the non-elevated
  sandbox is active. It runs the elevated setup and writes
  `windows.sandbox`.
- **Size.** This is the largest single item: a security subsystem on three
  operating systems, sequenced last so nothing else waits on it.
  - Codex's production sources are about 37.6k lines: `sandboxing` 4.6k,
    `linux-sandbox` 4.7k, `windows-sandbox-rs` 24k and
    `windows-sandbox-service` 4.3k.
  - A port that keeps Linux and macOS and the Windows elevated setup would
    add on the order of 25–35k lines.
  - It would also need the Agent's `ExecutorManager` to spawn through the
    sandbox, which touches the twinned `hanafubuki-agent-process`.

## 4. Crates and modules

| New | Kind | Depends on | `LAYERS` row (`xtask/src/dependencies.rs`) |
| --- | --- | --- | --- |
| `hanafubuki-agent-worktree` | domain | foundation, process | domain |
| `hanafubuki-agent-plugin-worktree` | plugin | contracts, worktree | plugin (facade dependency) |
| `hanafubuki-agent-hooks` | domain | foundation, process | domain |
| `hanafubuki-agent-plugin-hooks` | plugin | contracts, hooks | plugin |
| `hanafubuki-agent-sandbox` (P14) | domain | foundation, process | domain; `execution` may depend on it |
| `crates/agent/src/plugins/{recap,subagents,review,goal}` | agent plugins | the facade's own ports | n/a |

- **Every crate** gets an entry in the root `[workspace.dependencies]` with
  `path` and `version`, a `description`, and a row in its manifest table.
- **Facade services** go in new files under `crates/agent/src/service/`:
  `features.rs` (the §2.2 helper), `export.rs`, `review.rs`, `goal.rs`,
  `hooks.rs`, `worktree.rs`, and `workspace.rs` for `/cd`.
- **TUI modules** are `crates/tui/src/commands/<command>.rs`, plus
  `transcript_view/`, `keymap/`, `render/highlight.rs`, `clipboard/`, and
  `config_views/{providers,mcp,config_panel,memories,hooks}.rs`.

## 5. What the model sees: goldens and the history corpus

- **New model-visible text**, each regenerated on purpose
  (`HANAFUBUKI_AGENT_UPDATE_GOLDEN=1`, AGENTS.md "Plugins"):
  - the `/approve` note;
  - the `/cd` directory note;
  - the side policy and boundary;
  - the subagent tools and orchestrator prompt;
  - the review rubric and exit messages;
  - the goal tools, context and templates;
  - hook context and continuation fragments.
- **No golden change:** `/init` is a user message, and recap and title
  requests are side requests.
- **History corpus.** A history format change (`parent`, `origin`, `cwd`,
  `branch`, goal metadata) is read tolerantly. A new fixture is recorded
  deliberately, in both repositories (todo.md "Shared with the other
  repository"). The existing corpus must still open.

## 6. Phases

Each phase:

- is validated with the AGENTS.md "Validation" commands for its scope,
  plus a pty run of the interface where it touches the TUI;
- updates its site pages in both languages;
- adds its todo.md entry with done notes and Hanafubuki notes;
- is committed and pushed.

**Delegation.** The main agent splits each phase into subagent tasks with
a narrow brief: files to read, contracts to keep, tests to add. It then
reviews the diffs and runs the final validation itself.

- P0 has two independent halves: P0a for the TUI and P0b for the facade.
- Independent backends run in parallel in separate worktrees once P0b is
  merged: P7's worktree crate, P8, P12's goal plugin, and P13's hooks
  crate.
- TUI work runs in sequence, because those phases share `chatwidget`,
  `bottom_pane` and `app`.

| Phase | Content | Site pages (en + zh) |
| --- | --- | --- |
| P0a | TUI: pin the Codex revision and notices (§2.6). Add the command registry, with per-module isolation over the re-exported `catch_panic` (§2.3). Make the bridge sessionless and create the session lazily (§2.4). The existing 12 commands move into modules with no change in behaviour. | terminal-interface, architecture |
| P0b | Facade: the feature-call helper and its docs (§2.2); `check-plugin-panics` over `crates/agent/src/plugins/`; the `[tui]` section and facade preferences (§2.5); the `catch_panic` re-export. | plugin-rules, architecture, internals, config-reference, embedding |
| P1 | The form widgets; the provider screen with first-run setup; `test_provider_draft`; the MCP manager and form; credential helpers; the OAuth loopback with `mcp login`; validated MCP ids. | providers-models, mcp, terminal-interface, cli-reference, quick-start |
| P2 | The new TUI, layers A–G; owned mode by default; Ctrl+T in both modes; `/tui`; `/raw`; clipboard. | terminal-interface, architecture |
| P3 | Esc to edit, plus `edit_from_turn`. | terminal-interface, sessions, embedding |
| P4 | `/rename /archive /delete /resume /fork /clear /export /copy /ps /stop /pwd /init`; summary `cwd` and `branch`. | terminal-interface, sessions |
| P5 | `/theme` (with highlighting), `/keymap`, `/title`, `/statusline`, `/presets`, `/config`, `/warnings`, `/memories`. | terminal-interface, config-reference, memory |
| P6 | Per-session permission mode, `/permissions`, `/approve`. | permissions, terminal-interface, embedding |
| P7 | Mutable workspace, the `-C` fix, `/cd`; the worktree crate and plugin, `/worktree`, the worktree choice in `/fork`. | terminal-interface, architecture, tools, data-directory |
| P8 | Shared structured request; `recap` plugin; `/recap`; title suggestion. | terminal-interface, plugin-rules |
| P9 | Child and ephemeral sessions, `AgentSessionDriver`, cascading archive and delete; `/side`; layer H. | sessions, architecture, internals, terminal-interface |
| P10 | `subagents` plugin; `/subagents`; `/agents`. | tools, plugin-rules, terminal-interface |
| P11 | `review` plugin; `/review`. | tools, terminal-interface |
| P12 | `goal` plugin; `/goal`. | tools, plugin-rules, terminal-interface |
| P13 | Hooks crate and plugin, `ToolLifecycleHookRegistry`, hook events; `/hooks`. | plugin-rules, configuration, config-reference, json-events, terminal-interface |
| P14 | Sandbox; presets in `/permissions`; `/setup-default-sandbox` (Windows). | permissions, architecture, configuration |

**Dependencies between phases:**

- Everything depends on P0b. Every TUI phase also depends on P0a.
- P3 depends on P2.
- `/worktree` in P7 uses P4's `/fork`.
- P9 is needed by P10 and P11.
- P11 uses P7's git module.
- The `/presets` "save from settings" action in P5 gains `/permissions`
  once P6 lands.
- P14 changes `/permissions` from P6.

**Verification by the main agent at each phase:**

- the validation commands, reported exactly as run (AGENTS.md);
- a pty run against the fake Responses server that covers the phase's
  commands;
- `cargo xtask check`, whose output records any new module-size exception
  or strict-decoding entry; each is then reported in the handoff;
- for the facade, Hanafubuki notes in todo.md listing every addition and
  any change to goldens or the corpus.

## 7. Decisions

All accepted as recommended (2026-11-09).

1. **Codex revision:** pin the checked-out commit `57a808a392`
   (recommended), or `rust-v0.169.0-alpha.2`. No stable tag has the new TUI.
2. **Default interface:** owned (fullscreen) when the alternate screen is
   available, as in Codex (recommended); Terminal mode stays selectable.
3. **Esc to edit:** fork before the turn, switch, and archive the original
   (recommended: no history format change, and the original can be
   restored). Alternatives:
   - fork and keep both visible;
   - add an in-place `revert_session_before_turn` to the history store,
     which is Codex's semantics and costs a new truncation commit plus a
     corpus change.
4. **Sandbox and `/setup-default-sandbox`:** port Codex's sandbox as the
   last phase, P14 (recommended, because the request asks for the complete
   implementation). Its cost is in §3.14: about 37.6k lines of Codex
   source, roughly 25–35k after the port. Alternatively, drop the command and keep
   `/permissions` to the three modes. Either way the command does not
   appear before a sandbox exists.
5. **Goal persistence:** session metadata in the history store
   (recommended: it follows forks, archives and deletes), or a state file
   the plugin owns.
6. **Subagent tools:** Codex's v2 set only (recommended), or v1 and v2.
7. **`/permissions` scope:** session only, as in Codex (recommended), or
   also saved as the default.
8. **Provider tokens:** stay in `providers.toml`, masked on screen
   (recommended). Moving them to the keyring is a separate change.
9. **`/mcp`:** opens the manager, and `/mcp verbose` prints the report
   (recommended).
10. **Workspace panes:** ported with `/side` in P9 (recommended), or
    deferred, leaving `/side` as a full switch.
11. **TUI preferences:** `[tui]` in `config.toml` through the facade
    (recommended), or a separate `tui.toml` the TUI owns.
12. **Hook configuration:** Codex-compatible (`[hooks]` and `hooks.json`,
    trust by hash) (recommended), or a format of our own.
13. **Automatic recap** (on focus loss or after turns): left out
    (recommended), or ported with a `[tui]` toggle.

## 8. Not ported

- **Codex products:** ChatGPT login and accounts, cloud and remote
  workspaces, the daemon, `/app`, `/usage`, `/logout`, `/feedback`,
  `/rollout`, `/daybreak`, `/pets`, `/voice`, `/ide`, `/import`,
  `/plugins`, `/apps`, `/experimental`, `/vim` (not requested), and the
  debug commands.
- **Analytics and updates:** analytics, telemetry, update prompts, the npm
  registry.
- **Guardian:** the guardian reviewer model, used for Codex's `/approve`.
  Ours approves over the kernel's own auto-review (§3.9).
- **Inline image previews** in the transcript, and the `ui_tree` footer
  extensions.
- **Platform clipboard fallbacks beyond OSC 52** for tmux, WSL and Herdr.
  This is optional and can be added if the pty runs show a need.
