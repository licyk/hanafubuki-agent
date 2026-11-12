# Plan: a registry-driven settings center, Codex's command center, and a conversation timeline with thinking and tool output

Repositories, read from source on 2026-11-12:

- The Agent: `/root/code_workspace/hanafubuki-agent`, main `6d61cc8`.
- Codex: `/root/code_workspace/codex`, `57a808a392` (2026-11-07), the
  revision the TUI port and our files' "Modified from" headers name. Paths
  below are under `codex-rs/tui/src/` unless they say otherwise.
- KohakuTerrarium: `/root/code_workspace/KohakuTerrarium`, `c1d17b45`
  (2026-11-09), the revision `plan-modular-architecture.md` read.

The request has nine parts. Numbers in brackets refer to them throughout:

- [1] Redesign `/config` after Codex: layout, interaction, missing settings,
  every editor inside it.
- [2] Make settings registry-driven, after KohakuTerrarium: each module
  declares its settings and the center renders them without per-module code.
- [3] Port Codex's command center, which opens with Left.
- [4] Expand and collapse conversation content with the mouse.
- [5] Thinking as persistent, per-segment conversation entries. The status
  line becomes `Working 25s (Thinking 20.7s)`.
- [6] `+ Show details` / `− Hide details`, and command output shown in the
  conversation.
- [7] `exec_command` output without garbled text.
- [8] Visible, collapsible, streamed output for every tool, formatted per
  tool.
- [9] No layout jitter when tool calls appear.

This plan **supersedes** two earlier ones:

- `docs/plan-configuration-center.md` and its handoff. Their C1–C6 are done
  and committed (`e4dbb98`, C6's documentation, is the last). This plan replaces the hand-written
  forms they built.
- The thinking presentation that `todo.md`'s "visible thinking" plan
  (2026-11-09, T2/T3) accepted:
  - thinking is written once into scrollback;
  - Ctrl-O expands every thinking at once;
  - the status line stays `Working`.

  §4.5 changes all three; the model-API and history parts of that plan stay.

Every §8 decision (D1–D17) was accepted as recommended (2026-11-12).

The `todo.md` section "Plan: a registry-driven settings center, the command
center, and the conversation timeline" tracks the milestones.

## 0. Terms

- **Fullscreen** and **Scrollback** are the two presentations
  (`transcript_mode.rs`: `TranscriptMode::{Owned, Terminal}`).
  - Fullscreen repaints the whole conversation every frame and owns the
    mouse.
  - Scrollback writes each finished entry once into the terminal's own
    scrollback, and cannot change it afterwards.
- **Entry** is one cell of the conversation: a user message, an answer, a
  thinking, a tool call. A **live entry** is still changing.
- **Activity id** is the string that names an expandable entry
  (`HistoryCell::activity_ids`, `tool:{call_id}` today). The view keeps the
  expanded set keyed by it (`transcript_view/disclosure.rs`).
- **Setting** is one user-configurable value with a key path
  (`memory.maxSourceAgeDays`).
  - A **section** groups settings under one heading.
  - A **collection** is a list of records edited one at a time: Providers,
    MCP servers, presets, roles, hooks.
  - A **page** is a section or collection as the center shows it.
- **Presenter** is the code that turns one tool call's arguments and result
  into what the interface shows. It is owned by the plugin that owns the
  tool, and it is not model-visible.

## 1. The request against what is there

Several premises of the request turned out differently in the code. Each one
changes the design.

| Request | What the code says | Consequence |
| --- | --- | --- |
| [1] Port Codex's `/config` | Codex's `/config` (`config_panel/`) is smaller than ours. It is a bottom-pane picker over 19 hard-coded `[tui]` rows in 5 tabs, saved at once to the user file. MCP, permissions, models, feature flags and memories are separate commands, and it has no mouse support. | Port its **presentation and interaction**: tabs, label · value · description rows, search over every setting, saving at once, per-row hints, narrow-width fallback. Do **not** port its data model; [2] replaces it. |
| [2] Adopt KohakuTerrarium's registration | Only KohakuTerrarium's *module options* are schema-driven (`option_schema()` plus one module record shape). Its app settings (providers, MCP, keys) are hand-written in each of its six UIs, and it has no generic collection editor. | Take its principles (§3.5), extend our existing `PluginOptionSchema` vocabulary, and **design** the collection model ourselves. Neither reference has one. |
| [3] Codex's Agent Center opens with Left | It is Codex's "Agent command center" (`agents_overview`). Plain Left opens it on an empty composer, and so does `/agents`. Ours (`crates/tui/src/agents_center/`) ports the list and five actions. | Port the entry key, live refresh, details pane, root-only rows, search, rename and grouping. Adapt or drop what has no backend here (§4.4). |
| [4] Click to expand, as in Codex | Codex expands by click **only in Fullscreen**. So do we: `transcript_view/input.rs` `pointer_down` → `toggle_disclosure_at`. Scrollback writes lines that cannot change in either program. | Both presentations get it. Fullscreen gets it inline. Scrollback keeps its recent entries in a repainted tail above the composer, where they change and expand in place, and writes an entry to scrollback only when it leaves the tail (§4.11). Older entries expand in the Ctrl-T transcript. |
| [5] Thinking "displayed separately at the bottom" | Each reasoning item already has its own `ThinkingCell`, persisted as `ConversationItem::Reasoning` and replayed on resume. The "bottom" display is that cell while live, drawn as the live tail below everything committed (`chatwidget/mod.rs:434-456`, `live_tail.rs`). It moves into the conversation when it completes. It has no activity id, so it cannot be focused or clicked. | Make thinking an in-place entry from its start (§4.5). Persistence needs no format change. |
| [5] "Thought · 20.7s" as in Codex | Codex has **no** such label on any ref (`git log --all -S'Thought'` finds nothing). Its reasoning is hidden from scrollback and shown only in the detailed view. | The thinking timeline is a **new design**, not a port. |
| [6] Command output, as in Codex | Our TUI receives no streamed output. The bridge drops `ExecutionProgress` (`agent_bridge/mod.rs:575`, `_ => return None`). A tool's output is read from history after it completes, on the UI thread. | A typed event path from the Agent to the cell (§4.6). |
| [7] Port Codex's `exec_command` to fix garbling | Codex's `exec_command` path decodes with `String::from_utf8_lossy` only (`core/src/unified_exec/output.rs:28`, `async_watcher.rs:468`, `process.rs:313`). Its encoding detector `bytes_to_string_smart` serves the older shell path and the app server, not unified exec. Our UTF-8 carry is already better than Codex's: per stream, and never split between polls. | **Porting Codex would not fix it.** Codex sends Windows PowerShell exactly our prefix (`shell-command/src/powershell.rs:12`). The case you reported, Windows PowerShell 5.1 (D10), has causes neither program handles (§4.9): mixed encodings in one stream, PowerShell 5.1's own decoding, progress bars under ConPTY. Reproduce them on CI's Windows runner, then fix our pipeline in place. |
| [8] `read_text` | The tool is `read_file` (`plugin-workspace-fs/src/tool.rs`). Its result has a **byte** `offset`, not a line. | Line numbers need a start line (D9). |

## 2. What exists today (verified)

### 2.1 The configuration center and the other editors

`crates/tui/src/config_center/` (2,086 lines) is one full-screen workspace
overlay (`app/workspace/mod.rs:157`, `config: Option<ConfigCenter>`).

- **Sections.** `enum ConfigSection` has 13 variants, and each is described
  by five hand-written matches: `parse`, `key`, `label`, `command`,
  `summary` (`mod.rs:25-172`).
- **Forms.**
  - `forms/mod.rs` builds each section's form in one 200-line match
    (`section_form`, 234-431), over 26 `const FieldId` strings and choice
    arrays that copy enum orders (50-89).
  - `forms/draft.rs` reads each form back into a typed patch, repeating the
    config crate's bounds (e.g. `number(form, "max_source_age_days", 1,
    3650)` against `validate_memory`'s `1..=3650`).
- **Plugin options are the only generic form.** `options.rs` renders one from
  `PluginOptionSchema`. Because `FieldId = &'static str`, it pre-declares
  `"option_0"`…`"option_63"`.
- **Saving.** A save is synchronous: `app/workspace/config.rs:92-112`, then
  `runtime().block_on` in `agent_bridge/settings.rs:211-265`.
- **"Shadowed by a higher layer"** is a hand-written field-by-field
  comparison after writing (`forms/mod.rs:121-216`). A third copy exists
  for `[tui]` in `app/preferences.rs:175-220`.
- **Linked editors are not embedded.** `ConfigAction::Replay("/mcp")` closes
  the center and replays the command (`app/workspace/config.rs:77-81`). The
  editor is a bottom-pane view with asynchronous `CommandReply` routing
  (`commands/mod.rs:231-240, 433-442`), and it does not return to the
  center.

The separate editors are bottom-pane views, each with its own state machine:

| Editor | Where | Backend |
| --- | --- | --- |
| `/mcp` | `commands/mcp/` (~1,370 lines) | `agent_bridge/mcp.rs`, revision-gated |
| `/providers [policy]` | `commands/providers/` (~1,480) | `agent_bridge/providers.rs`, revision-gated |
| `/plugins` | `commands/plugins/` (~1,660); package settings as raw JSON (`browser.rs:290-311`) | `agent_bridge/plugins.rs` |
| `/hooks` | `commands/hooks/` (~690) | hooks-crate state file, not `config.toml` |
| `/theme`, `/statusline`, `/title` | `appearance/` | `App::save` → `write_tui_preferences` |
| `/keymap`, `/presets` | `keymap/`, `presets/` | `App::save` |
| `/tui`, `/raw` | `commands/tui.rs`, `app/preferences.rs` | `App::save` |
| `/memories` | `commands/memories.rs` (291) | `SettingsCall::SetMemory` (a second memory writer) |
| `/config quick` | `commands/config.rs` (286) | `SetCompactionStrategy` (a second compaction writer) |

`/permissions` and `/role` change the session only. They are not settings
and stay commands.

### 2.2 How one setting is known today

One `config.toml` key is written in up to ten places:

1. The struct field, its serde rename and its `Default`.
2. The decode key string and validator (`settings/decode.rs`,
   `standalone.rs:106-127`, `tui.rs:189-244`).
3. The patch field and `apply`, or a hand-written `document()`
   (`standalone.rs:34-46`, `tui.rs:138-184`).
4. The TUI `FieldId`, label and choices.
5. The TUI drafter, with its bounds.
6. The shadow comparison.
7. The overview summary and display-name helpers
   (`config_center/mod.rs:129-172, 446-491`).
8. `FORM_KEYS` in the coverage test (`config_center/tests.rs:12-59`). That
   test compares strings with serialized leaves and never builds a form.
9. The tables of `config-reference.en.mdx` and `config-reference.mdx`.
10. Sometimes `agent_settings_fields()` (`config/src/settings/schema.rs`), a
    second, JSON-shaped schema for 4 web keys. It is exposed as
    `AgentSettingsView.web_search_fields`, and **Hanafubuki's settings page
    reads it**.

Two existing pieces already point the right way:

- **The CLI.** `config get/set` (`config/src/keys.rs`) needs no per-key code.
  It derives keys from the serialized defaults and checks a write by
  inserting the value, re-decoding, and refusing any new repair
  (`check_setting`, 76-101).
- **Plugin options.** `PluginOptionSchema` (`packages/src/options/`) is a
  typed vocabulary: Boolean, Integer, Number, Text, Choice and TextList, plus
  a tolerant `resolve` and a strict `check`. Facade row descriptors
  (`host/descriptors.rs`) and guest packages (`configSchema` lowering)
  declare it.

`layers::Resolved` keeps no per-key provenance. Only `[pluginOptions]`
records the layer of each value.

### 2.3 The command center

- `/agents` opens it (`app/workspace/agents.rs`) on the alternate screen.
- **Rows.** Sessions are listed with their children (subagents and reviews)
  as rows of their own. There are five filters (All, Running, Idle, Failed,
  Archived), stepped with Tab, Left or Right, or 1–5.
- **Actions:** open, new, fork, archive, delete (y/N inline), and interrupt.
- **Loading.** Data is read synchronously through `list_sessions` (pages of
  25, children 50) on open, on a filter change and after an action. Nothing
  refreshes it in between.
- **Status.** `WaitingForUserInput` is folded into Running, and approvals
  waiting are not distinguished.
- **Missing:** a Left-key entry, a keymap action, a footer hint and mouse
  support.

### 2.4 Thinking

- **Events.** `ReasoningStarted`, `ReasoningDelta` and `ReasoningCompleted
  { duration_millis }` arrive per reasoning item. Their `reasoning_id` is
  unique in the session.
- **The snapshot is dropped.** `ReasoningUpdated { AgentReasoningSnapshot {
  phase, active_since, step_completed_millis, turn_completed_millis } }`
  carries the turn's total thinking time, but the bridge drops it.
- **The cell.** `ThinkingCell { text, expanded, Live{started} |
  Completed{duration_millis, truncated} }` (`history_cell/thinking.rs`).
  - Its header is `Thinking 5s` or `Thought for 12s`, in whole seconds.
  - Live, it sits in `ChatWidget::live_thinking` and is drawn in the live
    tail.
  - On completion it is removed there and sent through
    `AppEvent::InsertHistoryCell` as an immutable `Arc<dyn HistoryCell>`.
  - Its `expanded` flag is frozen at that moment, and it has no
    `activity_ids`.
- **Persistence.** It is stored as `ConversationItem::Reasoning { text,
  duration_millis, sampling_step, truncated, replay }`, with no reasoning
  id. On resume, `chatwidget/replay.rs:73-82` rebuilds a completed cell.
- **The status indicator** (`status_indicator_widget.rs`) shows
  `Working (25s • esc to interrupt)`. It is hidden at the first committed
  answer line (`chatwidget/mod.rs:400,409`) and not shown again that turn.

### 2.5 Tool calls

- **One cell kind.** `ToolCallCell` (`history_cell/tool.rs`) serves every
  tool: a header (`Running` / `Called` / `Failed` + name + `: subject`),
  then an excerpt of the result text (first and last 5 lines past 10) or a
  patch's diff.
- **The subject is fixed per tool**, not taken from the call's arguments.
  It is `None` for every core tool, so an exec cell reads `• Running
  exec_command`.
- **Results are read after completion.** `ToolCompleted` makes the bridge
  run a synchronous `read_turn_page` on the UI thread (`agent_bridge/mod.rs
  :662-684`). It then reads `Text`, or the `output`/`exitCode` keys of
  structured output, in `replay.rs:147-185`.
  - `read_file`, `list_directory`, `search_text`, the web tools, memory,
    Skills and goals return structured JSON without those keys, so **they
    show nothing but the header**.
- **Live output never reaches the TUI.**
  - The execution plugin's monitor polls every 100 ms and reports
    `AgentToolProgressUpdate::ExecutionOutput` with an execution id and no
    call id (`plugin-execution/src/monitor.rs`).
  - The kernel coalesces these updates (`kernel/src/service/progress.rs`)
    and builds `AgentActivitySnapshot.output` (head, tail, omitted lines).
  - The TUI uses none of this.
- **The tool's invocation does know its call.** The progress sink is the
  invocation's own (`contracts/src/tools/handler.rs:101`,
  `invocation.progress`), so the kernel can stamp the call id.

### 2.6 Disclosure and the mouse

- **Disclosure state** lives in the Fullscreen view: `Disclosure { expanded:
  HashSet<String>, focused, live_ids }`, keyed by activity id.
- **Controls.** `+ N lines` / `− Show less` / `+ Show details` / `− Hide
  details` is a synthetic, non-selectable row (`transcript_view/text/mod.rs
  :78-121`). A left click on its label toggles it
  (`transcript_view/input.rs:298-333`). F4 focuses activities, and Enter,
  Left and Right toggle.
- **Re-anchoring.** A toggle re-anchors the reading position
  (`disclosure.rs:358-417`).
- **The live tail** is one joined layout with one combined control, so its
  entries expand together.
- **Scrollback** does not capture the mouse. Its Ctrl-T pager does, but it
  uses the detailed presentation, so it has no controls.

### 2.7 Why the conversation jumps

All seven sources were traced in `chatwidget/`, `transcript_view/` and
`tui/mod.rs`:

1. A running tool is a one-row header with no output and no control. At
   completion it gains up to 12 rows, a control and a separator. In
   Fullscreen the view is bottom-anchored while it follows, so everything
   above moves.
2. A live cell is removed at once, but its committed copy arrives through
   the event channel. A frame can be drawn with the entry in neither place.
3. The live tail is one joined layout. It is re-laid out whenever any live
   entry changes, including every 32 ms thinking tick and every 600 ms
   spinner tick.
4. The status row and the blank row under it disappear at the first answer
   line, which shrinks the bottom pane and so the conversation's area.
5. Reasoning and tool updates held while an answer streams (`turn.rs:37-55`)
   land in one burst at its end.
6. In Scrollback, the live region above the composer grows and shrinks.
   `Tui::draw` clears the viewport when its height changes
   (`tui/mod.rs:179-193`).
7. The synchronous history read on every `ToolCompleted` stalls the UI
   thread.

### 2.8 `exec_command`'s output

Output takes this path, all in `crates/process`, `crates/execution` and
`crates/foundation`:

1. **Reader threads** read 16 KiB at a time from pipes or the PTY. They
   carry incomplete UTF-8 per stream (`Utf8Carry`), and pass PTY output
   through `DaFilter`, which answers terminal queries.
2. **A ring buffer** keeps the newest bytes on character boundaries
   (`executor/output.rs`).
3. **Decoding** is lossy, chunk by chunk (`execution/src/service/output.rs
   :50`).
4. **`render_plain_text`** (`foundation/src/terminal_text.rs`) strips
   escapes and applies `\r`, backspace and CSI C/D/G/K.

Ranked by likelihood, these are the places where output can garble:

1. **Non-UTF-8 output becomes U+FFFD.**
   - It happens at `service/output.rs:50`, `terminal_text.rs:78` and
     `manager/run.rs:64`.
   - Nothing detects an encoding, and `cmd.exe` gets no code page
     (`execution/src/shell.rs:233-238`).
   - On Windows, a pipe child of `cmd.exe`, or a program that writes the
     ANSI/OEM code page (GBK/CP936 on a Chinese system), produces bytes that
     are not UTF-8. This is not reproduced on Linux.
2. **One cell per `char`.** `render_plain_text` counts one cell for every
   `char` and has no `unicode-width` (`terminal_text.rs:14-28, 177-207`).
   After CJK text, cursor moves land on the wrong columns, and CUP `H`,
   `J` and `X` are ignored. This affects PTY mode, ConPTY's redraws above
   all.
3. **Reads can start inside an escape sequence**, and leftovers like `31m`
   print as text. This happens at the monitor's skip
   (`plugin-execution/src/monitor.rs:236-246`), at ring trimming, and at the
   front trims of the progress bound and the kernel's merge.
4. **The hold-back is skipped** when the tail holds U+FFFD
   (`execution/src/service/unified.rs:96-102`), so half a sequence prints on
   the next poll.
5. **`DaFilter` never flushes at end of file.** Up to 256 bytes starting with
   ESC are lost.
6. **Output is lost without a flag.** A finished command returns only 64 KiB
   from its offset, and the session is then disposed (`unified.rs:86,118`).

The case you reported (D10) is Windows PowerShell 5.1:

- **Which PowerShell runs.** `resolve_shell` tries `pwsh`, then `powershell`
  (`execution/src/shell.rs`), so a machine without PowerShell 7 runs 5.1.
- **How it is invoked.** It gets `-NoLogo -NoProfile -NonInteractive
  -Command`, with a prefix that sets only `[Console]::OutputEncoding`.
  §4.9 lists what 5.1 does beyond that.
- **The environment.** The command gets a clean Windows environment of seven
  variables (`execution/src/environment.rs:71-81`). Codex inherits the whole
  environment by default (D17).

Already equivalent to Codex: the environment (`NO_COLOR`, `TERM=dumb`,
`LANG/LC_*=C.UTF-8`, pagers set to `cat`), the PowerShell UTF-8 prefix, the
ConPTY code and the terminal-query answers.

## 3. The references, and what to take

### 3.1 Codex `/config` (`config_panel/`)

- **A bottom-pane view** (`ConfigPanel`) over the shared `ListSelectionView`:
  - tabs per section (`selection_tabs.rs`);
  - a header `Configuration` with a dim subtitle (`Changes save
    automatically`, or the last save's notice);
  - a shaded panel surface;
  - `max_visible_rows` set to the largest section, so the height does not
    change between tabs.
- **Rows** are label · value (padded to 20) · description, single line, cut
  with `…`. The label column is as wide as the widest label.
  - Values read `[x] On` / `[ ] Off`, or `Label ›` for anything that opens a
    sub-page.
  - The selected row is `›` plus a filled highlight.
  - Disabled rows are dimmed, skipped, and their reason replaces the
    description.
- **Narrow widths.** Under 45 columns of description, the selected row's
  description wraps below the list.
- **Search.** `/` searches every section's settings, as one flat list
  matched on section, label, value, keywords and description.
- **Hints.** Each row has its own footer hint
  (`left/right group · / search · enter toggle · esc close`). The hint area
  is sized to the tallest hint, so the layout does not jump.
- **Saving at once.** Enter on a toggle saves immediately. A choice opens a
  child picker with `(current)`, and selecting saves and returns.
- **After a save:**
  1. reload the layers;
  2. adopt the value;
  3. apply side effects;
  4. say `Saved · overridden by config or system setting` when a higher
     layer wins.
- **Sub-pages** (theme with live preview, keymap, status line, title,
  providers) are views pushed on a stack. Esc pops one level.
- **No mouse.**

### 3.2 Codex's command center (`app/agents_overview*.rs`, `app/agent_center/`)

- **Opening.**
  - Plain Left on an empty, focused composer opens it, when no popup is
    open, no paste burst runs, and Left is still the editor's `move_left`
    and no other binding.
  - `/agents` opens it too. The remappable action `open_agents` has no
    default binding.
  - The footer shows `← for agents`.
- **Rows** are root threads only.
  - A root's group is the most urgent of itself and its descendants: Needs
    you, Working, Ready, Inactive.
  - Rows are grouped under headings with counts, by Project, Directory,
    Status or Model (`g` cycles them; the choice persists). Pinned rows come
    first, and a `Show more` row pages.
- **Details pane** (38 wide when the body is at least 90): model, reasoning,
  project, branch, usage, the most urgent descendant, the pending request,
  the latest activity, the last message and the prompt.
- **Keys:**
  - Tab and Shift-Tab change the filter, and Enter or Right opens.
  - `/` or F3 searches, `n` starts a new task and `w` a new one in a
    worktree.
  - `f` forks, `r` renames inline, `x` stops, `a` archives (confirmed),
    Backspace deletes (confirmed), `h` hides, `p` pins and `?` shows help.
  - Esc goes back.
- **Live.** Every thread notification updates the state and repaints, and
  changed threads are re-read in the background.
- **No mouse.**

### 3.3 Codex's conversation (`history_cell/`, `exec_cell/`, `transcript_view/`)

- **Disclosure** as ours: Fullscreen only, keyed by namespaced activity ids
  (`exec:`, `mcp:`, `patch:`, `plan:`), and anchored on toggle.
- **Exec cells** (`exec_cell/`):
  - **Header.** `• Running` / `Ran` / `You ran` plus the highlighted command
    (2 continuation lines, then `… +N lines`).
  - **Live output** is fed by output deltas into `LiveCommandOutput`. It
    keeps everything up to 1 MiB, then the first and last 50 lines, with
    long lines cut in the middle.
  - **Collapsed** it shows the last 3 output lines, or `(no output)`.
  - **Expanded** it shows `$ cmd` and the full output, then `✓` or `✗
    (code)` with the duration.
  - **Exploration.** Read, list and search commands group into one
    `Exploring` / `Explored` cell, with merged `Read a, b` rows.
- **Patches** (`diff_render.rs`):
  - a header `• Edited <path> (+A -R)`, or `Edited N files` with a row per
    file;
  - a right-aligned **line-number gutter**, a sign column, tinted add and
    delete backgrounds, and syntax highlighting;
  - `⋮` between hunks;
  - 3 rows per file when collapsed.
- **MCP:** `Called server.tool(args)` with a 3-line preview.
- **Web search:** one line, no details.
- **Plans:** `Updated Plan · X/Y complete`.
- **Stability:**
  - the active cell's layout is cached by `(cell, revision, width, mode)`;
  - the live tail is rebuilt only when its key changes;
  - a reader scrolled onto a live entry sees a pinned snapshot;
  - the hint area keeps its height.
- **Thinking:** none of [5] exists in Codex (see §1).

### 3.4 Codex `exec_command`

The same environment and PowerShell prefix as ours, plus lossy decoding and
byte-split head and tail buffers. Only its separate whole-buffer decoder
(`protocol/src/exec_output.rs:63-165`, `chardetng` plus `encoding_rs`, tests
for CP1251, CP866 and Windows-1252) is worth taking (§4.9).

### 3.5 KohakuTerrarium

What it does, at `c1d17b45`:

- **Modules return their options schema** (`option_schema()`, or
  `runtime_option_schema(self)` computed at runtime). Every editor lists
  modules as one record shape, `{type, name, description, schema, options,
  enabled}`, and picks the control from the field type.
- **Field vocabulary:** `type`, `default`, `doc`, `hint`, `values`,
  per-value `labels`, `aliases` (legacy values mapped forward),
  `disabled_values: {value: reason}` (computed per call from credential
  state), `min`/`max`, `max_length`, `item_type`/`max_items`.
- **Override-only persistence.** `{key: null}` removes one override and `{}`
  resets them all. Validation runs on the merged map.
- **A reported apply outcome** for drive settings: `APPLIED_LIVE`,
  `RESTART_REQUIRED` or `REJECTED`, with content-hash revisions.
- **A settings-section registry** in the web UI: `{id, label, group, order,
  keywords, visible(), badge, component}`. The web presentation layer also
  has `visible`, `advanced` and `wide` per field.
- **Its weaknesses, which we avoid:**
  - providers, MCP and keys are hand-written six times;
  - four validators drift apart;
  - field presentation leaks into the frontend by module name.

To take (the plan-modular-architecture adopted none of these):

1. **One vocabulary** for every setting, not only plugin options.
2. **Settings declared by their owner**, computed when the catalog is read,
   so availability (`disabled reason`) can depend on state.
3. **Per-value labels and disabled reasons**, aliases, and `hint` separate
   from the summary.
4. **Reset**: removing a key from the user layer is a first-class write.
5. **A reported outcome** per write: applied now, at the next session or the
   next launch, or shadowed by a layer.
6. **Sections with group, order and keywords**, and fields with `advanced`
   and `visible_if`, all declared in the schema rather than in the UI.

## 4. Design

### 4.1 One settings vocabulary (`hanafubuki-agent-foundation::settings`)

The vocabulary moves from `hanafubuki-agent-packages::options` to
foundation, because config, packages, contracts and the facade all depend on
foundation and on nothing above it. It keeps its tolerant `resolve` and
strict `check`. The packages crate keeps `configSchema` lowering, and guest
package options become ordinary settings.

The types (field names are the plan; the implementation may refine them):

```rust
pub struct SettingSpec {
    pub key: SettingKey,             // "memory.maxSourceAgeDays", "pluginOptions.subagents.x"
    pub label: String,               // "Maximum source age"
    pub summary: String,             // one line, shown as the row's description
    pub help: Option<String>,        // longer, shown below when selected
    pub kind: SettingKind,
    pub default: Option<SettingValue>,
    pub apply: SettingApply,         // Live | NextSession | NextLaunch
    pub advanced: bool,
    pub visible_if: Option<SettingCondition>, // { key, equals: SettingValue }
    pub commit_group: Option<String>,// fields saved together (searxng provider + URL)
    pub keywords: Vec<String>,
}

pub enum SettingKind {
    Boolean,
    Integer { min: Option<i64>, max: Option<i64> },
    Number  { min: Option<f64>, max: Option<f64> },
    Text    { maximum_chars: Option<u32>, secret: bool, nullable: bool },
    Path    { must_be_absolute: bool },
    Choice  { values: Vec<SettingChoice> },     // value, label, disabled: Option<String>
    TextList{ maximum_items: Option<u32>, maximum_item_chars: Option<u32> },
}
```

- **Aliases.** `SettingChoice` carries `aliases`, so a renamed value is read
  forward. That happens in the tolerant decode, never on write.
- **`PluginOptionSchema` becomes a list of `SettingSpec`** whose keys are
  relative to `pluginOptions.<row>`.
  - The facade's `AgentPluginDescriptor.options` keeps its role.
  - The guest lowering produces the same type.
  - `MAX_PLUGIN_OPTIONS` stays.
- **The web-settings schema goes.** `agent_settings_fields()` and
  `AgentSettingsView.web_search_fields` are removed. Hanafubuki reads the
  `web` section of the catalog instead (§4.2, D3).
- **Not added:** a `dict` kind. KohakuTerrarium's has no generic editor, and
  maps here are collections (§4.2).

### 4.2 The settings catalog: who registers what

The facade builds one `SettingsCatalog` every time it is read. Registration
is explicit, with no global registry. Each owner contributes through a
function or a descriptor it already exposes, and the center and the CLI read
only the catalog.

```rust
pub struct SettingsCatalog { pub sections: Vec<SettingsSection>, pub collections: Vec<SettingsCollection> }

pub struct SettingsSection {
    pub id: String, pub title: String, pub group: SettingsGroup, pub order: u16,
    pub file: SettingsFile,          // Config | Providers | Mcp (decides layering and the writer)
    pub settings: Vec<SettingSpec>,
    pub values: Vec<SettingState>,   // effective value, the layer that set it, issues
}
pub struct SettingState { pub key: SettingKey, pub value: Option<SettingValue>, pub source: SettingSource, pub issue: Option<String> }
pub enum SettingsGroup { General, Model, Tools, Memory, Context, Network, Interface, Plugins, Advanced }
```

Who contributes:

| Owner | What | How |
| --- | --- | --- |
| `hanafubuki-agent-config` | General, Sandbox, Memory, Web, Compaction, Model metadata, Network & logging, and the `[tui]` scalars (`AgentTuiPreferences`, which already lives in `config/src/tui.rs`) | One `fn section() -> SettingsSection` **beside each settings struct**: a new `settings/memory.rs` holds `AgentMemorySettings` and its section. The decoder is generic over the section's specs (§4.3), so the struct and its specs are the only per-setting code. |
| `hanafubuki-agent-config` | Providers (records), the model policy (a section of `providers.toml`), MCP servers (records) | Collections (below), beside `providers/` and `mcp.rs`. |
| Plugin rows (facade `host/descriptors.rs`) and guest packages | Their options | Their existing descriptors, now `SettingSpec`s. A row that has options is a section in group Plugins. |
| `hanafubuki-agent-tui` | Page overrides only: theme, status line, title, keymap, presets | Pages in its `PageRegistry` (below) for keys whose specs the config crate declares, so `write_values` checks every `tui.*` key against a spec it holds. The facade's `standalone_settings_view` already carries `[tui]`. |
| The facade | Plugins on/off, marketplaces, hooks, roles, profiles, project trust | Collections whose records it already exposes (`plugin_statuses`, `hooks`, roles, layers). |

**Collections.** Neither reference has one, so this is our design:

```rust
pub struct SettingsCollection {
    pub id: String, pub title: String, pub group: SettingsGroup,
    pub record: Vec<SettingSpec>,        // fields of one record, keys relative to it
    pub records: Vec<SettingsRecord>,    // id, label, summary, state badge, values
    pub actions: Vec<RecordAction>,      // id, label, scope: Collection | Record, confirm: bool
    pub revision: Option<u64>,           // revision-gated files (providers.toml, mcp.toml)
}
```

- **Actions.** A record action (`test`, `login`, `logout`, `set_default`,
  `list_models`, `trust`, `connect`) runs through the facade's
  `run_settings_action(collection, record, action)`. That dispatches to the
  owner that declared the collection, with the same calls `/providers` and
  `/mcp` make today.
- **Secrets.** A field with `secret: true` is never returned. A record says
  only whether it is set, and writing it goes through the owner's credential
  path (`mcp_set_credential`, the Provider's token store).
- **Page overrides.** A collection or section can name a **page override**
  (`page: Some("theme")`) when a generic form cannot express it:
  - the theme picker's live preview;
  - the status line's and title's ordered multi-select with preview;
  - keymap capture;
  - the plugin marketplace browser;
  - hook trust review.

  The TUI keeps a `PageRegistry` from that id to a page factory. Each page
  is registered by the module that owns it (`appearance/`, `keymap/`,
  `commands/plugins/`). The center does not name them; it asks the registry
  and falls back to the generic page.

### 4.3 Reading and writing generically (`hanafubuki-agent-config`)

- **Binding a spec to its struct.** A spec's key is the serde name path of
  a field in the typed settings struct (`memory.maxSourceAgeDays` is
  `AgentSettings.memory.max_source_age_days`, renamed `maxSourceAgeDays`).
  - The struct's `Default` is the only source of defaults: the section
    builder fills `SettingSpec.default` from the serialized `Default`, and
    no spec states one.
  - The tolerant decoder becomes generic. For each spec of a section it
    takes the value at the key from the layer's document, checks it with
    `SettingSpec::check`, and on failure records a repair and leaves the key
    out. It then deserializes the section's checked object into the typed
    struct, with `#[serde(default)]` filling every key left out.
  - Per-field tolerance and repairs therefore stay as they are, and the
    hand-written `read_member(…, "useMemories", …)` calls and per-field
    validators go. This is raw JSON at an entry boundary, checked in Rust
    before use, which AGENTS.md allows.
  - Rules across fields (`minSummaryLength ≤ maxSummaryLength`) stay in one
    `validate` per section.
  - Tests pin the binding: every serialized leaf of the defaults has exactly
    one spec, every spec names a leaf, and every default passes its spec's
    check.
- **Adding a setting after this plan.**
  1. The struct field, its serde name and its `Default` value.
  2. Its `SettingSpec` in the same file.
  3. Its row in `config-reference` in both languages
     (`check-config-reference` fails until it is there).
  4. The code that reads it.

  The center, the CLI, provenance, shadow notices and the coverage test
  need nothing. A key with its own page override (the keymap) also needs
  that page.
- **Read.** Effective values come from the resolved settings, serialized
  (the CLI's `known_settings()` path), and looked up by key.
  - **Provenance** comes from the layers themselves: `layers::resolve`
    records, per key path, the highest layer whose document sets it
    (`SettingSource::{Default, User, Profile(name), Project(path),
    Override}`).
  - This replaces the three hand-written shadow comparisons, and gives every
    setting the "set by" information plugin options already have.
- **Write.** A new `AgentConfig::write_values(SettingsWrite)` takes `Vec<(key,
  SettingWriteOp)>`, where an op is `Set(SettingValue)` or `Reset`.
  1. It checks each value against its spec (`SettingSpec::check`, strict).
  2. It inserts the values into the user document, or removes them for
     `Reset`.
  3. It re-decodes and refuses any new repair: `check_setting`, generalized
     to a batch, so a commit group is checked as one.
  4. It publishes atomically through `file.rs`, keeping comments and unknown
     keys.
  5. It returns `SettingsWriteOutcome { written, shadowed: Vec<(key,
     SettingSource)>, apply: Vec<(key, SettingApply)> }`.

  This becomes the one write path for the center, `config set` and the
  facade. AGENTS.md allows raw values at this boundary: they are validated
  in Rust before persistence.
  - `SettingsWrite` comes from the embedding host's frontend too (through
    `write_setting_values`), so it is strict (`deny_unknown_fields`). Its
    entry goes into `architecture/strict-decoding-policy.toml` (category
    and reason) in S2, as `check-strict-decoding` requires.
  - `write_values` covers `config.toml`'s keys, `[tui]` included.
    `providers.toml` and `mcp.toml` keep their revision-gated writers.
- **What it replaces.**
  - `AgentSettingsPatch`, `AgentStandaloneSettingsPatch` with its
    `document()`, and `AgentTuiPreferencesPatch` with its `document()` stay
    only where non-UI callers need typed writes (Hanafubuki's
    `write_settings`). Their scalar paths are re-expressed on
    `write_values`, so there is one writer.
  - The TUI drafters, `FORM_KEYS` and the two duplicate panels (`/memories`
    writes, `/config quick`) are removed.
- **Applying.** The facade applies after a write as today:
  `apply_settings_to_plugins`, then reconfigure. It reports a plugin that
  restarted instead of taking the change (`on_update` returned `false`) as
  `Restarted`, so the center can say what happened.
- **Collections write through their owners.** Providers and MCP keep their
  revision-gated stores. A stale revision reloads and keeps the edited
  record, as today.

### 4.4 The settings center (`crates/tui/src/settings_center/`, replacing `config_center/`)

**Presentation, after Codex:**

- **Full screen** (the alternate screen in Scrollback, as now). The header
  is `Settings` with a dim subtitle: `Changes save automatically`, or the
  last outcome (`Saved`, `Saved · project file overrides it`, `Saved ·
  applies at next launch`, `Could not save: …`).
- **Tabs** for the groups (General · Model · Tools · Memory · Context ·
  Network · Interface · Plugins · Advanced), each with the count of issues
  in it.
  - Left and Right switch tabs, and a click on a tab selects it.
  - The body keeps the height of the largest group.
- **Rows** are label · value · description, aligned.
  - A section with more than one setting gets a dim heading row.
  - A collection shows `N servers ›`, and a page override shows `Name ›`.
  - The value column shows `[x] On`, `[ ] Off`, a choice's label, a number,
    `••••` for a set secret, `(unset)`, and a dim `· project` when a layer
    above the user's sets it.
- **Selection** uses Codex's `›` and filled highlight. A disabled row shows
  its reason in place of the description.
- **Search.** `/` searches every setting and record across tabs, matching
  label, key, value, keywords and summary. A result shows `Group · Section`
  as its description.
- **Hints.** Each row has its own footer hint (`enter toggle`, `enter edit`,
  `enter open`, `r reset`), and the hint area is sized to the tallest hint.
- **Narrow widths** (under 45 columns of description): descriptions move
  below the list for the selected row.

**Interaction:**

- **Booleans and choices save at once** (Codex). Enter or Space toggles a
  boolean. A choice opens an inline list with `(current)` and disabled
  values with their reasons, and selecting saves.
- **Text, number, path and list fields** edit inline on Enter: a one-line
  editor, or a list editor of rows with add and remove. A second Enter
  saves, and Esc cancels.
  - A strict check failure stays in the editor with the reason.
  - A commit group opens as a small form with every member, saved together.
- **`r` resets** the selected setting: it removes the key from the user
  file, and the value then shows its source.
- **Collections** open a list page (records with summary and state, `a`
  add, `d` delete, the collection's actions). A record opens as a form built
  from `record` (generic) plus its record actions. Secrets are write-only.
- **Page overrides** open in the center. Esc returns to the row that opened
  them.
- **Mouse.** Wheel scrolls, a click selects a row, and a second click on the
  selected row activates it. A click on a tab selects it. The center owns
  the screen, so this needs no new capture.

**Hosting.** The center is a page stack in the workspace overlay: `trait
SettingsPage { render, handle_key, handle_mouse, on_reply, title }`.

- Every write and action is asynchronous through the existing
  `CommandReply` route, keyed by page id. The center's `block_on` calls
  go.
- The bottom-pane editors (`/mcp`, `/providers`, `/plugins`, `/hooks`,
  `/theme`, `/statusline`, `/title`, `/keymap`, `/presets`, `/tui`,
  `/memories`) are removed as separate views.
- Each command becomes a shortcut that opens the center at its page
  (`/mcp` = `/config mcp`), so habits keep working and nothing redirects
  out of the center (D2).
- `/config quick` is removed.

**What the center shows that it did not before** (the settings missing
today):

- Profiles (`profiles.NAME`), shown and edited as a collection of named
  layers; a profile's own keys reuse the specs.
- Project trust (`projects."PATH".trustLevel`), shown as a collection with a
  trust/untrust action.
- Roles (`agents.NAME`, role files), shown as a collection: list, fields,
  and open the file.
- Hook definitions (`hooks.EVENT` matcher groups), beside their trust and
  enabled state. Only the latter is editable today.
- Per-model overrides (`[providers.models."ID"]`) inside a Provider record.
- A plugin's MCP servers and tool filters
  (`plugins."P@M".mcpServers.NAME.enabled|enabledTools|disabledTools`).
- Package settings from the package's `configSchema` as generic fields; raw
  JSON only for a package without one.
- Marketplaces (`marketplaces.NAME`) as a collection.
- `modelMetadata.*`, already a form, now generic.

**The coverage test** replaces `FORM_KEYS`. For every leaf of the serialized
default settings (config, standalone, `[tui]`, `providers.toml`'s model
policy, an MCP server record), it asserts:

- a spec exists;
- the center builds a row or field for it;
- an untouched page writes nothing.

A second test checks that every spec's key appears in `config-reference`'s
tables in both languages, as an `xtask` check (`check-config-reference`).
This is a check, not generation (D4).

### 4.5 Thinking as conversation entries

**The entry.**

- A reasoning item becomes a transcript entry **when it starts**. It is not
  a live-tail cell that moves later.
- Its activity id is `thinking:{reasoning_id}` while the run lasts. On
  resume, the replayed item's id is `thinking:h{history index}`.
- Disclosure state is per run and never persisted, so the id needs to
  survive only live → completed. It does, because it is the same entry
  (§4.7).

**What it shows:**

```
• Thinking 3.4s                 live: shimmer, tenths of a second, ticks every 100 ms
  + Show details                once any text has arrived

• Thought · 20.7s               completed: dim bullet
  + Show details                when there is text; no control when the Provider sent none

• Thought · 20.7s (truncated)   cut at AgentHardLimits::maximum_reasoning_text_bytes
```

- **Expanded**, it shows the thinking as dim Markdown under the header,
  streaming while live.
- **Collapsed** is the default, and every entry is expanded independently
  (by click, or by F4 and Enter).
- **Ctrl-O** keeps one job: expand or collapse every thinking entry in the
  conversation at once (D6).
- **Each reasoning item is its own entry, in the model's order** with
  answers and tool calls. A step that thinks, calls a tool, thinks again and
  answers shows four entries.
- **Durations** come from `ReasoningCompleted.duration_millis`, formatted
  `{s:.1}s` under a minute and `1m 05s` above.
- **An item that never completes** (an error or an interrupt) is completed
  at the turn's end with the time so far, as today.

**Persistence.** None of this changes `ConversationItem::Reasoning` or the
history corpus.

- `chatwidget/replay.rs` builds the same entries from the stored items, so
  a resumed session shows every segment, completed.
- A session written before this change replays the same way, because the
  format is unchanged.

**The status line** shows `Working 25s (Thinking 20.7s) • esc to interrupt`
(the hint is D14).

- **Working** is the pause-aware turn timer, as today.
- **Thinking** is the turn's total reasoning time:
  `turn_completed_millis`, plus `now − active_since` while a reasoning item
  is open. That comes from `AgentReasoningSnapshot`, which the bridge
  forwards as `TurnUpdate::Reasoning(snapshot)` instead of dropping it.
- The parenthesis appears once the turn has thought, and the clock advances
  only while thinking is active (D5).
- The **status row stays visible for the whole turn**: it is no longer hidden
  at the first answer line. That removes jitter source 4.
- The final separator keeps `Worked for 25s` and gains `· thought for 20.7s`
  when the turn thought.

**Removed:**

- the live-tail placement of thinking;
- `ThinkingCell`'s frozen `expanded` flag;
- the footer's `ctrl + o to show thinking` hint, replaced by the entry's own
  control.

### 4.6 Tool output: presenters, events, per-tool views

**The presenter contract** (`hanafubuki-agent-contracts`, not model-visible):

```rust
pub trait AgentToolPresenter: Send + Sync {
    /// The header subject, from the call's arguments (the command line, the path).
    fn subject(&self, tool: &str, arguments: &serde_json::Value) -> Option<String>;
    /// What the interface shows for the finished call.
    fn display(&self, tool: &str, arguments: &serde_json::Value, output: &AgentToolOutputView) -> AgentToolDisplay;
}

pub enum AgentToolDisplay {
    Command   { command: String, cwd: Option<String>, output: String, exit_code: Option<i32>, duration_millis: Option<u64>, truncated: bool },
    Patch     { preview: AgentPatchPreview },              // today's PatchPreview, gaining line numbers
    FileRead  { path: String, start_line: Option<u64>, text: String, truncated: bool },
    Listing   { path: String, entries: Vec<AgentListingEntry>, truncated: bool },
    Search    { query: String, path: Option<String>, matches: Vec<AgentSearchMatch>, truncated: bool },
    Web       { kind: AgentWebDisplayKind, target: String, results: Vec<AgentWebResult> },
    Mutation  { verb: String, paths: Vec<String> },        // move, delete
    Text      { text: String },                            // MCP text, plugin tools, the fallback
    Fields    { rows: Vec<(String, String)> },             // structured output without a presenter
}
```

- **Registration.** A plugin registers its presenter for its tool names
  through a new `ToolPresenterRegistry` in the contracts. The registration
  is an effect of the plugin and is removed when it stops, like every other
  registry.
  - The execution plugin presents `exec_command`, `write_stdin` and
    `resize_execution`.
  - workspace-fs presents `read_file`, `list_directory`, `search_text`,
    `apply_patch`, `move_path` and `delete_path`.
  - web, MCP, memory, Skills, goal and compress present their own tools.
- **The fallback.** A tool without a presenter shows `Text` when its output
  is text, and `Fields` (the top-level scalar keys, then `…`) when it is
  structured. Every tool shows something [8].
- **Presenters return values and never panic.** They are covered by
  `check-plugin-panics`.
- **Presenters take the arguments as normalized against the tool's schema**,
  the same value the handler received, not the model's raw JSON. They decode
  what they need tolerantly, and a value they cannot read falls back to the
  fallback display.
- **A tool whose plugin is off or gone** shows the fallback when an old
  session is resumed.

**Events (facade, additive):**

- `ToolStarted` gains `subject` from the presenter, computed by the kernel
  from the arguments it already has. The live header then reads `•
  Running cargo test -p foo`.
- `ExecutionProgress { call_id: Option<String>, update }`.
  - The kernel stamps the call id in its `SessionToolProgressSink`
    (`kernel/src/service/progress.rs:351`). That sink is built per
    invocation in `tool_execution/context.rs:36`, where `call.call_id` is in
    scope, so it gains a `call_id` field.
  - The execution monitor reports through the sink of the `exec_command`
    that started the execution. Its output therefore goes to that call's
    entry while the call runs.
  - Output that arrives after the call has returned (a background execution)
    is not appended to a completed entry. A `write_stdin` poll shows what its
    own result returned, as Codex's `Interacted with` / `Waited for` entries
    do, and `/ps` shows the rest.
- **The display is not an event.** Events are journaled (`events_since`)
  and emitted by `exec --json`, and a tool's output can be large.
  - On `ToolCompleted`, the bridge asks the facade for the display off the
    UI thread: `AgentHandle::tool_display(session, call_id)`, through the
    presenters. It applies the reply as the entry's last revision.
  - The bridge stops calling `read_turn_page` on the UI thread (jitter
    source 7). Until the reply arrives the entry keeps its running shape,
    marked completed.
- On resume, `replay::read_into` takes the display from the same presenters
  for each call and its output. Old sessions therefore show the new
  formatting.
- The `exec --json` stream gains `callId` on `execution_progress` and
  `subject` on `tool_started`, and nothing else. That is additive under the
  stability rule ([exec --json events](site/content/docs/json-events.en.mdx));
  the page documents both.

**The cells.** `ToolCallCell` becomes a frame (bullet, verb, subject, status,
duration) with a body chosen by `AgentToolDisplay`. Per-tool views live in
`history_cell/tool/`:

| Display | Collapsed (default) | Expanded |
| --- | --- | --- |
| Command (`exec_command`) | `• Ran cargo test` (`Running` with a spinner and live seconds; red `•` on failure), then the **last 3 output lines** under `└`, or `(no output)`, then `exit 101 · 4.2s` when nonzero | `$ cargo test` in the command style, then the whole retained output in a terminal style (monospace, ANSI colours kept as styles, not escapes), then `✓ 4.2s` / `✗ exit 101 · 4.2s` |
| Patch (`apply_patch`) | `• Edited src/a.rs (+12 -3)` or `Edited 3 files (+20 -5)` with a row per file | Codex's diff: line-number gutter, sign column, tinted add and delete rows, syntax highlighting, `⋮` between hunks |
| FileRead (`read_file`) | `• Read src/lib.rs (lines 1–120)` | the text with a right-aligned line-number gutter and syntax highlighting by extension |
| Listing | `• Listed src/ (24 entries)` | the entries, directories first, with kinds |
| Search | `• Searched "foo" in src/ (7 matches)` | `path:line: text`, matches highlighted |
| Web | `• Searched the web for q` / `• Fetched https://…` | the results (title, URL) or the page's first lines |
| Mutation (`move_path`, `delete_path`) | `• Moved a → b`, `• Deleted x` | (no details) |
| Text / Fields | `• Called server.tool` plus 3 lines | everything |

- **Exploration grouping.** Codex's `Exploring` / `Explored` grouping of
  consecutive read, list and search calls into one entry is ported: the
  entry's ids are its members' ids.
- **Streaming.** A running `exec_command` entry appends each
  `ExecutionOutput` chunk to a bounded live buffer: everything up to 1 MiB,
  then the first and last 200 lines, after Codex's `LiveCommandOutput`. The
  collapsed view always shows the last 3 lines, and the expanded view
  streams.
- **The model's output is unchanged.** The display is computed beside it.
- **The control.**
  - `+ N lines` when the hidden part is lines of output, `+ Show details`
    otherwise, `− Hide details` / `− Show less` when expanded.
  - An entry with nothing more to show has no control.
  - Ctrl-T's detailed view shows everything expanded, as today.

### 4.7 Entries that change in place, and a stable layout [9]

The transcript stops treating "live" as a separate tail. An entry is
inserted where it happens and changes in place:

- **A live entry** is a cell with interior state (`Arc<parking_lot::RwLock<…>>`)
  shared between the chat widget and the transcript, and a `revision` that
  the widget raises on every change.
  - The transcript's layout cache is keyed by `(entry, revision, width,
    presentation, expanded)`.
  - Completion is the last revision. No entry is removed and re-inserted
    through the event channel, so no frame can miss it (sources 1 and 2).
  - AGENTS.md's lock rule holds: a parking_lot lock, and state computed
    before it is assigned.
- **Each entry is laid out alone.** The joined live tail goes, and with it
  the combined disclosure (source 3).
  - Animation ticks re-lay out only the entry that animates, and only its
    header row.
  - The thinking clock ticks every 100 ms and the spinner every 600 ms.
- **Running entries reserve their collapsed shape.**
  - A running command shows its header and the `└` output rows from the
    start: one `(running…)` row, growing to at most 3. It completes without
    adding rows, except the control row and the exit line.
  - A running tool of any other kind shows a dim `└ …` row, which its
    collapsed result replaces with at most 3 rows.
  - The largest jump at completion is therefore bounded and small: the
    control row plus at most 2 rows.
- **No deferral.** Updates that arrive while an answer streams are applied
  at once in their place, since entries no longer move (source 5). This
  holds in Scrollback too, because its tail writes entries in order
  (§4.11).
- **The status row** stays for the whole turn (source 4). The pending-input
  preview keeps one reserved row while the turn runs, so it does not change
  the area each time a message is queued.
- **Anchoring.**
  - While following, the view keeps its bottom anchor: growth above the
    bottom is the conversation advancing.
  - While reading, the existing anchor holds the top line.
  - A toggle keeps the clicked entry's header row on the same screen row:
    the anchor is the toggled entry's header, not the top line, when that
    header is visible.
- **Scrollback** gets the same in-place entries in its tail (§4.11). It
  writes an entry to the terminal's scrollback only when the entry leaves
  the tail.

### 4.8 Disclosure and the mouse everywhere [4][6]

- Every entry with more to show has an activity id and a control: thinking,
  every tool, the exploration group, `compress`, plan updates and long
  messages from the user (over 12 lines).
- **In Fullscreen:**
  - a left click on the control toggles it;
  - a left click on the **header row** of an entry with a control toggles it
    too, a larger target than Codex's;
  - a drag still selects, with a press-and-release on the same cell counting
    as a click;
  - F4, Up and Down, and Enter keep working.
- **In Scrollback**:
  - the entries in the tail have the same controls, clicks (D16) and F4;
  - written entries expand in the Ctrl-T transcript, which gains the
    compact presentation with controls and already captures the mouse.

  See §4.11.
- **Live entries expand independently.** A live entry expanded by the user
  stays expanded when it completes, because the id is the same.

### 4.9 `exec_command` output [7]

The pipeline stays ours: process ownership, receipts, owner groups and the
ring buffer are not touched. The fixes go where each garbling arises, and
Windows PowerShell 5.1 comes first because it is the reported case (D10).

**Why Windows PowerShell 5.1 garbles.** These are the mechanisms, from the
code and from how 5.1 (.NET Framework) behaves. E1 confirms each one on CI's
`windows-latest` runner, which ships 5.1, before anything is changed.

- **(a) Mixed encodings in one stream (pipes).**
  - PowerShell's own output (cmdlets, `Write-Output`) is written in
    `[Console]::OutputEncoding`, which is UTF-8 after our prefix.
  - A native program the script runs writes straight to the inherited handle
    in its own encoding:
    - programs that follow the console's code page (cmd's built-ins, many
      Windows tools) use the hidden console's code page;
    - programs that write the ANSI code page (`GetACP`: 936, GBK, on
      Chinese Windows) write GBK.
  - Our decoder takes the whole stream as UTF-8, so the GBK parts become
    U+FFFD. Codex's whole-buffer detector would pick one encoding for the
    stream and garble the other part.
- **(b) PowerShell decodes captured native output with
  `[Console]::OutputEncoding`.** `$x = ipconfig`, or `tool | Select-String`,
  makes 5.1 decode the native program's bytes as UTF-8. A program that wrote
  GBK is garbled inside PowerShell, before the bytes reach us, and nothing
  downstream can repair it. Only making the native program write UTF-8 helps,
  through the console code page (step 2).
- **(c) Input to native programs uses `$OutputEncoding`.** It is ASCII by
  default in 5.1, so `'中文' | git commit -F -` sends `??`.
- **(d) `write_stdin` to a 5.1 session** is read with
  `[Console]::InputEncoding`, the console's input code page, not UTF-8.
- **(e) Progress bars and console redraws.** `Write-Progress` (from
  `Invoke-WebRequest`, `Expand-Archive`, `Install-Module`, …) draws with
  cursor positioning. In PTY mode, these and ConPTY's own repaints reach
  `render_plain_text`, which ignores rows, so lines overwrite each other
  (§2.8, source 2).
- **(f) A byte-order mark.** `[System.Text.Encoding]::UTF8` has a preamble.
  E1 checks whether 5.1 writes `EF BB BF` at the start of redirected output.
- **(g) CLIXML.** Windows PowerShell can write errors to a redirected stderr
  as serialized `#< CLIXML` objects. E1 checks for it, and `-OutputFormat
  Text` rules it out.

**The steps:**

1. **Reproduce first (E1).**
   - **On `windows-latest`**, running `powershell.exe` explicitly (not
     `pwsh`) through the real `ExecutorManager`, in pipe and PTY mode:
     - `Write-Output '中文 ü'`;
     - raw ANSI-code-page bytes, written as a native program would
       (`[Console]::OpenStandardOutput().Write(…)`);
     - both in one command;
     - `cmd /c echo 中文`, and `$x = cmd /c echo 中文; $x`;
     - `'中文' | findstr .`;
     - a `Write-Progress` loop under a PTY;
     - a failing cmdlet's stderr (no CLIXML), and the first bytes (no BOM);
     - `Read-Host` answered through `write_stdin` with non-ASCII text.
   - **The runner's code pages are 1252/437**, so the GBK cases are also
     unit tests of the decoder with code page 936 given explicitly.
   - **Platform-independent:**
     - CJK text followed by CSI G and CR redraws;
     - a read that starts inside `ESC[31m`;
     - a tail of U+FFFD with half a sequence;
     - `DaFilter` holding `ESC[` at end of file;
     - a finished command with more than 64 KiB after its offset.
   - **Your case.** If you can share the command that garbled and what it
     printed, it becomes a test too.
2. **A complete PowerShell prefix**, for 5.1 and 7 alike:

   ```powershell
   try {
     $__hbUtf8 = New-Object System.Text.UTF8Encoding $false
     [Console]::OutputEncoding = $__hbUtf8
     [Console]::InputEncoding = $__hbUtf8
     $OutputEncoding = $__hbUtf8
   } catch {}
   $ProgressPreference = 'SilentlyContinue'
   ```

   - This covers (c), (d), (e) for progress, and (f): it is UTF-8 without a
     byte-order mark.
   - `-OutputFormat Text` joins the arguments, for (g).
   - **The console code page.** Setting `[Console]::OutputEncoding` on a
     console also sets that console's output code page. E1 verifies this, and
     with it the hidden console's programs write UTF-8, which covers most of
     (a) and (b).
   - **Approvals.** The canonical shell action's `version` becomes
     `agent-shell-v5`. The argv is part of the approval hash, so an approval
     remembered for a v4 command is asked again once.
3. **Decode by run, not by stream.** This replaces `Utf8Carry` in the reader
   threads, before the ring buffer.
   - ASCII passes through.
   - Each maximal run of non-ASCII bytes is checked:
     - valid UTF-8 is kept as it is;
     - otherwise the run is decoded with the fallback code page (D15): on
       Windows the ANSI code page (`GetACP`); elsewhere there is none, and
       decoding stays lossy as today, since children get `LANG=C.UTF-8`.
   - A run that ends on a lead byte of a double-byte code page takes the
     next byte, since a GBK trail byte can be ASCII (`0x40`–`0x7E`).
   - A run at the end of a read waits for the next read, or 100 ms, or the
     end of the stream.

   This handles a mixed stream (a), is deterministic, and needs no
   `chardetng`. The buffer, its offsets and the UTF-8 trimming keep seeing
   UTF-8 only, which is why the decoding happens before the buffer. Offsets
   count decoded bytes, which is what every reader already assumes.
4. **`cmd.exe` and Python in UTF-8.**
   - Prefix `chcp 65001 >NUL & ` to `Cmd` scripts.
   - Set `PYTHONUTF8=1` and `PYTHONIOENCODING=utf-8` in
     `NON_INTERACTIVE_ENVIRONMENT`.
5. **PTY output through a screen model.**
   - A PTY session's output is rendered through `vt100`, which is already in
     the workspace (0.16, the TUI's test terminal). It is sized to the PTY,
     with a scrollback.
   - The model and the interface get the screen's rows plus the lines
     scrolled off. ConPTY's repaints, progress bars, cursor moves, erases and
     wide characters therefore land where a terminal would put them (e).
   - Pipe output keeps `render_plain_text`, made width-aware with
     `unicode-width` (already used by the TUI). That suits CR redraws such
     as `pip`'s and `curl`'s progress lines.
6. **Sequence-aligned starts.** Any read whose start is not the stream's
   start begins after the next line break, or after a complete sequence when
   none is near. This applies to:
   - the monitor's skip;
   - ring trimming at read time;
   - `bound_execution_output`'s front trim;
   - the kernel's merge.

   The skipped bytes count in `truncated`.
7. **Hold-back always.** The incomplete-sequence hold-back no longer depends
   on the tail being free of U+FFFD.
8. **`DaFilter::finish`** flushes its held bytes at end of file, before the
   decoder's.
9. **No silent loss.** A finished command whose unread output exceeds the
   read bound returns its head and tail with a `[… N bytes omitted …]`
   marker, and sets `truncated`. This is Codex's head and tail with our
   character boundaries. It changes what the model sees for long outputs, so
   the exec goldens regenerate on purpose.
10. **The Windows environment (D17).** The clean allowlist lacks variables
    that Windows PowerShell 5.1 and common tools read:
    - `USERPROFILE`, `APPDATA`, `LOCALAPPDATA`;
    - `ProgramData`, `ProgramFiles`, `ProgramFiles(x86)`;
    - `SystemDrive`, `HOMEDRIVE`, `HOMEPATH`;
    - `PSModulePath`, `PROCESSOR_ARCHITECTURE`, `NUMBER_OF_PROCESSORS`, `OS`.

    This does not cause garbling, but output differs from a terminal's:
    module autoload, git's global configuration, npm's cache.

**Not ported, with reasons:**

- `codex-utils-pty`: it spawns and kills outside our `ExecutorManager`'s
  ownership, and kills on drop.
- `codex-utils-head-tail-buffer`: it splits on raw bytes.
- Codex's lossy `CapturedStream`.
- Codex's whole-buffer detector: it cannot stream, and it picks one encoding
  for a mixed stream.
- The environment list, the PowerShell prefix and the ConPTY code: ours are
  the same or, after step 2, more complete.

### 4.10 The command center [3]

| Codex feature | Here | Why |
| --- | --- | --- |
| Left on an empty composer, under Codex's conditions; `Action::OpenAgents` (no default binding); `← for agents` in the footer | **Port** | — |
| Inline full-height bottom-pane presentation | **Adapt**: keep the alternate screen | Our center is a workspace overlay and Fullscreen already owns the screen |
| Root rows only; a child's status rolls up; the most urgent child in the details | **Port**; children listed in the details pane and openable from there (Enter on a child row in the details list) | Codex's model; `/subagents` keeps the per-child list |
| Needs you / Working / Ready / Inactive (+ Error) | **Port**: Needs you = waiting for approval or a question (`AgentTurnPhase` + the session's pending approvals); Archived stays as a filter here | We have archived sessions; Codex removes them |
| Grouping by project, directory, status, model; `g`; persisted | **Port**, persisted as `tui.agentsGrouping` (a `[tui]` setting with its spec in the config crate) | — |
| Details pane (≥ 90 columns): model, reasoning, project, branch, usage, children, pending request, last activity, last message, prompt | **Port**: model, reasoning, cwd, branch and usage are in `AgentSessionSummary`; the last message and prompt come from the session's last turn page, read in the background for the selected row | — |
| Search (`/`, F3) | **Port** through `AgentSessionListRequest.search` | — |
| Rename (`r`) | **Port** through `rename_session` | — |
| Archive and delete confirmations | **Port** (both confirmed in a modal; Backspace deletes) | — |
| Stop (`x`), fork (`f`), new (`n`) | **Port** (ours: `i`, `f`, `n`); keys change to Codex's | — |
| New in a worktree (`w`) | **Port** through the worktree plugin's `/worktree` path | — |
| Pin (`p`), hide (`h`) | **Adapt**: kept in a new TUI state file (D11), not the session | No backend for sections; a session format change is not worth it |
| Unread marker | **Adapt**: a session whose `updated_at` passed the last time it was shown | No read-state in the Agent |
| Usage in credits, voice badge, server-version notice | **Drop** | No such backend |
| Live refresh | **Port**: while open, the center subscribes to the facade's session events (turn phase changes, approvals, titles) through a bridge watch like the subagents watch, debounced to 4 Hz, and re-reads changed sessions in the background | Nothing refreshes it today |
| `?` help, wrap-around selection, configurable keys (`KeymapContext::Agents`) | **Port** | — |
| Mouse | **Add**: wheel, click to select, double click to open, click on a filter | Codex has none; our center owns the screen |

Inside the center, Left and Right stop changing the filter (Tab and
Shift-Tab do, and `1`–`5` stay), so Right opens, as in Codex.

### 4.11 Scrollback: the interactive tail [4][5][9]

Scrollback cannot change a line once it is in the terminal's scrollback. To
give it the features Fullscreen has, the part of the screen the interface
repaints keeps the recent entries, and only entries that leave it are
written. That part is the inline viewport above the composer.

- **The tail** is the inline viewport's upper part.
  - It holds the newest entries not yet written, in order, of every kind:
    your message, thinking, tool calls, answers.
  - Each entry is laid out as in Fullscreen: the same cells, ids, controls
    and per-entry cache (§4.7).
- **Writing to scrollback.**
  - When the tail would grow past its limit (60% of the screen, at least 8
    rows), its oldest **completed** entries are written to the scrollback
    (`insert_history_lines`) in their current state, expanded or collapsed.
  - A written entry that still has more to show ends with a dim `ctrl + t
    to expand`.
  - A live entry is never written. If it alone exceeds the limit, the tail
    shows its header and its last rows.
  - **An answer is the one entry written in parts.** It streams into the
    tail like any entry, and its lines are written from the top as it
    passes the limit. It has no control, so nothing is lost by writing it
    early.
- **When everything is written.** The whole tail is written when you send
  the next message, and on `/new`, `/resume`, a resize and quitting. So a
  finished turn's thinking and tool calls stay expandable in place until you
  send the next message.
- **Thinking in Scrollback.** `Thinking 3.4s` changes in place in the tail
  and becomes `Thought · 20.7s` there. It stays expandable until it is
  written, and it is written as it is shown.
- **Stability.**
  - The tail's height never shrinks during a turn. Collapsing an entry
    leaves blank rows until the next write.
  - It grows, then stays at its limit while writes from its top scroll the
    content up into the scrollback.
  - `Tui::draw`'s clear on a height change therefore happens only while the
    tail grows. Once the tail reaches its limit, nothing is redrawn for a new
    entry (source 6).
  - The status row stays the whole turn, as in Fullscreen.
  - No updates are deferred: entries are written in order from the tail, so
    order holds without holding updates back (source 5).
- **The mouse.**
  - Scrollback does not capture the mouse today, so the wheel scrolls the
    terminal's scrollback. Capturing it would let a click reach a control in
    the tail, but it takes the wheel.
  - D16 recommends capturing while the tail holds an entry with a control,
    and releasing on the first wheel event until the next key or the next
    new entry. The terminal's own scrolling then works from the second
    notch.
  - A new setting chooses the behaviour: `tui.scrollbackMouse = "tail" |
    "off"`, with its spec in the config crate.
  - Shift-drag keeps the terminal's own selection, as in Fullscreen.
  - The keyboard always works: F4 focuses the tail's activities, and Enter
    toggles.
- **Older entries.** The Ctrl-T transcript (alternate screen, with the mouse)
  shows every entry in the compact presentation with controls. Its detailed
  view stays one Ctrl-T further.
- **Windows.**
  - The inline viewport relies on the cursor-position query.
  - Mouse capture turns off the legacy console's Quick Edit while it is
    held.
  - Both are tested on Windows Terminal and the legacy console (conhost),
    in CI's Windows job and by hand.

## 5. Compatibility and the host

- **Persisted formats.** Nothing changes in history, sessions or
  `providers.toml`/`mcp.toml`.
  - `config.toml` gains `tui.agentsGrouping` and `tui.scrollbackMouse`, and a new TUI state file
    (`tui-state.json`, D11) holds pinned, hidden and last-seen session ids.
  - A reset removes keys, which the tolerant reader already handles.
- **Facade API (Hanafubuki):**
  - `AgentRuntime::settings_catalog()`, `write_setting_values(..)` and
    `run_settings_action(..)` are new.
  - `AgentSettingsView.web_search_fields` and `agent_settings_fields()` are
    removed (Hanafubuki's settings page reads the `web` section).
  - `PluginOptionSchema`'s path and field names change.
  - `AgentToolPresenter`, `ToolPresenterRegistry` and `AgentToolDisplay` are
    new.
  - `AgentHandle::tool_display(session, call_id)` is new. The
    `ToolStarted` (`subject`) and `ExecutionProgress` (`call_id`) fields are
    additive. `ToolCompleted` is unchanged.
- **The `hanafubuki-agent-host` twin is not touched.**
- **Goldens:**
  - The model-visible goldens change only through §4.9 steps 3 and 8 (the
    environment and long-output head and tail) and D9 (`startLine`, if
    accepted). Each is regenerated on purpose in its milestone.
  - The TUI snapshot goldens change throughout.
- **Removed, per the development principles:**
  - `config_center/forms`, `/config quick`, the separate editor views;
  - `agent_settings_fields`;
  - the live tail's joined layout;
  - the thinking footer hint.

  No aliases are kept, except the slash commands as shortcuts into the
  center (D2).

## 6. Tests

- **Settings.**
  - Every serialized leaf has a spec, a row, and a no-op on an untouched
    page (`settings_center/tests`).
  - Every spec's key is in the reference tables of both languages (`cargo
    xtask check-config-reference`).
  - `write_values`: set, reset, a commit group, a strict refusal with the
    file unchanged, a repair refused, comments and unknown keys kept, and
    provenance for each layer.
  - `config set` and the center produce the same file for the same input.
- **Collections.** The Provider and MCP record round trip, a stale revision,
  secrets never returned, and each action dispatched to its owner.
- **Center interaction** (driven through `App`, not Tab counts):
  - search across tabs, a toggle saving at once, an inline edit with error;
  - Esc from a page override returning to its row;
  - a shadowed write's notice;
  - mouse select and activate;
  - snapshots at 140 and 50 columns.
- **Command center.**
  - Left opens only under the conditions, and not when Left is rebound.
  - Rollup of child statuses, grouping, search, rename, confirmations.
  - A session event repaints while open.
  - Snapshots with and without the details pane.
- **Thinking.**
  - Entries in order across thinking, tool, thinking, answer.
  - Live tenths, then `Thought · 20.7s`, then `(truncated)`.
  - A live entry expanded stays expanded at completion.
  - Resume shows every segment.
  - The status line's `(Thinking …)` with an open and a closed item, and the
    pause during an approval.
- **Tools.**
  - Each presenter on recorded arguments and outputs.
  - The fallback for an unknown tool.
  - A streamed command's live tail and its bound.
  - `callId` on progress for a foreground and a background execution.
  - Exploration grouping.
  - Snapshots per display, collapsed and expanded.
- **Stability.**
  - A frame-by-frame test: insert a running tool, stream output, complete
    it. Assert the rows above the entry never move while reading, and that
    at completion the entry grows by at most the control row plus 2.
  - No frame without the entry.
  - The status row present throughout.
  - Scrollback's viewport not cleared when a second tool starts.
- **Scrollback** (the `vt100` test terminal the TUI already uses):
  - entries change in place in the tail, and a click or F4 expands them;
  - the oldest completed entry is written when the limit is reached, as it
    is shown;
  - a live entry is never written;
  - a long answer is written line by line in order;
  - the tail is written whole on the next message;
  - the order in the scrollback equals the conversation's order;
  - the viewport's height never shrinks during a turn;
  - mouse capture is released on the first wheel event, and taken again at
    the next key.
- **Exec output.** Every test of §4.9 step 1, then each fix turning it
  green.
  - The Windows PowerShell 5.1 cases run in CI's `windows-latest` job, with
    `powershell.exe` named explicitly, and their results are reported as
    such.
  - The code page 936 cases are decoder unit tests, since the runner's code
    page is 1252.

## 7. Milestones

**Order:**

- S1 → S2 → S3 → S4 → S5 → S6 → S7.
- A1 → A2, after S1 (for `tui.agentsGrouping`).
- T1 → T1b → T2, then T3 → T4 → T5 → T6. T7 runs alongside T1, T1b and
  T5.
- E1 → E2 → E3, independent of the rest.

Each milestone, when done, records in `todo.md`:

- what landed;
- the validation commands that ran;
- whether goldens changed;
- what Hanafubuki must change at its next pin;
- any module-size exception added or raised.

Every milestone runs AGENTS.md's validation matrix. Those that touch the
site run the site's too.

**Settings**

- **S1 — the vocabulary.**
  - `foundation::settings` (§4.1); `PluginOptionSchema` and the guest
    lowering moved onto it, with aliases, labels and disabled reasons.
  - Not model-visible; no golden.
  - Hanafubuki: the renamed types.
- **S2 — specs beside the settings, and generic reads and writes.**
  - A section per config domain, the decoders reading their keys and bounds
    from the specs, and per-key provenance in `layers::resolve`.
  - `AgentConfig::write_values` with reset and outcomes; `config get/set`
    on it.
  - The new coverage test, and `check-config-reference`.
  - `SettingsWrite` in `strict-decoding-policy.toml`.
- **S3 — the catalog and collections in the facade.**
  - `settings_catalog`, `write_setting_values`, `run_settings_action`.
  - Collections for Providers, the model policy, MCP, plugins,
    marketplaces, hooks, roles, profiles and trust.
  - `agent_settings_fields` removed.
  - Hanafubuki: the settings page's web section from the catalog.
- **S4 — the center.**
  - `settings_center/`: page stack, Codex's presentation (§4.4), generic
    section and collection pages, inline editors, reset, search, mouse,
    asynchronous writes.
  - The `PageRegistry` with the theme, status line, title, keymap, presets,
    plugin browser and hook trust pages moved in.
  - Validation: the center's snapshots at 140 and 50 columns, and the
    interaction tests of §6.
  - `config_center/` removed.
- **S5 — one interface.**
  - The separate editor views removed; their commands open the center at
    their page.
  - `/config quick` removed; the second memory and compaction writers
    removed.
  - The TUI's `[tui]` section registered.
- **S6 — the settings that were missing** (§4.4's list), each with its
  tests.
- **S7 — documentation.**
  - In both languages: `terminal-interface` (Settings, and the commands
    table), `configuration`, `config-reference` (every key checked),
    `architecture` (the catalog), `embedding` (the facade calls),
    `plugin-rules` (options as settings), `writing-plugins` and
    `host-plugins` (declaring settings).

**Command center**

- **A1 — entry and model.**
  - Left with Codex's conditions, `Action::OpenAgents`, the footer hint.
  - Root rows with rollup, the four groups plus Error, the new keys,
    `?` help.
  - Grouping and `tui.agentsGrouping`.
  - Confirmations, search, rename, worktree.
- **A2 — details and liveness.**
  - The details pane, the session watch with background re-reads, pin,
    hide and unread in `tui-state.json` (D11), mouse.
  - Docs: `terminal-interface#agents` and `data-directory`, both languages.

**Timeline**

- **T1 — entries in place.**
  - Live entries with shared state and revisions, and a per-entry layout
    cache.
  - The joined live tail removed; Fullscreen's deferral removed.
  - The status row kept for the turn.
  - No visible change beyond stability; snapshots updated.
- **T1b — Scrollback's tail** (§4.11).
  - The tail with entries in place, writing oldest completed entries at the
    limit, answers written line by line, the whole tail written on the next
    message.
  - The height kept during a turn, mouse capture per D16, the Ctrl-T
    transcript with controls.
  - Docs: `terminal-interface#modes` and `#reading`, both languages.
- **T2 — thinking entries.**
  - Ids; `Thinking 3.4s` and `Thought · 20.7s`; independent expansion;
    Ctrl-O for all.
  - `ReasoningUpdated` forwarded; `Working 25s (Thinking 20.7s)`; the
    separator's `thought for`.
  - Docs: `terminal-interface#thinking` and `#layout`, `internals` (the
    terminal interface).
- **T3 — disclosure and the mouse everywhere.**
  - Ids and controls on every expandable entry; a header click.
  - In both presentations: Fullscreen inline, Scrollback in its tail and
    in Ctrl-T.
  - Docs: `terminal-interface#reading` and `#transcript`.
- **T4 — presenters and events.**
  - The contract and the registry.
  - Presenters for every built-in tool; the fallback.
  - `subject` on `ToolStarted`; `callId` on `ExecutionProgress`, stamped
    by `SessionToolProgressSink`.
  - `tool_display`, fetched off the UI thread at completion and used for
    replay; the synchronous history read removed.
  - Docs: `json-events`, `plugin-rules` (presenters), `writing-plugins`.
  - No model-visible change: the request goldens hold.
- **T5 — per-tool views.**
  - §4.6's table, the line-number diff, exploration grouping.
  - D9's `startLine`, if accepted: model-visible, so the goldens regenerate.
  - Docs: `terminal-interface`, a new "Tool calls" heading (`#tools`), both
    languages.
- **T6 — streamed command output.** The live buffer, the collapsed tail, the
  expanded stream.
- **T7 — stability.** The reserved collapsed shape, the toggle anchor,
  Scrollback's reserved live region, and the frame-by-frame tests (§6).

**exec_command**

- **E1 — reproduction.** The failing tests of §4.9 step 1: Windows
  PowerShell 5.1 on `windows-latest` in pipe and PTY mode, the code page 936
  decoder cases, and the platform-independent ones. No fix. Record which of
  (a)–(g) reproduce.
- **E2 — encodings.**
  - The complete PowerShell prefix and `-OutputFormat Text`
    (`agent-shell-v5`); decoding by run with the ANSI code page as fallback;
    `chcp 65001` for `Cmd`; the Python variables; D17's environment.
  - The prefix and the environment are not model-visible. The execution
    tests that pin them are updated.
  - Docs: `tools`, `environment`.
- **E3 — terminal text and bounds.**
  - The `vt100` screen model for PTY output, width-aware pipe rendering,
    sequence-aligned starts, the hold-back, `DaFilter::finish`, head and
    tail with a marker.
  - Model-visible for PTY and long outputs: goldens regenerated on purpose.
  - Docs: `tools#exec`.

**Module size.**

- `settings_center/`, `history_cell/tool/` and `transcript_view/` are each
  split by responsibility from the start (pages, editors, render; one module
  per display; entries and layout).
- `config/src/settings/` gains a file per domain.
- No exception is planned. Any that becomes necessary is reported.

## 8. Decisions

Every decision was accepted as recommended (2026-11-12). D7, D10 and D13
record your answers.

| # | Question | Recommendation |
| --- | --- | --- |
| **D1** | Port Codex's settings *data model* (a hard-coded list) or only its presentation? | **Presentation only.** Its list is less complete than ours and hard-coded, which is what [2] asks to remove. |
| **D2** | Keep `/mcp`, `/providers`, `/theme`, … as commands? | **Yes, as shortcuts that open the center at their page.** Nothing opens a separate interface ([1]), and the commands keep working. |
| **D3** | Remove `agent_settings_fields` / `web_search_fields`, which Hanafubuki reads? | **Yes.** Two schemas is the problem [2] names. Hanafubuki moves to the catalog's `web` section at its next pin. |
| **D4** | Generate `config-reference`'s tables from the specs? | **No: check them** (`check-config-reference`). The tables carry prose and links per key that specs should not hold, and generated `.mdx` would fall under "do not edit generated files". |
| **D5** | `(Thinking …)`: the turn's total, or the current segment? | **The turn's total**, which `AgentReasoningSnapshot` already keeps. The current segment is on its own entry. |
| **D6** | Ctrl-O after per-entry disclosure? | **Keep it as "expand or collapse every thinking"**, no longer sticky for later entries. Dropping it would break a documented key for no gain. |
| **D7** | Scrollback cannot change written lines: how does it meet [4]? | **Answered (2026-11-12): it must work in Scrollback too.** The recent entries stay in a repainted tail, where they change and expand in place, and are written when they leave it (§4.11). Older entries expand in the Ctrl-T transcript, which gains controls. |
| **D8** | Saving at once (Codex) or per form (today)? | **At once** for booleans and choices; **on Enter** for typed fields; **as a group** for a commit group. A form-wide Save only exists for collections' records, whose revision gate needs one write. |
| **D9** | `read_file` line numbers: its `offset` is in bytes. | **Add `startLine` to `FileReadResult`**, counted by the reader. It is model-visible (goldens regenerate in T5), and the model gains a useful fact. The alternative is numbering only reads from offset 0. |
| **D10** | Which platform, shell and command garbled? | **Answered (2026-11-12): Windows, Windows PowerShell 5.1.** §4.9 lists its mechanisms (a)–(g). E1 reproduces them on `windows-latest`. A sample of the command and its output would add a test. |
| **D11** | Pin and hide in the command center, with no backend. | **In a new TUI state file, `tui-state.json` in the data directory**, decoded tolerantly and quarantined when unreadable, not in the session format. The TUI has no state file today. |
| **D12** | Exploration grouping (Codex) for read, list and search calls. | **Yes**, since the request asks for Codex's presentation, and it keeps a run of reads to one entry. |
| **D13** | Which presentation do you run: Fullscreen or Scrollback? | **Answered (2026-11-12): both must work.** T1b (Scrollback's tail) follows T1, and every later T milestone covers both presentations. |
| **D14** | The status line: `Working 25s (Thinking 20.7s)` alone, as asked, or with today's `• esc to interrupt`? | **Keep the hint**, after the times, as today: Esc is the only interrupt and the line is where people learn it. If "only" was meant strictly, the hint moves to the footer's shortcuts. |
| **D15** | The fallback code page for non-UTF-8 runs. | **The ANSI code page (`GetACP`) on Windows, none elsewhere.** On Chinese, Japanese and Korean Windows the ANSI and OEM code pages are the same (936, 932, 949, 950). They differ only for Western accents, where a console-page program's output follows the console's code page, which the prefix sets to UTF-8 (E1 verifies it). Codex's detector cannot handle a mixed stream. |
| **D16** | Mouse capture in Scrollback, which takes the terminal's wheel. | **Capture while the tail holds an entry with a control, and release on the first wheel event until the next key or new entry** (`tui.scrollbackMouse = "tail"`; `"off"` for keyboard and Ctrl-T only). Without capture, a click cannot reach the tail. |
| **D17** | The clean Windows environment (seven variables) differs from a terminal's. | **Add the non-secret system variables §4.9 step 10 lists**, not user variables generally. Codex inherits everything; ours stays an allowlist, but one tools work in. |

## 9. Not in this plan

- A raw TOML editor, or editing unknown keys.
- A settings page for the CLI or a web UI (the catalog makes one possible).
- Persisting disclosure state across runs.
- Thinking summaries generated by the Agent (Codex's status-header summary
  line) — the Provider's text is shown as sent.
- Images inside expanded tool entries (Codex previews `view_image` in
  Fullscreen); `view_image` shows its path.
- Pinned sections synchronized across machines, usage in credits, voice.
