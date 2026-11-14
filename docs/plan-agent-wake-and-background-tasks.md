# Plan: wake events, nested sub-agents and the background task manager

Repositories, read from source on 2026-11-13:

- The Agent: `/root/code_workspace/hanafubuki-agent`, main `147fdaa`. Every
  `file:line` below is at that commit; the milestones of the two earlier
  plans land first and move lines, so a milestone re-reads the symbol named
  beside each reference.
- Codex: `/root/code_workspace/codex`, `57a808a392`. It has a footer for
  running background terminals and an exit watcher, but nothing that resumes
  a conversation; Hoshimi has neither. Neither is a model for the wake.

The user answered the questions of §5 on 2026-11-13. Q4's answer (the
command timeout becomes a setting) was worked in against main `ed2828f`,
after W1 (`981765d`): the references D12 and R4b add are at that commit,
and are marked so.

The request has five parts. Numbers in brackets refer to them throughout:

- [1] **The main agent's "Hook" mechanism**: after delegating, the main
  agent either keeps working or ends its turn and suspends. Suspended, it
  resumes by itself when a sub-agent finishes or sends it a message. An
  option turns this on and off, and it is on by default. The aim is that
  nothing occupies execution resources while it waits.
- [2] **Sub-agents**: at most 8 by default. A sub-agent may spawn its own,
  two levels deep at most (Main → Sub-agent → Sub-agent). The same
  mechanism applies to sub-agents.
- [3] **Terminals**: a long-running command works the same way. The agent
  keeps calling tools, or ends its turn and is resumed when the command
  ends.
- [4] **Management**: `/` commands and a dedicated view that lists the
  running sub-agents and terminal sessions, to inspect and close them. A
  manual close also wakes the suspended owner, telling it what was closed.
- [5] **Status**: an indicator at the bottom showing the number of running
  agents and terminals, kept current as they start, end or are closed.

**Name.** User-configured hooks already exist (`crates/hooks`,
`crates/plugin-hooks`: `PreToolUse`, `PostToolUse`, `SessionStart`,
`UserPromptSubmit`, `Stop`). So what the request calls the "Hook" mechanism
is called **wake** everywhere in code, configuration and documentation:
*wake events*, a session *is woken*, a *woken turn*. The plan says so once,
here.

The `todo.md` section "Plan: wake events, nested sub-agents and the
background task manager" tracks the milestones R1–R4, R4b and R5–R8
(§6).

A path this plan names that does not exist yet is a file its milestone
creates; it is marked "(new)".

## 0. Terms

- **Root**: a session without a parent. **Tree**: a root and its sub-agent
  descendants (origin `Subagent`), at depth 1 (the root's sub-agents) and
  depth 2 (theirs). Side conversations and reviews are children, but not
  part of a tree (D19).
- **Owner**: the session a background task reports to. A sub-agent's owner
  is its parent; a terminal's owner is the session whose `exec_command`
  started it.
- **Background terminal**: a model's `exec_command` execution still running
  after the call that started it returned a `session_id`. It is the same
  thing `/ps` lists (`list_background_terminals`).
- **Wake event**: a typed record of something that happened to an owner's
  background task (§3.1). It is queued on the owner.
- **Live agent**: a sub-agent running a turn, or idle while its own
  sub-agents or terminals still work. It is "waiting", and it is not done.
- **Suspended**: a session with no turn and live background work. It holds
  no thread, task or turn, only its queue. A **woken turn** is the
  Programmatic turn started for a suspended session whose queue is not
  empty.
- **Wake policy**: whether wake events are delivered and sessions woken
  (D5). It is on by default.

## 1. The request against the code

The surveys this plan started from were checked against `147fdaa`. Where
the code says otherwise, the design follows the code.

| Premise | What the code says | Consequence |
| --- | --- | --- |
| A parent learns of a child's end only through `wait_agent` | Correct. `SubagentRuns` (`crates/agent/src/plugins/subagents/service.rs:135-152`) keeps an in-memory `unreported` flag that `take_updates` (`:184`) clears and `wait` (`:324`) blocks on with a `Notify`. A result that arrives while the parent is idle waits in memory for the next `wait_agent`, and a restart loses it. | The kernel queue replaces `unreported` (D2, D5). Its results become durable. |
| Children cannot message their parent | Correct. The header says "agents never message their parent or each other" (`service.rs:9-13`). `send_message` resolves only the caller's children (`resolve`, `:501`). | `send_message` gains `target` `parent` (D7). |
| Depth is refused in `spawn` | Correct (`service.rs:231-235`): "a sub-agent cannot spawn agents of its own…". The guidance is shown only when `parent_session_id.is_none()` (`prompt.rs:84`). | Depth comes from lineage and allows 2 (D9). |
| `maximumRunningAgents` counts running turns of one parent | Correct: `SubagentRuns::running(parent)` (`service.rs:155`) and the check again in `start` (`:456-465`). The default is 3 and the range 1..16 (`options.rs:11,17`). | Default 8, counted as live agents across the tree (D10). |
| Child turns run in a plain `tokio::spawn` with no sink | Correct (`service.rs:478`, through `AgentSessionDriver::run_turn`, `session_driver/mod.rs:102`, which calls `AgentService::start_turn` with `NoopAgentEventSink`). Their events land only in the child's journal. | The TUI reads the journals of running children every 400 ms (`crates/tui/src/agent_bridge/subagents.rs:35`), including to surface a child's approval. R3 gives child turns a forwarding sink (D14). |
| Continuation hooks continue only the same turn | Correct: `run_continuation_hooks` runs at a candidate final answer (`loop_driver.rs:122-129`). Nothing starts a turn on an idle session but a caller. | The wake is new: a dispatcher plus a wake admission (D4). |
| Steering is consumed at each step | Correct: `consume_pending_steering` (`loop_driver.rs:88`, `:146`) turns pending steering into `SteeringMessage` items and forces a follow-up. `finish_completed` refuses to end a turn with steering pending (`terminal.rs:61`). | Wake events are drained at the same boundary, right after steering (D3). |
| A `!command` item is the precedent for a typed, user-role history item | Correct. `ConversationItem::UserShellCommand` (`model/conversation_item.rs:239`) is lowered to the user-role `<user_shell_command>` fragment (`model/user_shell_command.rs`, `conversation_item.rs:458`). c930bcc shows every site a new item touches: the history store (`store/mod.rs:577`, `store/memory.rs:301`), the reviewer (`permission.rs:173`), compression (`compression/adapter.rs:186`), `/context` (`sampling/composition.rs`), validation (`validation/persistence.rs`, `tool_pairs.rs`), export (`service/export.rs:113`), `cli/src/sessions.rs:136`, TUI replay (`agent_bridge/replay.rs:155`). | `ConversationItem::WakeEvents` follows it (§3.2), with one new golden. |
| A terminal exit while the session is idle reaches no sink | **Not quite.** `emit_session_event` (`kernel/src/service/progress.rs:522-531`) sends session events to the sink of the session's **latest managed turn**, which `spawn_managed_turn` retains (`turn/admission.rs:168-171`) until the session is removed. A child (no managed turn) has none. | The model is never told, and a UI is told only if it kept its last turn's sink. Terminal wakes are produced in the kernel (D11). |
| `exec_command` yields after at most 30 s and completion is seen only on a poll | Correct: `MAX_WAIT_MS` (`crates/execution/src/resolver.rs:18`), the model's default yield is 10 s (`plugin-execution/src/arguments.rs:67-74`), and a `write_stdin` poll defaults to 5 s. An exit is reported only when a call returns it. The tool then reports `ExecutionDisposed` (`tool.rs:255-262`, `:294-300`). | Unchanged. The wake is added beside it (D11). |
| Every command is killed after 10 minutes | Correct: `DEFAULT_COMMAND_TIMEOUT_MS` (`resolver.rs:10`) is the plan's timeout for every `exec_command` (`shell.rs:107`). | The timeout becomes the setting `pluginOptions.execution.commandTimeoutMinutes`, 10 minutes by default (D12, R4b). A timeout is a wake event. |
| Terminals survive turn end and are stopped only on archive, delete, side close or shutdown | Correct. `/stop`, archive (`service/sessions.rs:329`) and side close (`service/children.rs:107`) all go through `stop_background_terminals_confirmed` (`service/background_terminals.rs:23`, via `quiesce_background_terminals`, `:108`). | The person's stop and the Agent's teardown must split, so that only the person's wakes the owner (D13). |
| `SubagentStart`/`SubagentStop` hooks are "unsupported: the Agent has no subagents" | The text is stale (`crates/hooks/src/event.rs:78`). | The text is fixed in R2. The events stay unsupported (Q10). |
| Module sizes | Code lines (`cargo xtask module-size-report`): `subagents/service.rs` about 459 (585 physical lines), `agent_bridge/mod.rs` 596, `stage_turn_admission` 196 of 200 (`turn/state.rs:116`), `conversation_item.rs` 545, `kernel/src/conversation/mod.rs` 578, `agent/src/service/mod.rs` 556 (its `construct` 188), `session_driver/mod.rs` 518, `kernel/src/activity.rs` at its 647 exception ceiling. | Splits are planned per milestone (§6, "Module size"). Nothing is added to `activity.rs`. |
| W1 (a POSIX shell on Windows) is in progress | It is not in `todo.md` at `147fdaa`. Its scope, as the coordinator described it, is `crates/execution`'s shell resolution. | W1 landed (`981765d`) before the questions were answered. R4b, which makes the timeout a setting, edits `shell.rs` after it. |

### 1.1 Corrections to the direction given

The coordinator's direction is followed except where marked here.

- **"Exit while idle reaches no sink."** It reaches the last managed turn's
  sink (above). The design does not rely on it either way.
- **"With the option off, behaviour is exactly today's."** With the option
  off, a sub-agent's final status still goes through the kernel queue
  (D5), which `wait_agent` reads. The model sees exactly today's results,
  but they now survive a restart. Keeping the in-memory `unreported` path
  for "off" would leave two stores of the same fact.
- **"Every background terminal wakes, no new argument."** This is kept, with
  one consequence spelled out. A command meant to run indefinitely (a
  development server) is killed at the command timeout, 10 minutes unless
  the person sets `pluginOptions.execution.commandTimeoutMinutes` (D12).
  The wake then resumes its owner to say so: a woken turn nobody asked
  for, at most once per such terminal. In `exec`, which waits for the tree
  (D18), such a terminal holds the run open until its timeout. A longer
  setting lets a server or a build outlive 10 minutes, and makes both
  waits that much longer, up to 24 hours; the guidance tells the model to
  stop terminals it no longer needs.
- **"A dispatcher starts the woken turn using the session's sink."** The
  Agent does not start a root's woken turn by itself. It tells the host
  (`AgentBackgroundEvent::Woken`), and the host submits it (D4). A host's
  sinks may be per-turn (an IPC channel closed at the turn's end), and a
  terminal interface showing another session must not have turns start in
  a sink it no longer reads. Sub-agents' woken turns are started by the
  Agent, as their turns are today.
- **"Cap across the tree, counting running turns."** Counting running turns
  lets suspended sub-agents resume past the cap: their woken turns cannot
  be refused. The cap counts **live agents** instead (D10), which woken
  turns never change.

## 2. Decisions

Each decision gives what is decided, why, and the alternative rejected.
Those marked **Q** were also questions for the user, who answered them on
2026-11-13 (§5): each as recommended, except Q4, which made the command
timeout a setting. D12 records that answer.

| # | Decision | Why | Rejected |
| --- | --- | --- | --- |
| **D1** | The mechanism is **wake**. Nothing new is called a hook. | Hooks are a user feature with their own config key, page and plugin. | "Hooks", "continuation" (the kernel's continuation hooks continue a turn, never start one). |
| **D2** | **One durable queue per session, in the kernel**: `SessionRuntime.wake_events` (new), typed `AgentWakeEvent` (§3.1), committed and recovered with the session's runtime as `pending_steering` is (`service/persistence.rs:443`, `recovery/helpers.rs:76`), decoded tolerantly per event. | Delivery happens at step boundaries and at admission, which are the kernel's. Durability comes free with staged commits. One store serves `wait_agent`, the step and the woken turn. | An in-memory queue in the agent crate: it cannot be drained at a step, and it is lost at restart. |
| **D3** | **Delivery** (with the policy on). Every turn admission drains the queue into one `ConversationItem::WakeEvents` committed right after `TurnStart`, before the turn's opener. Every step boundary drains it after steering (`loop_driver.rs:88`) and forces a follow-up, as steering does; `finish_completed` refuses to end with events pending, as with steering (`terminal.rs:61`). A wake admission has the item as its only opener. | A running turn ("keeps working") sees results at its next step. A person's turn sees what arrived while they typed, before their message. One rendering for all three. | Delivering only through new turns: a running turn would not see a result until it ended. Delivering as steering text: untyped, unbounded, and not distinguishable from the person's words. |
| **D4** | **Waking.** A session is woken when its queue is not empty, it runs no turn, its policy is on and its tree is not paused (D16). A dispatcher (§3.4) does it on two signals: an event queued on an idle session, and `TurnFinished` with events left. A **sub-agent** is woken by `SubagentService`, which runs the woken turn as it runs its turns. A **root** is woken by its host: the Agent emits `AgentBackgroundEvent::Woken { session_id }`, and the host calls `submit_wake_turn_with_sink`, which admits the turn if events are still queued and returns `None` otherwise. Events that arrive together become one turn. | A sub-agent's turns are already the Agent's. A root's events go to the host's sink, and only the host knows whether the session is shown and which sink is live. A drained queue is a no-op, so a person's message racing a wake is harmless. | Starting a root's woken turn on the kernel's retained sink (§1.1). A wake port in `AgentHostPorts` built per instance: the terminal interface and `exec` install their sinks at launch, as `status` and `plugin_event_sink` are (`agent/src/service/mod.rs:86-100`, `standalone/mod.rs:65`). |
| **D5 · Q1** | **One option: `wakeOnBackgroundTasks`**, a boolean in `config.toml`'s General section, default `true`, for sub-agents and terminals together. The driver gives each session the kernel policy `AgentWakePolicy` at activation, reconciled while idle and committed without a revision, as the compaction strategy is (`set_session_compaction_strategy`, internals "Sessions and turns"). It also gives each turn the extension `TurnWake { on }` (frozen at admission), which the guidance reads. With it **off**: the kernel drains nothing at an admission or a step and admits no wake turn, terminals queue nothing, the dispatcher wakes nothing, and a parent cannot be messaged; a sub-agent's final status is still queued, and `wait_agent` takes it. Side conversations and reviews always have it off (D19). | One switch. A host's `config.toml` carries it like the other General keys. A single store for final statuses (§1.1). | A plugin row as the switch (`disabledPlugins`): the behaviour spans the kernel, two plugins and the facade, and a row's teardown would not undo a policy given to sessions. Two options (agents and terminals): nothing asks for it. |
| **D6** | **What the model is told.** With the policy on, the sub-agent guidance replaces "wait for them before yielding" with "end your turn; you are resumed", and the execution plugin adds a short `background_terminals` fragment. Both texts are in §3.3. `wait_agent` stays for when the model needs a result within the turn. The tool descriptions stay true in both modes. | The guidance is per step and can follow the policy. A tool description is fixed per registration (plugin rules, "Tools") and shared by every session. | Putting the rule in tool descriptions: they would be wrong with the option off, or restart plugins when it changes. The kernel's instructions: they would name plugin tools from the kernel. |
| **D7 · Q7** | **A child messages its parent with `send_message`, `target` `parent`.** Delivery is a wake event: at the parent's next step if it runs, else it wakes the parent. At most 16 unread messages are queued per parent; a 17th is refused to the child. | No new tool in every request's tool list. `target` already names whom to message. | A dedicated `message_parent` tool. Steering the parent's turn: untyped, and impossible while it is idle. |
| **D8** | **A child's end, as its parent hears it.** *Completed* is reported once the child's own subtree is quiet (no live sub-agent, no running terminal). An end while it still waits on them is not reported, so the parent is not woken for an interim "I am waiting". *Errored*, *closed by the user* and *lost at a restart* are reported at once. An interrupt the parent made itself (`interrupt_agent`) is not reported, as today. | The parent should wake once, when the delegated work is done. Failures and closes are news it must act on. | Reporting every child turn end, which wakes the parent for each interim yield. |
| **D9** | **Nesting.** A session's depth is its count of `Subagent` ancestors up to the root (`parent_session_id`). A session spawns when it is a root, or a `Subagent` at depth 1. Depth 2 is refused, and so are side conversations and reviews (§3.5 has the texts). Wake events go to the **direct** parent only. `list_agents`, `interrupt_agent`, `send_message`, `followup_task` and `wait_agent` address direct children, as today. | This is the request's two levels. Each agent coordinates what it spawned. | One flat namespace per tree: a depth-1 agent's children would leak into the root's tools. |
| **D10 · Q2/Q3** | **`maximumRunningAgents`: default 8, range 1..16, counting live agents in the root's tree.** `spawn_agent` and a `followup_task` that would make an idle agent live are refused at the cap. A woken turn never is: a woken agent is already live, or becomes live only through an event about its own work. | Eight agents of the whole tree work at once. A suspended agent holds no turn but is not done, and counting it keeps the bound strict without refusing wakes. | Running turns per parent: 8 at the root and 8 under each child allow 72. Running turns across the tree: wakes past the cap would have to be refused or allowed to exceed it. |
| **D11** | **Terminal wakes.** Every background terminal of a **model's** `exec_command` (never a `!command`'s, which its own turn waits for) queues an event on its owner when it ends: exited, failed or timed out, with no call having reported that end. The kernel produces it as it applies the terminal `ExecutionStatus` in the session's progress queue (`stage_execution_progress`, `progress.rs:488`), for the tracked execution (the disposal, not whether the call is in flight, proves its result reported the end; see §3.8). It is **withdrawn** when that execution's `ExecutionDisposed` is applied: a model's poll reported the end, or the Agent disposed of it. A `Cancelled` status (only the Agent's own stops produce it) queues nothing. No argument is added. | The monitor already orders a terminal status after its output and before disposal (`monitor_execution`, `plugin-execution/src/monitor.rs:197`; internals "Execution progress"). The kernel knows the session, the call and the turn. The withdrawal covers a `write_stdin` poll that saw the end, since its call id is not the execution's. | Producing it in the execution plugin: it would need per-execution state shared between calls and the monitor, and its own ordering against disposal. An opt-in `notify_on_exit` argument: a schema change in every golden, and a model that forgets it is never resumed. |
| **D12 · Q4** | **The command timeout becomes a setting: `pluginOptions.execution.commandTimeoutMinutes`**, an integer from 1 to 1440, default 10 (today's limit). It bounds every `exec_command` execution, the model's and a `!command`'s, from its start. A change applies to the commands started after it, never to a running one, and restarts nothing. A timeout is a wake event that says how long the command ran (§3.13). | The user's answer to Q4. With terminals waking their owner, a command that outlives its limit is news rather than a poll, and a person who runs long builds or development servers can let them live. The default stays 10 minutes, so a hung command fails, and holds a waiting owner or `exec`, no longer than today unless the person chooses. | Keeping 10 minutes fixed: §1.1's development server would always be killed at 10 minutes. A longer default: every hung command would hold a sub-agent's completion and `exec` that much longer. A `timeout` argument of `exec_command`: a schema change in every golden, and the model, not the person, would choose how long a process lives. |
| **D13 · Q9** | **Manual close.** *Closing a sub-agent* closes its live sub-agents first (depth 2), then interrupts its turn and waits for confirmation, stops its terminals, marks it `Closed`, and queues "closed by the user" on its parent, which wakes. The session stays, so a follow-up task can reuse it. *Closing a terminal* is `stop_confirmed` plus the stop's progress, then "stopped by the user" queued on its owner. `/stop` closes every terminal of the session this way. Archive, delete, side close and shutdown stop terminals and children **without** queuing anything, through the internal path (`quiesce_background_terminals`, split from the facade's `stop_background_terminals_confirmed`). | This is request [4]. The person's close is news for the agent; the Agent's own teardown is not, and its owner is going away. | Archiving a closed sub-agent: closing should not take its history out of the parent's view. Cascading `interrupt_agent`: the model's own interrupt redirects one agent and leaves the rest to it. |
| **D14** | **The host contract** (§3.7). `AgentBackgroundEventSink`, installed at launch, receives `Woken`, `Changed { root }` and `Child { root, session_id, event }`. `Child` forwards a child turn's approval, question and terminal events: child turns get a forwarding sink. The snapshot is `background_tasks(root)`. The calls are `submit_wake_turn_with_sink`, `close_subagent` and `close_background_terminal`. | Polling child journals every 400 ms is how the TUI finds a child's approval today; it does not scale to two levels or to `exec`. | Forwarding every child event (streamed deltas included): a shown sub-agent's transcript is followed from its journal, as today. |
| **D15 · Q6** | **Restart.** The queue is durable. Work that a stop of the Agent ends is reported as *lost* to its owner, by one function run at two moments. At a **clean shutdown**, it runs first in the stage that cancels turns, while history is open. A turn cancelled there is recorded `UserInterrupted` (`TerminalTransition::cancelled`, `turn/terminal.rs:13-21`), like a person's Esc, so it cannot be recognized afterwards. At **start**, after recovery, it covers a crash: turns settled as `ApplicationRestarted` (`recovery/repair.rs:142`) and the terminals `reconcile_recovered_execution_activities` disposes (`background_terminals.rs:117`). Each lost sub-agent is reported to its parent, and each lost terminal to its owner, unless that owner is itself reported lost. Keyed replacement makes the two runs idempotent. Nothing is woken at start: the events are delivered with the session's next turn. | The model is told the truth. The parent's queue is not lost with the process. No turn starts behind the person's back after a crash, which could also feed the crash-loop marker. | Waking at start. No wake events after restart at all: a parent suspended on a child would never learn the child is gone. |
| **D16 · Q5** | **Pause and the runaway bound.** A person's interrupt of a root's turn pauses that root's wakes. More than 100 woken turns in a tree since the person's last message also pauses them. Events still queue, and the status says "paused". The person's next message (a turn with origin `User`) clears the pause and delivers what queued. A host's explicit `submit_wake_turn_with_sink` still runs while paused. The state is in memory. | Esc means "stop for now". A child and its parent can wake each other with messages and follow-ups without end, and each turn costs a model call. | No bound: a loop runs until a budget or the person notices. A per-turn bound: it does not stop a loop of short turns. |
| **D17** | **Goal and user hooks.** `AgentContinuationContext` gains `background: AgentBackgroundWork { agents, terminals }`, which the kernel computes over the loaded sessions of the tree (§3.6). The Goal plugin passes while it is not zero, so the turn can end and the woken turn continues the goal. Stop hooks run at the end of every turn, woken ones included. `UserPromptSubmit` does not run for a woken turn: it has no user message, and the hook context runs only on one (`plugin-hooks/src/context.rs:62`). `SessionStart` is unchanged. | A goal would otherwise push the model to keep going and call `wait_agent`, which defeats the suspension. | Turning Goal continuation off while waiting: the goal's budget and status would drift. |
| **D18 · Q8** | **`exec`.** Today it returns at the root's `TurnCompleted` (`cli/src/exec/mod.rs:180`). It now waits while the root's tree has live work or queued events, and submits each woken turn itself. It exits with the last root turn's outcome and prints that turn's final answer (`--output-last-message`). It answers the tree's approvals and questions with its `--approve` policy, through `Child` events. With the policy off it exits as today. | A model that ends its turn while sub-agents work would otherwise lose them at shutdown. | Exiting at the first turn end: the delegated work would be killed. |
| **D19** | **Excluded sessions.** Side conversations and reviews have the policy off, never spawn and are never woken. A review's parent is told by `start_review`, as today. | Both are driven by a caller that awaits them, or by the person. | Waking a side conversation: it is ephemeral and the person is in it. |
| **D20 · Q11** | **The terminal interface.** `/tasks` (new) opens the background task manager: a tree of the session's sub-agents and its terminals, to inspect and close them. The status-line item `background-tasks` (`2 agents · 1 terminal`, omitted at zero) joins `tui.statusLine`'s default. While the session is suspended, the composer shows `Waiting for 2 agents · 1 terminal — resumes by itself` instead of the idle hint. `/ps`, `/subagents` and `/stop` keep working; their cells end with `/tasks to manage`. The keymap action is `open_tasks`, default `leader t`, with a row in the `?` overlay. | One place for request [4]. The status item is request [5]. The existing commands keep their muscle memory. | Folding the manager into `/agents` (the command center lists roots across the workspace, not one session's work). Replacing `/ps` and `/subagents` (churn with no gain). |

## 3. Design

### 3.1 The wake queue [1][3]

**The event** (kernel, `crates/kernel/src/model/wake.rs` (new)). It is
persisted with the session, so it is tolerant: an unknown kind or a field
that does not decode drops that one event with a warning, never the session
(AGENTS.md, decoding by input source).

```rust
pub struct AgentWakeEvent {
    pub sequence: u64,          // per session, from the session's counter
    pub queued_at: u128,        // ms
    pub source: AgentWakeSource,
}

pub enum AgentWakeSource {
    Subagent {
        session_id: AgentSessionId,
        name: String,               // its task name
        event: AgentSubagentWake,
    },
    Terminal {
        execution_id: u64,          // the model's `session_id`
        command: String,            // the presenter's subject, bounded
        event: AgentTerminalWake,
    },
}

pub enum AgentSubagentWake {
    Completed { answer: Option<String>, truncated: bool },
    Errored { error: String },
    ClosedByUser,
    Lost,                           // stopped by a restart
    Message { text: String },
}

pub enum AgentTerminalWake {
    Ended {
        status: AgentExecutionProgressStatus, // Exited | Failed | TimedOut
        exit_code: Option<i32>,
        duration_millis: u64,
        output: String,             // its last output, plain text
        truncated: bool,
    },
    StoppedByUser { exit_code: Option<i32> },
    Lost,
}
```

**Keys and bounds** (`MAXIMUM_*` constants beside the type):

- **One final status per sub-agent and per terminal.** A newer final event
  of the same `session_id` or `execution_id` replaces the queued one. A
  sub-agent's `Completed` replaces an `Errored` and vice versa, and a
  person's close replaces either. So the queue holds at most one final
  event per child and per terminal.
- **Messages** are appended. At most 16 per session are queued
  (`MAXIMUM_QUEUED_WAKE_MESSAGES`). Past that, `send_message` to `parent`
  is refused: "your parent has 16 unread messages; wait until it reads
  them".
- **Text**: a `Completed` answer is cut at 32 KiB (`truncated`; the rest
  stays in the sub-agent's own conversation). A message is bounded by
  `send_message`'s 64 KiB (`tools.rs:46`). A terminal's output is its
  last 4 KiB, rendered as plain text from the line it starts on
  (`terminal_text::aligned_start`, as the progress queue's coalescing).
  `command` is at most 1 KiB.

**Kernel API** (`crates/kernel/src/service/wake/` (new): `mod.rs` for
queuing and taking, `admission.rs` for the wake turn, `work.rs` for the
counts of §3.6):

| Call | Does |
| --- | --- |
| `enqueue_wake_event(session, source) -> Result<AgentWakeQueued, AgentError>` | Commits the event under the session's lock, as a delta (it is never a revision), and notifies `WakeEventQueued`. Returns `Delivering` when a turn runs (it drains it), `Waiting` when idle, `Disabled` when the policy is off and the source is not a sub-agent's final status (dropped). `SessionArchived` or `SessionNotFound` for a session going away: the caller drops the event. |
| `take_wake_events(session, filter) -> Vec<AgentWakeEvent>` | Removes and returns the matching events: `wait_agent`'s (the final statuses of `parent`'s children) and a terminal's withdrawal (`Terminal { execution_id }`). |
| `submit_wake_turn_with_sink(session, input, sink) -> Result<Option<AgentTurnAccepted>, AgentError>` | Admits a `TurnAdmission::Wake` turn when the queue is not empty and no turn runs, then schedules it as `submit_turn_with_sink` does. `Ok(None)` when the queue is empty or a turn runs (that turn drains it). Refused while the policy is off. |
| `start_wake_turn(session, input) -> Result<Option<AgentTurnOutcome>, AgentError>` | The awaited form, for a sub-agent, as `start_turn` is. |
| `set_session_wake_policy(session, AgentWakePolicy)` | At activation, while idle; no revision. |
| `background_work(session) -> AgentBackgroundWork` | §3.6. |

`AgentSessionRuntimeSnapshot` gains `pending_wake_events: usize` and
`wake_policy`. `AgentNotification` gains `WakeEventQueued { session_id }`
and `BackgroundWorkChanged { session_id }` (the dispatcher is the first
consumer of each, as the observer module requires).

**Admission.** `TurnAdmission` (`service/mod.rs:286`) gains `Wake`, and
`TurnKind` stays `Regular`: a woken turn samples the model like any other.
With the policy on, every admission drains the queue: a `Regular`,
`ImplementPlan` or `Wake` admission commits `WakeEvents` after
`TurnStart`. With it off, no admission drains it, so a sub-agent's final
status waits for `wait_agent` as today. A `UserShellCommand`
admission does not drain it, since that turn samples no model. A `Wake`
admission's `AgentTurnInput.text` is empty and its origin `Programmatic`.
`stage_turn_admission` (`turn/state.rs:116`, 196 of 200 lines) first
moves its opener items, per `TurnAdmission`, into a function of their own
(`admission_items`), which is the split by responsibility the new arm
needs.

**At a step.** `consume_pending_wake_events` runs right after
`consume_pending_steering` (`loop_driver.rs:88`) and the same way. It
first flushes the session's execution progress (`flush_execution_progress`),
so a withdrawal from a poll that just returned is applied. It commits one
`WakeEvents` item, admits it into the turn's budget ledger as one pending
input (`admit_pending_input_items(1)`), and returns whether it drained
anything, which forces a follow-up with the new reason
`AgentFollowUpReason::BackgroundEvents`. With the policy off it drains
nothing. `finish_completed` returns `SteeringPending`'s sibling,
`WakeEventsPending`, if events arrived after the last step's check, and
the loop takes another step, as with steering.

### 3.2 The history item and what the model reads [1][3][4]

`ConversationItem::WakeEvents { events: Vec<AgentWakeEvent> }`, after
`UserShellCommand` (`conversation_item.rs:239`). It is lowered to a
user-role `ContextFragment` of kind `background_events`, rendered by
`wake_events_record` in `model/wake.rs`. The text below is exact; each
event is one block, in queue order:

```text
<background_events>
Updates on work you started in the background: your sub-agents and your background terminals. The user did not write this.

Sub-agent `explore_api` (agent id 41) completed. Its final answer:
<answer>
…the answer…
</answer>

Sub-agent `fix_tests` (agent id 42) failed: model remote error status=Some(500) …

Sub-agent `docs` (agent id 43) was closed by the user: its turn, its terminals and its own sub-agents were stopped. Do not start it again unless the user asks.

Sub-agent `lint` (agent id 44) stopped when the Agent restarted. It keeps its conversation and takes follow-up tasks.

Message from sub-agent `explore_api` (agent id 41):
<message>
…the message…
</message>

Terminal session 12 exited with code 101 after 252 seconds:
<command>
cargo test --workspace
</command>
Its last output (poll session 12 for the rest):
<output>
…
</output>

Terminal session 13 timed out after 600 seconds and was stopped:
<command>
npm run dev
</command>

Terminal session 14 was stopped by the user:
<command>
python -m http.server
</command>

Terminal session 15 stopped when the Agent restarted:
<command>
make bench
</command>
</background_events>
```

- An answer cut at 32 KiB ends `… (cut; the rest is in the sub-agent's
  conversation)`. A failed terminal reads `failed to run` in place of
  `exited with code N`. A terminal without output omits the output block.
- A timed-out terminal says how long it ran, from the event's
  `duration_millis`, as an exit does, and never names the setting: the
  kernel does not read a plugin's options, and the command ran under the
  timeout it started with, which a later change of the setting does not
  alter (D12).
- The item opens a woken turn the way a user message opens a turn, so a
  provider sees a user-role message first.
- **Everywhere else the item goes**, by c930bcc's list (§1):
  - the reviewer reads it as a tool result named `background_events`, as
    it reads `user_shell_command` (`permission.rs:173`);
  - memory reads it as a call of that name (`history/src/store/memory.rs`);
  - hierarchical compression leaves it whole (`compression/adapter.rs:186`);
  - standard compaction summarizes it with the rest;
  - `/context` counts it with the tool calls and outputs, as the
    `!command`'s item (`sampling/composition.rs`);
  - validation accepts it anywhere after a `TurnStart` and pairs no tool
    call with it (`validation/persistence.rs`, `tool_pairs.rs`);
  - export writes a `## Background updates` section, one line per event
    (`service/export.rs`);
  - `cli sessions show` prints it (`cli/src/sessions.rs`);
  - the terminal interface replays it as a "Background updates" cell
    listing the events, each with its first answer or output line
    (`agent_bridge/replay.rs`, `chatwidget/replay.rs`).
- **Golden**: `wake_events_request.json` (new), a woken turn's request with
  one event of each kind, in R1.

### 3.3 What the model is told [1][2][3]

**The sub-agent guidance** (`subagent_guidance`, `prompt.rs:133`). The
first paragraph, for a root:

```text
`spawn_agent` delegates a task to a sub-agent: a separate conversation that runs in the background with your permission mode, and with your model and your tools unless its role says otherwise. At most {N} sub-agents work at once in this conversation, counting the ones they spawn; a sub-agent you spawn can spawn sub-agents of its own, which cannot spawn further.
```

For a depth-1 sub-agent that may spawn:

```text
`spawn_agent` delegates a task to a sub-agent: a separate conversation that runs in the background with your permission mode, and with your model and your tools unless its role says otherwise. At most {N} sub-agents work at once under the main conversation, you and yours included; the sub-agents you spawn cannot spawn sub-agents of their own.
```

The fourth bullet (`prompt.rs:143-144`), with `TurnWake { on: true }`:

```text
- You do not have to wait for sub-agents. When nothing is left for you to do until they report, end your turn: you are resumed in a new turn, with a `<background_events>` update, when a sub-agent finishes, fails or sends you a message, or when the user closes one. Call `wait_agent` only when you need a result before you can go on in this turn.
  - If the user asks a question, answer it first, then continue coordinating sub-agents.
```

A depth-1 sub-agent also reads:

```text
- You are a sub-agent yourself: your final answer reaches the agent that spawned you once you and your sub-agents are done. Use `send_message` with `target` `parent` only for what it must know before then.
```

With the policy off, the bullet is today's ("**wait for them before
yielding**"). `MaySpawn` (`prompt.rs:54`) carries the depth and the policy.
`prepare` reads them, and `contribute` stays pure.

**The background-terminals fragment** (`crates/plugin-execution/src/prompt.rs`
(new): a `PromptContributor` the execution plugin registers, Capabilities
slot, order 140, developer role, with the same "fixed tools" preamble as
the sub-agent guidance). It is shown when the step offers `exec_command`
and the turn's `TurnWake` is on:

```text
<background_terminals>
## Background terminals
A command still running when `exec_command` or `write_stdin` returns keeps running as a background terminal under its `session_id`. You do not have to poll it: when nothing is left for you to do until it ends, end your turn, and you are resumed in a new turn, with a `<background_events>` update, when it exits, times out (after {M} minutes) or is stopped by the user. Poll it with `write_stdin` only when you need its output now. Stop the ones you no longer need (Ctrl+C with `write_stdin`): a terminal left running wakes you when it ends.
</background_terminals>
```

`{M}` is `commandTimeoutMinutes` as the execution plugin holds it at that
step (§3.13), 10 in the goldens. The fragment is contributed per step, so
it follows a change of the setting at the next step. A command started
before the change keeps the timeout it started with, and its wake event
states how long it ran (§3.2), not either value.

**Tool descriptions** (`tools.rs`). They are true in both modes:

- `spawn_agent` (`SPAWN_AGENT_DESCRIPTION`, `tools.rs:380`): "…unless its
  `agent_type` says otherwise; an agent spawned by the main conversation
  can spawn agents of its own, one spawned by another agent cannot. …
  When it finishes, wait_agent returns its final answer, and the sub-agent
  guidance says whether it also reaches you by itself. You can send it
  messages and follow-up tasks meanwhile." This replaces "it cannot spawn
  agents of its own" and "Its final answer is provided to you by
  wait_agent when it finishes".
- `send_message` (`tools.rs:416`): "Send a message to an existing agent,
  or, with `target` `parent`, to the agent that spawned you. An agent that
  is running reads it promptly, at its next message boundary; an idle
  sub-agent reads it with its next follow-up task, and an idle parent is
  resumed to read it. Does not give a sub-agent a new turn." `target`'s
  description adds "or `parent`".
- `wait_agent` (`WAIT_AGENT_DESCRIPTION`, `tools.rs:385`): "…when no agent
  is running" becomes "when none of your agents is still working". A wait
  takes the final statuses from the queue (D5), so a status `wait_agent`
  returned is never delivered again.
- `list_agents`' statuses gain `waiting` (idle while its own sub-agents or
  terminals work) and `closed`.

**Goldens.** R2 changes the 12 that carry `## Sub-agents` and the 14 that
carry `spawn_agent`'s spec. R4 changes the same again (depth). R4b
changes none: no tool description states the command timeout (§3.13).
R5 changes the 14 that offer `exec_command` (the fragment). Each regeneration is on
purpose and reviewed (`HANAFUBUKI_AGENT_UPDATE_GOLDEN=1 cargo test -p
hanafubuki-agent --lib golden`).

### 3.4 Suspension and resumption [1][2][3]

**The dispatcher** (`crates/agent/src/wake/` (new): `dispatcher.rs` and
`pause.rs`). It is an `AgentObserver` registered by a core row,
which cannot be turned off (the option is the switch). **To confirm in
R2**: `agent-host-services` (`host/mod.rs:309-318`) provides only the
`BlockingService` and injects no registry. R2 either has it inject the
observer registry, or adds a core row `wake` (new) beside it
(`host/descriptors.rs:49-55`), whichever the plugin rules allow with less. `observe` does bounded work only: it sends the
session id to a task on the Agent runtime, which:

1. reads the session's snapshot: its origin, whether a turn runs,
   `pending_wake_events`, and its policy;
2. stops unless the queue is not empty, no turn runs, the policy is on and
   the root is not paused;
3. **for a sub-agent**, resolves `SubagentService` (per call, as the facade
   does) and calls `resume(child)`, which runs the woken turn through
   `driver.run_wake_turn` (new, sharing `run_turn`'s preparation:
   Provider, permission mode, world state without a user message, turn
   extensions; `session_driver/mod.rs:102-139`) in the background, with
   the run's status `Running` and its end reported by §3.5's rules;
4. **for a root**, emits `AgentBackgroundEvent::Woken { session_id }` to
   the host's sink;
5. **for a side conversation or a review**, does nothing (D19).

It runs on `WakeEventQueued` (an event on an idle session) and on
`TurnFinished` (events that arrived after the turn's last step). When a
`TurnFinished` belongs to a sub-agent, it also re-checks the parent's
quiet rule (§3.5). Several signals for one session coalesce: the woken
turn drains everything queued by its admission.

**A session's lifecycle**:

```text
           person's message / spawn / follow-up
 idle ───────────────────────────────────────────────▶ running
  ▲  ▲                                                   │  ▲
  │  │ turn ends, nothing live                           │  │ step: drains queue
  │  └───────────────────────────────────────────────────┤  │ (keeps working)
  │                                                      │──┘
  │ turn ends, live work remains (sub-agents/terminals)  ▼
  │◀────────────── suspended ◀───────────────────────────┘
  │                 │  event queued, not paused
  │                 ▼
  │            woken turn (Programmatic, opener = WakeEvents) ──▶ running
  └── last live work ends and its event is delivered
```

- **Running** is a turn of any origin. Its steps drain the queue.
- **Suspended** holds nothing but the queue. A root that is suspended has
  no sink and no task. A sub-agent that is suspended has no `tokio::spawn`
  running: its last turn's task ended.
- **Woken** is an ordinary turn: same deadline (30 minutes), budget,
  permission mode, approvals, continuation hooks and Stop hooks as any
  `Programmatic` turn. Its first request carries `WakeEvents` as its
  opener.
- **Interrupting a woken turn** (Esc, `cancel_turn`) cancels it as any
  turn. The events it drained are already in history. D16 pauses the
  root.

**Pause** (`wake/pause.rs`, in memory, per root):

- `paused_by_person` is set when a person cancels a turn of the root
  (`TurnCancelled` with `UserInterrupted`, seen by the dispatcher through
  `TurnFinished` and the snapshot's terminal reason).
- `woken_since_person` counts the woken turns of every session in the tree
  since the root's last `User` turn; past 100 (`MAXIMUM_WOKEN_TURNS`, a
  constant, as Q5 decided) the root is paused.
- A `TurnAdmitted { origin: User }` of the root clears both. The
  facade's `background_tasks(root)` reports `wake: On | Paused { reason }
  | Off`.
- Paused roots and their sub-agents queue events, but nothing is woken.
  The next person's message delivers the root's queue. A sub-agent's queue
  is delivered when its tree is resumed: the next `User` turn of the root
  re-runs the dispatcher over the tree's suspended sub-agents.

### 3.5 Nesting, limits and a child's end [2]

**Depth** (`crates/agent/src/plugins/subagents/tree.rs` (new)): `depth(session)`
walks `parent_session_id` through the driver's snapshots, at most two
steps, and is checked against `AgentSessionOrigin::Subagent` at each step.
`spawn` refuses, in this order, before anything is created:

| Case | The tool error |
| --- | --- |
| a side conversation or review | `this conversation cannot spawn agents` |
| depth 2 | `a sub-agent spawned by another sub-agent cannot spawn agents: sub-agents nest at most two levels below the main conversation; do the work yourself, or send your parent a message` |
| the cap (D10) | `at most {N} sub-agents work at once under the main conversation, counting theirs; wait for one to finish, or interrupt or close one` |

The task name stays unique per parent.

**Live agents across the tree.** `SubagentService::live(root)` counts the
tree's sub-agents that run a turn, or are idle with
`background_work(session) > 0` (§3.6). It reads `SubagentRuns` for this
instance's runs and the kernel for the rest. `ensure_capacity` and
`start`'s check (`service.rs:438`, `:456-465`) use it, under the runs'
lock as today, so two spawns at once cannot both pass.

**A child's end** (D8) is decided in `SubagentRuns::finish` (`service.rs:169`):

| Outcome of the child's turn | Its subtree | Queued on the parent |
| --- | --- | --- |
| Completed | quiet | `Completed { answer }` |
| Completed | live | nothing yet. When the subtree's last live work ends and the child is idle with no queue, `Completed` with the child's latest answer (`driver.last_answer`) |
| Errored | any | `Errored { error }` |
| Cancelled by `interrupt_agent` | any | nothing (as today) |
| Cancelled by a person's close, or by `exec` for a question no one can answer | — | `ClosedByUser` (from §3.9, not from here) |
| `ApplicationRestarted` | — | `Lost` (from §3.10) |

"The subtree's last live work ends" is the dispatcher's `TurnFinished`
re-check and `BackgroundWorkChanged` for the child: it calls
`SubagentService::settle(child)`, which queues the held `Completed` when
the child is idle, its queue is empty and `background_work(child)` is zero.
A child that is woken and whose woken turn ends goes through the same
rule.

**What becomes of `SubagentRuns`.** `unreported` and `changed` go (D5).
`wait` waits on the kernel's `WakeEventQueued` for the parent (a `Notify`
the dispatcher signals per session), then takes the final statuses with
`take_wake_events`. `Idle` ("No agent is running.") becomes "none of your
agents is still working": no live child and no queued final status.
`AgentRunStatus` gains `Waiting` (shown, never stored: idle with live
work) and `Closed`.

**`AgentSubagentStatus`** (`subagents/mod.rs:45`, after O3 adds
`provider_id`, `model` and `reasoning_effort`) gains `parent_session_id`
and `depth`. `AgentSubagentState` (`:57`) gains `Waiting` and `Closed`.
`subagents(parent)` keeps listing direct children; the tree is
`background_tasks(root)` (§3.7).

**Module size.** `service.rs` becomes `service/mod.rs` (the tool
operations), `service/runs.rs` (`SubagentRuns`, a child's end, the held
completion) and `service/statuses.rs` (the facade's statuses), with
`tree.rs` beside it for depth, live counts and the close cascade. The
split follows responsibilities that already exist; nothing is cut to fit.

### 3.6 Background work, counted [1][5]

`AgentService::background_work(session) -> AgentBackgroundWork { agents,
terminals }` (`service/wake/work.rs` (new)) is the kernel's own count, so
the Goal hook (D17), the dispatcher and the status indicator agree:

- **terminals**: the session's execution activities whose status is not
  terminal and whose call is not in flight (D11's test), plus the same
  over its live sub-agents. **Terminals are counted from R5 on**, the
  milestone in which they wake their owner. Before it, a running terminal
  would hold a Goal turn, a child's completion or the waiting line on
  something that never resumes it, so R2–R4 count agents only;
- **agents**: its `Subagent` children that run a turn or whose own
  `background_work` is not zero, counted recursively (two levels).

It reads loaded sessions only: a session that runs a turn, or has an
execution, is loaded. A child that is not loaded is idle with nothing
running. The count is computed per call under each session's lock in
turn, never all at once. `BackgroundWorkChanged { session_id }` is
notified when an execution starts or ends, when a sub-agent's turn starts
or ends, and when a wake event is queued or drained. The dispatcher maps
the session to its root and emits `Changed { root }`.

### 3.7 The facade and the host [1][4][5]

**The sink**, installed at launch: `AgentServiceInputs.background_event_sink`
(new, beside `plugin_event_sink`, `service/mod.rs:95`) and the standalone
launch's `background` (beside `status`, `standalone/mod.rs:65`). Both
default to a no-op.

```rust
pub trait AgentBackgroundEventSink: Send + Sync {
    /// Bounded, non-blocking work, as `AgentEventSink::emit`.
    fn event(&self, event: AgentBackgroundEvent);
}

pub enum AgentBackgroundEvent {
    /// A root has wake events and no turn: `submit_wake_turn_with_sink`
    /// starts the turn that reads them. Ignoring it leaves them queued
    /// for the session's next turn.
    Woken { session_id: AgentSessionId },
    /// The tree of `root` changed: an agent or a terminal started or
    /// ended, an event was queued or delivered, the wake was paused or
    /// resumed. Read `background_tasks(root)` again.
    Changed { root: AgentSessionId },
    /// An event of a sub-agent's turn, anywhere in the tree:
    /// `ApprovalRequested`, `ApprovalResolved`, `UserInputRequested`,
    /// `UserInputResolved`, and the turn's start and terminal events.
    Child { root: AgentSessionId, session_id: AgentSessionId, event: AgentEvent },
}
```

`Child` comes from a forwarding `AgentEventSink` that `SubagentService`
passes to the child's turn (`run_turn` and `run_wake_turn` take a sink,
in place of `start_turn`'s no-op). It forwards only the kinds listed, so
streamed deltas never cross. The journal still has everything, and a
sub-agent shown in the terminal interface is still followed from it.

**The calls** (`AgentHandle`; `crates/agent/src/service/background.rs`
(new)):

| Call | Returns / does |
| --- | --- |
| `background_tasks(root)` | `AgentBackgroundTasks { work, agents, terminals, pending_wake_events, wake }`. `agents` is a `Vec<AgentSubagentStatus>` over the tree in tree order, with `parent_session_id`, `depth` and the elapsed time of the current turn or wait. `terminals` is a `Vec<AgentBackgroundTerminal { owner: AgentSessionId, owner_title: Option<String>, terminal: AgentBackgroundTerminalSnapshot }>`, each owner's `list_background_terminals`. `wake` is `On`, `Paused { reason: PersonInterrupted \| RunawayBound }` or `Off`. Async: the facade reads the tree on its blocking pool. |
| `submit_wake_turn_with_sink(session_id, sink)` | `Option<AgentTurnAccepted>` (§3.1), for a root. The facade prepares it as `submit_turn_with_sink` does. |
| `close_subagent(session_id)` (async) | §3.9. `InvalidInput` for a session that is not a sub-agent. |
| `close_background_terminal(session_id, execution_id)` | §3.9. `InvalidInput` for an execution that is not one of the session's background terminals. |
| `stop_background_terminals_confirmed(session_id)` | Unchanged signature. Now the person's "close all": each stopped terminal queues "stopped by the user" (D13). |
| `interrupt_subagent(session_id)` | Unchanged: interrupts the turn, reports nothing, cascades nothing. |

**Who is told what**:

- **The terminal interface**: `Woken` for the session it shows (or the
  sub-agent or side conversation it shows beside it) submits the wake turn
  with the bridge's sink. For another session it marks the session, whose
  command-center row shows `N updates`, and the events wait for that
  session's next turn. `Changed` re-reads the snapshot (the manager, the
  status item). `Child` drives a child's approvals and questions.
- **`exec`**: §3.11.
- **Hanafubuki**: implements the sink (a channel to its frontend), submits
  wake turns for the sessions it decides, and adapts to the new
  `AgentServiceInputs` field, `AgentSubagentState`'s variants and the new
  history item at its next pin.

### 3.8 Terminal wakes [3]

The kernel stages execution progress and terminal tracking together. The
runtime map records each model-started execution by id and generation,
with its command, whether it ended and whether its end is queued.

| Update | Condition | Wake |
| --- | --- | --- |
| `ExecutionStarted` | from a model tool call, not `user-shell-…`; the activity accepted this start | track the execution and the starting call's presentation subject |
| First terminal `ExecutionStatus`: `Exited`, `Failed`, `TimedOut` | matching tracked generation, from a tool sink, wake policy on, session not archived | queue `Terminal { Ended { … } }` with duration and last output |
| `ExecutionStatus` `Cancelled`, or the Agent's own report | — | mark ended, queue nothing |
| `ExecutionDisposed` from a tool | matching tracked generation | remove tracking and withdraw its queued end: the call result reported it |
| `ExecutionDisposed` from the Agent | matching tracked generation | remove tracking, preserving an end already queued: no call reported it |

An in-flight call set is deliberately unnecessary. A command can exit
after `exec_command` has returned its session id but before the kernel
commits the result. Ignoring an end while that call is in flight loses
the update. Instead, the disposal proves that the call reported the end;
steps flush progress before delivering wake events, and turn completion
also flushes progress. A command that finishes within its call, or a
`write_stdin` poll that reports its end, therefore leaves no duplicate
update. Tests cover the yield-to-commit interleaving explicitly.

Tracking is not persisted: no process survives this Agent run. It is
part of the session runtime rollback, so a failed append restores it and
the queue together. Duplicate starts do not reset it, repeated statuses
queue once, and a stale generation cannot dispose another execution.
The command comes from the starting tool's presenter, evaluated outside
the session lock. Output is the activity's plain-text tail, bounded by
the queue to 4 KiB. Nothing changes in `activity.rs`.

### 3.9 Manual close [4]

`close_subagent(child)` (`subagents/tree.rs`, through `SubagentService`):

1. For each live sub-agent of `child` (depth 2), steps 2–4 for that
   sub-agent, with no event queued on `child` (it is being closed too).
2. Interrupt its turn and wait for the confirmation (`driver.interrupt`,
   `session_driver/mod.rs:146`).
3. Stop its terminals through the internal path (no wake on itself).
4. Set its run `Closed`, drain and drop its own queue (its owner is
   closed).
5. Queue `ClosedByUser` on `child`'s parent, which wakes it.

`close_background_terminal(owner, execution)`:

1. `stop_confirmed` (`execution/src/service/lifecycle.rs:35`).
2. In one kernel checkpoint, publish the stop’s status and disposal and
   replace any queued exit with `StoppedByUser { exit_code }`. This avoids
   a running owner consuming an exit between disposal and its replacement.
3. Notify `owner` once after the commit (a sub-agent through the dispatcher,
   a root through its host). Only tracked model-started terminals report;
   foreground `!` commands do not.

`stop_background_terminals_confirmed(owner)` (the facade's, `/stop`) is
step 1–3 for each. `quiesce_background_terminals`
(`background_terminals.rs:108`), which archive, delete and side close use,
calls `stop_background_terminal_authority` and the publication directly,
and queues nothing.

### 3.10 Persistence and restart [1][3]

- **What is durable**: the queue (the session's runtime, D2); delivered
  events (history); the policy is reconciled at activation.
- **What is not**: `SubagentRuns` (rebuilt from the kernel, as today), the
  pause state, the dispatcher's signals.
- **At start**, after recovery and `reconcile_recovered_execution_activities`
  (`agent/src/service/mod.rs:532`), the new `report_lost_background_work`
  (`service/background.rs`):
  1. collects the sessions that lost work: a turn settled as
     `ApplicationRestarted`, or a background terminal the reconciliation
     disposed;
  2. closes the set upward: a sub-agent whose child lost work lost work
     too (two levels);
  3. for each lost sub-agent, queues `Lost` on its parent unless the parent
     is itself a lost sub-agent. For each lost terminal of a session that
     is not a lost sub-agent, queues the terminal's `Lost` on it.
- **Nothing is woken at start.** The dispatcher ignores the events queued
  before the instance's start completes. Recovered events are delivered
  with the session's next turn. The terminal interface's status item shows
  them as `N updates`.
- **At a clean shutdown**, the same `report_lost_background_work` runs
  before turns are cancelled. It is the first step of the stage that
  cancels them (`stop_operations`, `kernel/src/service/lifecycle.rs:16`,
  is called after it), within that stage's 5 s. It takes the sub-agents
  with live work (from `SubagentRuns` and `background_work`) and the
  running background terminals, and queues `Lost` on their owners by the
  same rule. It must run there because the cancellation `stop_operations`
  makes is recorded `UserInterrupted` (`turn/terminal.rs:13-21`), so
  afterwards a quit and a person's Esc look the same. The sub-agents' runs
  are then cancelled, and their `finish` queues nothing (D8: a cancel
  reports nothing).
- **A crash** (no shutdown) leaves turns that recovery settles as
  `ApplicationRestarted`, and terminals the start's reconciliation
  disposes. The start's run reports them. After a clean shutdown the
  start's run finds the same terminals still recorded and queues the same
  keys, which replaces what is queued (§3.1), so nothing is reported twice.

### 3.11 `exec` [1][2][3]

- `exec` installs a background sink that forwards to its channel
  (`cli/src/exec/mod.rs`, `Message::Background`).
- After the root's turn ends with `TurnCompleted`: if
  `background_tasks(root)` shows live agents, running terminals or queued
  events and the wake is `On`, it keeps reading. On `Woken` it submits the
  wake turn with its sink, and the turn's events print as any turn's. It
  ends when the tree has no live work, no queued events and no turn runs,
  with the last root turn's outcome and final answer.
- `Child` approvals and questions are answered by `--approve`, as the
  root's are, and printed with the sub-agent's title (`· approval denied:
  sub-agent explore_api: exec_command: …`). A question no one can answer
  closes **that sub-agent** (`close_subagent`, `exec` standing in for the
  person, from R6; before R6 it interrupts the sub-agent's turn). The run
  goes on, and the parent is told it was closed.
- Ctrl-C cancels the running turn and closes every live sub-agent and
  terminal of the tree, then exits 130.
- With the wake `Off` or `Paused`, it exits at the root's turn end, as
  today.
- `exec --json` prints the woken turns' events as any turn's; there is no
  new event kind. The `history_committed` that carries the item names it
  (`background_events`), which the compatibility rule allows (a new value
  may appear, `json-events#stability`).

### 3.12 The terminal interface [4][5]

**The manager** (`crates/tui/src/tasks/` (new): `mod.rs`, `render.rs`,
`input.rs`, `tests.rs`). It is a full-screen view, as the command center
is (`agents_center/`). It reads `background_tasks(root)` off the loop and
again on `Changed`. It has two sections:

- **Agents**, as a tree: `▸ explore_api  running · 3m12s · gpt-5.4 ·
  explorer`, then its children indented. States: running, waiting (`2
  agents · 1 terminal`), idle, failed, closed. The detail pane shows the
  task (its first message), the last answer excerpt, the role, the model
  (O3's fields) and its own terminals.
- **Terminals**: `#12  cargo test --workspace  running · 4m02s ·
  explore_api · ~/repo`, and in the detail pane the recent output
  (`recent_output`, 3 lines in the list, all in the detail).
- **Keys**: Enter shows a sub-agent (as `/subagents`' picker does) or
  expands a terminal's output. `x` closes the selected sub-agent or
  terminal after a confirmation (`Close explore_api and its 2 sub-agents?
  It is told it was closed.`). `X` on the Terminals heading is `/stop`.
  `r` resumes a paused wake (`submit_wake_turn_with_sink`). Tab switches
  sections. Esc leaves.
- **Footer**: `wake: on` / `wake: paused (you interrupted) · r resumes` /
  `wake: off (wakeOnBackgroundTasks)`.

**Commands** (`commands/tasks.rs` (new); `COMMANDS` grows to 51,
`commands/mod.rs:91`):

| Command | Does |
| --- | --- |
| `/tasks` | Opens the manager. `during_task: true`. |
| `/ps` | As today, plus the cell's last line `/tasks to manage`. |
| `/stop` (`/clean`) | As today, plus each stopped terminal wakes the session (D13); the confirmation's subtitle says so: "Every background terminal of this session is stopped, and the agent is told." |
| `/subagents` | As today, over direct children, with the new states. Its cell ends `/tasks to manage`. |

**The keymap**: `open_tasks` (Global, `leader t`; `keymap/catalogue.rs`)
and a row under the `?` overlay's Session heading, `leader t  Background
tasks`.

**The status item** (`appearance/items.rs`): `StatusLineItem::BackgroundTasks`,
id `background-tasks`, description "Running sub-agents and background
terminals (omitted when none)". Its value is `2 agents · 1 terminal`,
`1 agent`, `3 terminals`, or `paused · 2 agents` while paused, from
`SessionStatus.background` (new, `agent_bridge/status.rs`), refreshed on
`Changed`. It joins `DEFAULT_STATUS_LINE` (`crates/config/src/tui.rs:118`)
after `model-with-reasoning`. A person who set `tui.statusLine` keeps their
list; `/statusline` offers the item.

**The suspended state.** While the session shown runs no turn and its
snapshot's `background` is not zero, the status indicator
(`status_indicator_widget.rs`) shows, without a spinner, `Waiting for 2
agents · 1 terminal — resumes by itself · /tasks`. When paused it shows
`Paused: 2 agents · 1 terminal still running — send a message to resume`.
The composer stays editable: a message goes as a `User` turn and carries
the queued events (D3).

**The bridge.** `agent_bridge/background.rs` (new) holds the background
sink's receiver and the tree snapshot. It replaces the 400 ms watch of
`subagents.rs` (`run_watch`, `:308`): approvals come from `Child` events,
statuses from `Changed`. `agent_bridge/mod.rs` (596) gains only the module
line and the dispatch arm: R3 first moves the session-creation part of its
`Op` handling (`create_session` and the ops it replays, `:345-447`) into
`agent_bridge/start.rs`, where the session start already lives. The
following of a sub-agent shown beside the session (`run_follow`,
`:416`, 150 ms) stays.

### 3.13 The command timeout [3]

D12, as the user decided Q4. The references in this section are at
`ed2828f`.

**Today.** `DEFAULT_COMMAND_TIMEOUT_MS` (`crates/execution/src/resolver.rs:10`,
10 minutes) is the plan's `timeout_ms` for every `exec_command`
(`prepare_exec_command`, `crates/execution/src/shell.rs:116`), and the
approval's canonical bytes carry it (`CanonicalShellAction.timeout_ms`,
`shell_action`, version `agent-shell-v5`). `MAX_COMMAND_TIMEOUT_MS`
(30 minutes, `crates/execution/src/command.rs:28`) bounds only the argv
harness of the crate's contract tests (`StartCommandRequest`; `mod
command` is compiled for tests and `test-support` only), which is the
default's other user. The harness keeps both. The
executor enforces any `timeout_ms` with no ceiling of its own
(`spawn_timeout`, `crates/process/src/executor/backend_common.rs:89`).
The `execution` row has no options (`AgentPluginRow::unconfigured`,
`crates/agent/src/host/manifest.rs:102`).

**The option**, the first of `[pluginOptions.execution]`:

| Key | Kind | Default | Label and summary |
| --- | --- | --- | --- |
| `commandTimeoutMinutes` | An integer, 1 to 1440 | `10` | "Command timeout": "How long a command may run before it is stopped (minutes)" |

- **Where it is declared.** A `PluginOptionSchema` is
  `hanafubuki-agent-packages`', which the execution plugin's crate may not
  depend on (its row in `xtask/src/dependencies.rs`). The schema is built
  in the agent crate instead: `execution_option_schema()` (new) in
  `crates/agent/src/host/descriptors.rs`, whose `execution` row takes
  `.with_options(…)`, as `subagents` takes `subagents_option_schema()`.
  Its bounds and default are constants `hanafubuki-agent-plugin-execution`
  exports (`COMMAND_TIMEOUT_MINUTES_LIMITS`, `DEFAULT_COMMAND_TIMEOUT_MINUTES`),
  so the schema's default and the plugin's are one value.
- **How it reaches the plugin.** The manifest row becomes
  `AgentPluginRow::from_settings(Capability, execution_plugin(…), |settings|
  settings.options_value("execution"))`, and `execution_plugin` becomes
  `PluginDef::typed` over `ExecutionOptions { command_timeout_minutes }`
  (`Deserialize`, camelCase, `default`; the crate gains `serde` with
  `derive`). The value arrives in range: the settings boundary clamps a
  file's value with a warning and refuses a write out of range
  (`config-reference#plugin-options`), so the plugin checks nothing again.
- **A change is taken in place.** `ctx.on_update` stores the new value in
  `ExecServices` (an `AtomicU64` of milliseconds) and returns `true`. The
  plugin never restarts for it: a restart would stop the output monitors
  of running terminals (its `execution monitors` disposer), and no tool
  spec states the limit (`crates/plugin-execution/src/specs.rs`).
- **How it reaches a command.** `ExecCommandRequest` gains
  `timeout_ms: u64`. `parse` (`crates/plugin-execution/src/arguments.rs`,
  where the request is built) takes the value `parse_call`
  (`crates/plugin-execution/src/tool.rs`) reads from `ExecServices`, and
  `prepare_exec_command` puts `request.timeout_ms` in the plan in place of
  `DEFAULT_COMMAND_TIMEOUT_MS`. With no production user left, that
  constant moves from `resolver.rs` into `command.rs`, beside the argv
  harness that still uses it.
  The canonical bytes keep their shape and `agent-shell-v5`: an approval
  is of the command with its timeout. `AllowForSession` allows a tool,
  not a digest, so a change does not undo it.
- **A change between a call's approval and its start.** `permission` and
  `invoke` each parse the call, and the start prepares it again and checks
  the exact approval (`start_exec_command`,
  `crates/execution/src/service/start.rs`). A change of the setting in
  between fails that one call's check, and the model reads the error and
  calls again. It fails closed, it needs the person to change the setting
  while the call waits, and it gets no code of its own (AGENTS.md on
  unlikely races).
- **Scope.** Every `exec_command` execution: a model's call and the
  background terminal it leaves, a sub-agent's, and a `!command`, which
  runs through the same tool (`run_user_shell_command`,
  `crates/kernel/src/service/tool_execution/user_shell.rs`). A `!command`
  holds its tool call until it exits (`run_to_exit`), so the turn's tool
  timeout (`[modelPolicy.turnBudget].toolIdleTimeoutSeconds`, 600 by
  default, `crates/kernel/src/service/tool_execution/handler.rs:147`) and
  the turn's 30-minute deadline (`drive_turn`,
  `crates/kernel/src/service/turn/worker.rs`, which covers a `!command`'s
  turn too) still stop it first when they are shorter: the
  setting never lengthens a `!command` past them. Hooks' commands, MCP
  servers and Git queries keep their own limits.
- **Running commands** keep the timeout their plan had at their start
  (the executor's timer). A change applies from the next `exec_command`.
- **What the model reads.** Nothing changes in R4b: no tool description
  names the limit, and a timed-out call's result is as today. From R5, the
  `background_terminals` fragment states the current value (§3.3), and a
  timed-out terminal's wake event states how long it ran (§3.2).

## 4. Edge cases

| Case | What happens |
| --- | --- |
| The person types while the session is suspended | Their message is a `User` turn. Its admission drains the queue into `WakeEvents` before the message (D3). It clears a pause (D16). A `Woken` that arrives meanwhile finds the queue empty or a turn running: `submit_wake_turn_with_sink` returns `None`. |
| The person types while a woken turn runs | It is steering, as for any turn. |
| A wake arrives while an approval or a question of the running turn waits | The turn is running, so the event waits for the step after the answer. Nothing is woken. The status item shows the work. |
| A child's approval while the root is suspended | `Child` reaches the host. The terminal interface shows it as today, labelled `Subagent NAME:` (now for depth 2 too). `exec` answers it by `--approve`. A host without a sink leaves it waiting until the child's turn deadline, as today. |
| The person interrupts a woken turn | It is cancelled as any turn. The drained events are in history. The root is paused until the person's next message (D16). Its sub-agents keep working. |
| The person interrupts the root while sub-agents work | Same pause. Children are not interrupted (no cascade, as today). Their final statuses queue and are delivered with the next message. |
| The person closes a sub-agent or terminal of a suspended root | Its event wakes the root, unless paused (§3.9). |
| The person closes a sub-agent whose parent is a suspended sub-agent | The parent is woken by the dispatcher; its woken turn's end follows §3.5 toward the root. |
| `/stop` with five terminals | Five `StoppedByUser` events, one woken turn. |
| Archive or delete of a session with live work | Its terminals and children stop through the internal path. Nothing is queued on it. Its children's ends find their parent archived: `enqueue` returns `SessionArchived` and the event is dropped. |
| Events for a closed side conversation, a review, a deleted session | Never queued (D19), or dropped on `SessionNotFound`. |
| Parent in Plan mode | A woken turn runs in the session's mode. Plan mode offers `wait_agent` and `list_agents` only, and reads the events like any update. |
| Parent in Manual mode, the terminal interface on another session | The root is not woken (the interface submits only for what it shows). Its row in `/agents` shows `N updates`. Opening it shows the waiting state; the next message delivers them. |
| Parent in Manual mode under `exec` | Woken turns run with `exec`'s answers (§3.11). |
| Parent in a host | The host decides on `Woken`. Not submitting leaves the events for the next turn. |
| `exec` and a terminal that never exits | `exec` waits until the command timeout (`commandTimeoutMinutes`: 10 minutes by default, at most 24 hours; D12), then the woken turn sees `timed out`, and `exec` ends if nothing else is live. The guidance says to stop terminals no longer needed (§1.1). |
| `exec` Ctrl-C | Cancels, closes the tree's work, exits 130 (§3.11). |
| Wake option turned off with events queued | Nothing more is queued from terminals or messages, and no session is woken. Nothing is dropped and nothing is delivered by a turn: `wait_agent` takes the sub-agents' final statuses as today, and messages and terminal ends wait until the option is on again. |
| A sub-agent ends its turn with a terminal still running | Its `Completed` is held until the terminal ends: at most the command timeout (`commandTimeoutMinutes`: 10 minutes by default, at most 24 hours; D12). Today its parent would hear at once. The guidance tells it to stop terminals it no longer needs. |
| The person changes `commandTimeoutMinutes` while terminals run | They keep the timeout they started with; commands started afterwards take the new one, and the fragment states it from the next step (§3.13). Nothing restarts. |
| Wake option turned on while sub-agents work | From the next activation of each session. A sub-agent's final status already queued is delivered at the parent's next step or wake. |
| A child finishes while its parent is in `wait_agent` | `wait_agent` takes its status from the queue. It is never delivered twice. |
| The model polls a terminal whose end is queued but not delivered | The poll's disposal withdraws the event (§3.8). |
| A sub-agent messages its parent 17 times without being read | The 17th `send_message` fails: "your parent has 16 unread messages…". |
| Parent and child message each other without end | Each wake counts toward the tree's 100 woken turns; past them the root is paused (D16). |
| Two wakes race (dispatcher on `TurnFinished` and on `WakeEventQueued`) | The second `submit_wake_turn` or `resume` finds the queue empty, or a turn running, and does nothing. |
| A sub-agent at the cap is woken | Allowed: it is already live (D10). |
| `followup_task` to an idle sub-agent at the cap | Refused with the cap's text. A follow-up to a waiting one (already live) is allowed. |
| The woken turn's context window is nearly full | It compacts first, as any turn. The item compacts with the rest; hierarchical compression keeps it whole. |
| Goal active, the model ends its turn with sub-agents working | The Goal continuation passes (`background > 0`). The woken turn continues the goal. A Stop hook can still keep the turn going. |
| Restart with a suspended root and a running child | §3.10: the child is lost. The root's queue gains `Lost`, and the next message delivers it. Nothing starts by itself. |
| Restart with a pending (undelivered) event | It is durable and is delivered with the next turn. |
| A host's per-turn sink is closed when `Woken` comes | Nothing uses it: the host submits the wake turn with a sink of its choice. |
| The Agent restarts automatically after a kernel fault | Same as a restart: nothing is woken while starting. The new instance's dispatcher starts with no pause state. |

## 5. The questions, answered

The user answered every question on 2026-11-13: each as recommended,
except Q4, where the user chose the alternative. Each entry keeps the
question and records the decision; none is open.

- **Q1 — the option.** One key, `wakeOnBackgroundTasks` (General,
  boolean, default `true`), for sub-agents and terminals, also in a
  host's `config.toml` and the settings center's General section; or two
  keys (`wakeOnSubagents`, `wakeOnTerminals`)? **Decided by the user on
  2026-11-13, as recommended: the one key** (D5). Nothing in the request
  asks for two.
- **Q2 — what the cap counts.** Live agents across the tree (running or
  waiting on their own work); or running turns across the tree, which
  frees a slot while an agent is suspended but makes a woken turn exceed
  the cap? **Decided by the user on 2026-11-13, as recommended: live
  agents across the tree** (D10). 8 means at most 8 sub-agents with
  unfinished work under one conversation, and woken turns never exceed
  it.
- **Q3 — the range.** **Decided by the user on 2026-11-13, as
  recommended: 1..16 unchanged**, default 8 (D10).
- **Q4 — the command timeout.** Keep 10 minutes for this plan, as the
  plan recommended; or make it an option,
  `pluginOptions.execution.commandTimeoutMinutes` (1–1440, default 10)?
  **Decided by the user on 2026-11-13: the option** (D12, designed in
  §3.13, landed by R4b). Its default stays 10 minutes. It bounds every
  `exec_command` execution, a `!command`'s included, and a change applies
  to the commands started after it, never to running ones. A longer
  setting lets a development server or a long build outlive 10 minutes,
  and holds `exec` and a waiting owner that much longer (§1.1).
- **Q5 — runaway bound.** Pause after 100 woken turns in a tree since the
  person's last message; or no bound; or an option? **Decided by the user
  on 2026-11-13, as recommended: 100 woken turns per tree, a constant**
  (D16, `MAXIMUM_WOKEN_TURNS`).
- **Q6 — restart.** A durable queue, `Lost` events for work a restart
  stopped, and no wake at start; or no durable queue and no restart
  events (about 150 fewer lines, but a parent suspended on a child never
  learns it is gone)? **Decided by the user on 2026-11-13, as
  recommended: the durable queue, `Lost` events and no wake at start**
  (D15).
- **Q7 — messages from a child.** `send_message` with `target` `parent`,
  at most 16 unread; or a `message_parent` tool? **Decided by the user on
  2026-11-13, as recommended: `send_message` to `parent`, at most 16
  unread** (D7).
- **Q8 — `exec`.** Wait for the tree (agents, terminals, queued events),
  answer the tree's approvals by `--approve`, and exit with the last root
  turn's outcome; or wait for agents only and stop terminals at the end,
  which drops a model's `cargo test` it left to wake it? **Decided by the
  user on 2026-11-13, as recommended: `exec` waits for the tree and
  answers its approvals and questions with `--approve`** (D18, §3.11).
- **Q9 — what closing a sub-agent does.** Stop its subtree's turns and
  terminals, keep its session (it takes follow-up tasks) and tell its
  parent; or archive it as well? **Decided by the user on 2026-11-13, as
  recommended: stop, keep the session, tell the parent** (D13).
- **Q10 — `SubagentStart` and `SubagentStop` user hooks.** Support them,
  or keep them unsupported? **Decided by the user on 2026-11-13, as
  recommended: still unsupported, with a correct reason** ("the Agent
  runs no hook when a sub-agent starts or stops"). Supporting them is a
  separate request (§8).
- **Q11 — names in the terminal interface.** `/tasks`, the
  `background-tasks` status item (on by default, omitted at zero) and
  `leader t`; or `/background`; or `/ps` opening the manager? **Decided
  by the user on 2026-11-13, as recommended: `/tasks`,
  `background-tasks` and `leader t`** (D20).
- **Q12 — order.** After O3; or before M1, which moves O1–O3 after R8 and
  has O's plan re-read for the split `service/` module and the new
  guidance? **Decided by the user on 2026-11-13, as recommended: after
  O3** (§6, "Order").

## 6. Milestones

### Order

Strictly sequential. Each milestone is one commit on main, pushed, with
green CI, before the next starts:

**(shortcuts plan: K3 → K4 → I1 → M1 … M8) → (O1 → O2 → O3) → R1 → R2 →
R3 → R4 → R4b → R5 → R6 → R7 → R8**

- **After O3** (Q12). O1–O3 edit the same files as R2–R4:
  - `subagents/{tools,service,mod}.rs` (`spawn`, `SpawnRequest`,
    `list_agents`, `AgentSubagentStatus`);
  - `prompt.rs` (O2 adds the Models part under the roles; R2 and R4
    rewrite the paragraph and the bullet above it);
  - `commands/subagents/{mod,picker}.rs` (O3; R4 and R8 add states).

  O's plan was decided against today's `service.rs`; R4 splits it. Landing
  O first keeps an approved plan valid, and R4 splits the module with O's
  code in it.
- **After K4 and I1**: R8 adds a keymap action and an overlay row, which
  K3, K4 and I1 also edit (`keymap/catalogue.rs`, `shortcut_help.rs`).
- **After M7**: M7 (SGR colour in command output) changes "the progress
  event" (its plan's file list) to carry a styled copy of command output.
  R5 reads execution progress in the kernel to queue terminal ends, and
  its tail of output must stay the plain text. Whether M7 also edits
  `kernel/src/service/progress.rs` is to be confirmed when M7 lands; the
  order avoids the question.
- **R4b, its own milestone right before R5** (Q4). The setting does not
  depend on the wake, and changes nothing the model reads, so it lands
  as a small commit with no golden of its own. R5's fragment then states
  its value, and R5 stays about terminal wakes. Its label keeps R1–R8's
  numbers, as T1b did in the settings plan. W1, which also edits
  `crates/execution/src/shell.rs`, is on main already (`981765d`).

Each milestone records in `todo.md`, when done:

- what landed, and any deviation from this plan;
- the validation commands that ran and those skipped;
- the CI run's URL (Linux, Windows and macOS from the workflow, never
  claimed from a Linux run);
- whether goldens changed;
- what Hanafubuki must change at its next pin;
- any module-size exception added or raised (none is planned).

Every milestone runs AGENTS.md's validation matrix (`cargo fmt --all
--check`; `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
--all-features`; `cargo clippy … -D warnings`; `cargo xtask check`; `cargo
test --workspace --all-features`). When it touches `site/`, it also runs
the site's (`bun install --frozen-lockfile`, `bun run format:check:site`,
`bun run site:check`, `bun run site:test`, `VITE_BASE_PATH=/hanafubuki-agent/
bun run site:build`, every page prerendered). Each milestone documents
what it changes, in both languages.

**Tests and platforms.** Agent-level tests run on the fake model server
(`crates/agent/src/test_support/model_server.rs`). Terminal tests use
commands that work under both the POSIX shell and PowerShell, as
`tests/executions.rs` does, and wait on files rather than on time. The
history-write latency on Windows runners ("Flaky Windows tests" in
`todo.md`) means every wait in a new test is on an event or a file, with a
deadline of 20 s or more, never a fixed sleep.

---

- **R1 — the wake queue, its history item and the wake admission**
  (§3.1, §3.2). The kernel side only. Nothing queues events yet but
  test support.
  - Files:
    - kernel: `model/wake.rs` (new: the types and `wake_events_record`),
      `model/conversation_item.rs` (the variant and its lowering),
      `model/mod.rs`, `lib.rs`;
    - `service/state/record.rs` (`SessionRuntime.wake_events`), the
      persisted runtime (`conversation/mod.rs`), `service/persistence.rs`,
      `recovery/helpers.rs`;
    - `service/wake/{mod,admission}.rs` (new),
      `service/mod.rs` (`TurnAdmission::Wake`),
      `turn/state.rs` (`admission_items` extracted), `turn/loop_driver.rs`
      (`consume_pending_wake_events`), `turn/terminal.rs`
      (`WakeEventsPending`), `loop_control.rs`
      (`AgentFollowUpReason::BackgroundEvents`);
    - observers: `contracts/src/observer/mod.rs`
      (`WakeEventQueued`; `BackgroundWorkChanged` comes in R2 with its
      consumer);
    - every site of c930bcc's list (§3.2): history store (`store/mod.rs`,
      `store/memory.rs`), `permission.rs`, `compression/adapter.rs`,
      `sampling/composition.rs`, `validation/{persistence,tool_pairs}.rs`,
      `agent/src/service/export.rs`, `cli/src/sessions.rs`,
      `tui/src/agent_bridge/replay.rs` and `chatwidget/replay.rs` (the cell);
    - `test_support`: `enqueue_wake_event` reachable from the agent's tests.
  - Tests:
    - kernel unit (`model/wake/tests.rs`): the rendering of each kind, the
      bounds and cuts, keyed replacement, tolerant decoding (unknown kind,
      bad field: that event dropped, the session kept);
    - `crates/kernel/tests/loop_contract/wake.rs` (new):
      - an event queued while a turn runs is drained at the next step, as
        one item, with a follow-up;
      - one queued after the last step makes `finish_completed` take
        another step;
      - an admission drains the queue before the user message;
      - `submit_wake_turn_with_sink` admits a turn whose opener is the
        item, returns `None` on an empty queue or a running turn, and is
        refused with the policy off;
      - with the policy off, steps drain nothing and only a sub-agent's
        final status is kept;
      - `take_wake_events` by filter;
    - `persistence_contract`: the queue survives a reopen; an undecodable
      event is dropped, the session recovers;
    - agent: `tests/golden.rs` adds `wake_events_request.json` (new),
      queued through test support;
    - export, `cli sessions show`, the reviewer and memory read the item;
      TUI: the replayed cell's snapshot.
  - Module size: `stage_turn_admission` is split by its openers;
    `conversation_item.rs` (545) takes the variant and its arms only, with
    the text in `model/wake.rs`.
  - Goldens: one new (`wake_events_request.json`); none changed.
  - Docs: `internals` (a new "Wake events" section, `#wake-events`: the
    queue, keys, bounds, delivery, the admission); `embedding#events-and-history`
    (the new history item); `json-events` (`history_committed`'s item kind).
  - Acceptance: a queued event reaches the model exactly once, either at
    the running turn's next step or as the opener of a wake turn, and
    survives a restart until then.
  - Hanafubuki: a new `ConversationItem` variant to render (or skip).

- **R2 — the main agent suspends and wakes on its sub-agents** ([1]; D4–D8,
  D16, D17; §3.3, §3.4).
  - Scope:
    - the option and the policy;
    - the dispatcher and the pause;
    - `SubagentService` queues final statuses and `wait_agent` takes them
      (`unreported` removed);
    - `send_message` `parent` (depth-1 children → root);
    - the guidance and tool descriptions;
    - the host sink with `Woken`, and `submit_wake_turn_with_sink` on the
      facade;
    - Goal's pass, using `background_work` (`service/wake/work.rs` (new),
      counting agents only until R5, §3.6) and `BackgroundWorkChanged`;
    - the `SubagentStart`/`SubagentStop` diagnostic text;
    - in the terminal interface, `Woken` for the shown session submits the
      wake turn, and the waiting state's line;
    - `exec` waits for the tree (agents only until R5) and submits wake
      turns.
  - Files: `crates/config` (the setting, its spec and default), the driver
    (policy at activation, `TurnWake` extension, `run_wake_turn`),
    `agent/src/wake/{mod,dispatcher,pause}.rs` (new), the
    `agent-host-services` row's registration, `service/background.rs`
    (new: `submit_wake_turn_with_sink`), `service/mod.rs` and
    `standalone/mod.rs` (the sink), `contracts/src/continuation/mod.rs`
    (`background`), `plugins/goal/mod.rs`,
    `plugins/subagents/{service,tools,prompt,mod}.rs`,
    `hooks/src/event.rs:78`, `cli/src/exec/mod.rs`,
    `tui/src/agent_bridge/background.rs` (new),
    `status_indicator_widget.rs`.
  - Tests (`plugins/subagents/tests/wake.rs` (new), fake server):
    - the root spawns two agents and ends its turn; each agent's end wakes
      it once (two events arriving together make one turn); the request
      carries the item with their answers;
    - an agent finishing during the root's running turn reaches its next
      step; during `wait_agent`, the wait returns it and no step repeats
      it;
    - a child's `send_message` to `parent` wakes the idle root and reaches
      the running one at its next step; the 17th is refused;
    - the person's interrupt pauses; the next `User` turn delivers and
      clears it; 101 woken turns pause;
    - the option off: no wake, the old bullet, `wait_agent` returns the
      statuses, and they survive a restart;
    - Goal active: the turn ends while an agent works and the woken turn
      continues the goal;
    - a woken turn runs no `UserPromptSubmit` hook, and does run Stop
      hooks;
    - `exec` (`crates/cli` tests): the root yields, the agent ends, the
      woken turn answers, and `exec` exits 0 with that answer;
    - TUI: `Woken` for the shown session starts a turn whose first cell is
      the item; for another session it does not; the waiting line's
      snapshot.
  - Goldens: the 12 with `## Sub-agents` (bullet, paragraph) and the 14
    with `spawn_agent`'s spec (`spawn_agent`, `send_message`, `wait_agent`
    descriptions), regenerated on purpose.
  - Module size: `service.rs` gains about 30 lines net (`unreported` and
    `Notify` go); `agent/src/service/mod.rs`'s `construct` (188) gains one
    call into `wake::install`.
  - Docs: `configuration` and `config-reference` (General:
    `wakeOnBackgroundTasks`; `cargo xtask check-config-reference`); `tools#subagents`
    (no "wait before answering"; `parent`; the wake); `sessions#subagents`;
    `embedding` (the sink, `submit_wake_turn_with_sink`, `Woken`);
    `architecture` (a "Wake" paragraph: queue, dispatcher, host);
    `internals#wake-events` (the dispatcher, pause, bound);
    `exec` (waiting for sub-agents); `terminal-interface#subagents` (the
    wake and the waiting line); `plugin-rules` (the `subagents` row; the
    continuation context's `background`); `configuration#hooks` (the
    corrected reason).
  - Acceptance: with the defaults, a root that delegates and ends its turn
    holds no turn, no task and no thread, and resumes by itself with its
    agents' results; with the option off, the model sees today's tools
    and guidance.
  - Hanafubuki: `AgentServiceInputs.background_event_sink`; `Woken` to
    submit or ignore; the new setting in its settings page (from the
    catalog); goldens.

- **R3 — the background feed for hosts and the terminal interface**
  (D14, §3.7). The depth is still 1.
  - Scope:
    - `background_tasks(root)` and `Changed`;
    - `Child` through a forwarding sink on child turns;
    - the terminal interface's 400 ms watch replaced by the feed
      (approvals and statuses);
    - `exec` answers a child's approvals and questions by `--approve`.
  - Files: `service/background.rs`, `plugins/subagents/service.rs`
    (the forwarding sink), `session_driver/mod.rs` (`run_turn` takes a
    sink), `tui/src/agent_bridge/{background,subagents,start,mod}.rs`
    (the `Op` session start moved out of `mod.rs` first), `cli/src/exec/`.
  - Tests:
    - a child's approval reaches the host as `Child` with its root, and is
      answered on the child;
    - `Changed` on a child's start and end and on a queued event;
    - `background_tasks` lists children and terminals of the root;
    - TUI `app/tests/subagents.rs` passes unchanged over the feed (the
      approval labelled `Subagent NAME:`, closed when answered elsewhere
      or the turn ends), with no timer;
    - `exec --approve deny` denies a child's approval and prints it.
  - Goldens: none.
  - Docs: `embedding#child-sessions` and `#feature-calls`
    (`background_tasks`; "no parent-side event is added" no longer
    holds); `terminal-interface#subagents` ("reads them about every 0.4
    seconds" goes); `exec#approvals`; `internals#tui-details`.
  - Acceptance: no polling of sub-agents remains in the terminal
    interface; `exec` never leaves a child's approval unanswered.
  - Hanafubuki: `background_tasks`, `Child` events (it can drop its own
    polling of `subagents` if it has any).

- **R4 — two levels of sub-agents, eight at once** ([2]; D8–D10, §3.5).
  - Scope:
    - depth from lineage and the refusals;
    - live agents across the tree, default 8;
    - the held completion and `settle`;
    - `Waiting` and `Closed` states (`Closed` set in R6);
    - depth-1 guidance;
    - `send_message` `parent` from depth 2;
    - the dispatcher wakes sub-agents through `SubagentService::resume`.
  - Files: `plugins/subagents/service/{mod,runs,statuses}.rs` (split),
    `tree.rs` (new), `options.rs` (default 8; summary "How many sub-agents
    work at once under one conversation, theirs included"), `prompt.rs`,
    `tools.rs`, `mod.rs` (`AgentSubagentStatus`, `AgentSubagentState`),
    `wake/dispatcher.rs`, `tui/src/commands/subagents/`.
  - Tests (`plugins/subagents/tests/nesting.rs` (new)):
    - a depth-1 agent spawns, a depth-2 one is refused with the text;
      side and review refused;
    - eight live agents across two levels, the ninth refused; a suspended
      depth-1 agent counts; its woken turn is not refused at the cap;
    - a depth-1 agent that yields while its child works is not reported;
      when its child ends it is woken, answers, and only then is the root
      woken with its answer;
    - a depth-2 child's failure wakes its parent, not the root;
    - `list_agents` at depth 1 lists only its own children;
    - the limit's default and summary in the settings catalog test.
  - Goldens: the 12 with the guidance and the 14 with `spawn_agent`
    (depth sentences), regenerated on purpose.
  - Docs: `tools#subagents` (limits, depth), `sessions#subagents`,
    `roles#subagents` if it states depth, `config-reference#plugin-options`
    (`maximumRunningAgents`: default 8, what it counts),
    `embedding#feature-calls` (`AgentSubagentStatus` fields and states),
    `terminal-interface#subagents` (states), `plugin-rules` (`subagents`
    row).
  - Acceptance: Main → Sub-agent → Sub-agent works; a third level is
    refused; at most 8 work at once; each parent wakes once per child's
    finished work.
  - Hanafubuki: `AgentSubagentState::{Waiting, Closed}`,
    `AgentSubagentStatus.{parent_session_id, depth}`; goldens.

- **R4b — the command timeout becomes a setting** ([3]; D12, §3.13).
  The user's answer to Q4. Nothing in it reads or queues a wake event.
  - Scope:
    - `pluginOptions.execution.commandTimeoutMinutes` (1–1440, default
      10) in the `execution` row's descriptor, so the settings catalog,
      `config get`/`config set`, `/plugins`' options form and a host's
      settings page have it;
    - the `execution` row derives from its options and takes a change in
      place, without a restart;
    - every `exec_command` execution, a `!command`'s included, runs with
      the value at its call (`ExecCommandRequest.timeout_ms`), in its plan
      and its approval's canonical bytes.
  - Files (at `ed2828f`): `crates/execution/src/model.rs`
    (`ExecCommandRequest.timeout_ms`), `crates/execution/src/shell.rs`
    (`prepare_exec_command` takes it), `crates/execution/src/resolver.rs`
    and `crates/execution/src/command.rs` (`DEFAULT_COMMAND_TIMEOUT_MS`
    moves to the argv harness, its only user);
    `crates/plugin-execution/Cargo.toml` (`serde`),
    `crates/plugin-execution/src/plugin.rs` (`PluginDef::typed`,
    `on_update`), `crates/plugin-execution/src/options.rs` (new:
    `ExecutionOptions` and the exported bounds and default),
    `crates/plugin-execution/src/tool.rs` (`ExecServices` holds the
    value; `parse_call` passes it), `crates/plugin-execution/src/arguments.rs`
    (`parse` puts it in the request it builds),
    `crates/plugin-execution/src/lib.rs`;
    `crates/agent/src/host/descriptors.rs` (`execution_option_schema()`,
    the row's `.with_options`), `crates/agent/src/host/manifest.rs`
    (`from_settings`). Every test that builds an `ExecCommandRequest` or
    calls `parse` sets the field: `crates/execution/tests/contracts/`
    (`main.rs`, `output.rs`, `windows_posix.rs`, `windows_powershell.rs`),
    `crates/execution/tests/windows_sandbox.rs`,
    `crates/plugin-execution/src/monitor/tests.rs`,
    `crates/plugin-execution/src/arguments/tests.rs` and
    `crates/agent/src/tests/sessions/mod.rs`.
  - Tests:
    - execution contract (`crates/execution/tests/contracts/unified.rs`):
      the plan and the action carry the request's `timeout_ms`; two
      requests differing only in it have different action digests; a
      request with a timeout of 1 s (the request is in milliseconds; only
      the option counts minutes) ends a sleeping command `TimedOut`, POSIX
      and PowerShell;
    - plugin (`crates/plugin-execution`): a call's request takes the
      plugin's value in milliseconds; an update changes the next call's
      and leaves the plugin running, and a command already started keeps
      its timeout; a `!command` (`ToolCallOrigin::User`) takes it too;
    - agent: the settings catalog's `pluginOptions.execution` section with
      the key, its label, default 10 and bounds 1..1440
      (`crates/agent/src/tests/settings_catalog.rs`); a write of 0 or 1441
      refused; a file's 2000 clamped to 1440 with a warning; a write
      reaches the running plugin with no restart (`/plugins`' status).
    No test waits on the option: its minimum is 1 minute.
  - Goldens: none. No tool description or fragment states the limit until
    R5.
  - Module size: nothing near a limit (the largest file it edits,
    `crates/execution/src/model.rs`, has 365 physical lines).
  - Docs (both languages): `config-reference#plugin-options` (the row,
    written by hand: `cargo xtask check-config-reference` reads only the
    specs under `crates/config/src`, so it does not cover plugin options;
    and "A change the Agent writes applies at once" gains `execution`,
    which takes a new timeout in place for the commands started after
    it); `tools#commands` ("Time limits": every process is stopped after
    the command timeout, 10 minutes unless set); `terminal-interface#composer`
    (a `!command` runs at most the command timeout or the tool timeout,
    the shorter); `plugin-rules#host-configuration-and-status` ("The
    options of `subagents` are the first" gains `execution`) and
    `#the-plugins-today` (the `execution` row's option).
  - Acceptance: a person sets the command timeout in the settings center,
    `config set` or `config.toml`, and every command started afterwards
    runs up to it, while the running ones keep theirs and nothing
    restarts; with the default, nothing the model or the person sees
    changes.
  - Hanafubuki: the `pluginOptions.execution` section reaches its
    settings page from the catalog; `ExecCommandRequest.timeout_ms` if it
    builds the request itself.

- **R5 — terminals wake their owner** ([3]; D11, D12, §3.8).
  - Scope:
    - kernel production and withdrawal of terminal events;
    - execution tracking and withdrawal after a call reports the end;
    - the execution plugin's `background_terminals` fragment, which
      states R4b's current `commandTimeoutMinutes`;
    - `!command` excluded;
    - `exec` also waits for terminals.
  - Files: `kernel/src/service/progress.rs` (`stage_execution_progress`),
    `service/wake/terminal.rs` (new: the check and the event),
    `service/state/record.rs` (runtime execution tracking),
    `plugin-execution/src/{plugin,prompt}.rs` (`prompt.rs` new; it reads
    the value `ExecServices` holds), `cli/src/exec/mod.rs`.
  - Tests (`crates/agent/src/tests/executions.rs` and
    `tests/background_terminals.rs` (new); POSIX and PowerShell):
    - a command still running at yield, the turn ends, its exit wakes the
      session with its exit code and last output;
    - a command that ends within its call queues nothing;
    - a `write_stdin` poll that sees the end withdraws the event, so no
      woken turn and no duplicate at the next step;
    - a timeout queues `timed out` with how long the command ran. The
      option's minimum is 1 minute, so this test does not run a real
      timeout: a kernel contract test reports a terminal `TimedOut`
      `ExecutionStatus` through a tool's progress sink, the boundary where
      §3.8 produces the event. The executor's own timeout is R4b's
      contract test, with a request timeout in milliseconds;
    - a `!command`'s end queues nothing;
    - the policy off queues nothing;
    - the fragment shown with `exec_command` offered and the policy on,
      and absent otherwise; it states the configured minutes, and a
      change of the setting at the next step;
    - `exec` waits for a background terminal and exits after the woken
      turn.
  - Goldens: the 14 offering `exec_command` (the fragment), regenerated on
    purpose.
  - Module size: nothing in `activity.rs`; `progress.rs` (446) calls into
    `wake/terminal.rs`.
  - Docs: `tools#commands` (background terminals wake their session);
    `internals#execution-progress` (the wake and the withdrawal) and
    `#wake-events`; `exec` (waits for terminals, at most the command
    timeout); `plugin-rules` (the `execution` row's fragment);
    `config-reference#plugin-options` (`commandTimeoutMinutes` also
    bounds how long a terminal holds its owner and `exec`).
  - Acceptance: a model that starts a long command and ends its turn is
    resumed when it ends, and never told twice.
  - Hanafubuki: goldens; nothing else.

- **R6 — closing a sub-agent or a terminal** ([4]; D13, §3.9).
  - Scope:
    - `close_subagent` (cascade) and `close_background_terminal`;
    - `/stop` as the person's close of all;
    - the internal stop path split out of `quiesce_background_terminals`;
    - the command center's child "stop" calls `close_subagent`.
  - Files: `plugins/subagents/{tree,service/runs}.rs`,
    `agent/src/service/{background,background_terminals,features}.rs`,
    `tui/src/agents_center/` (its stop action), `commands/terminals.rs`
    (the confirmation's subtitle).
  - Tests:
    - closing a depth-1 agent with a working child stops both and their
      terminals, and wakes the root once with "closed by the user";
      the closed agent takes a follow-up task;
    - closing a terminal of a suspended root wakes it with "stopped by the
      user";
    - a terminal that exited just before the close reports the close, not
      the exit;
    - `/stop` with three terminals makes one woken turn with three events;
    - archive, delete and side close queue nothing and wake nothing.
  - Goldens: none.
  - Docs: `embedding#feature-calls` (`close_subagent`,
    `close_background_terminal`, `stop_background_terminals_confirmed`'s
    new effect); `terminal-interface` (`/stop`, the command center's
    stop); `sessions#subagents`.
  - Acceptance: every person's close reaches the owner and resumes it; the
    Agent's own teardown never does.
  - Hanafubuki: the two calls; `stop_background_terminals_confirmed` now
    wakes.

- **R7 — restart** (D15, §3.10).
  - Scope: `report_lost_background_work`, run at a clean shutdown before
    turns are cancelled and at start after recovery; no wake while
    starting.
  - Files: `agent/src/service/{mod,background}.rs` (the shutdown stage and
    the start), `wake/dispatcher.rs`.
  - Tests (`tests/restart.rs` beside today's, and `tests/shutdown.rs`):
    - a root suspended on a running child, shut down cleanly and started
      again: the root's queue holds `Lost` for the child, nothing runs,
      and the next message delivers it;
    - the same after a simulated crash (the turn left running, settled
      `ApplicationRestarted` by recovery);
    - a depth-1 agent suspended on a lost grandchild, restarted: `Lost` for
      the depth-1 agent on the root (not for the grandchild);
    - a background terminal of a root lost at restart: `Lost` on the root;
    - pending events from before the restart are delivered once, and a
      shutdown followed by a start reports each lost item once.
  - Goldens: none.
  - Docs: `internals#wake-events` (restart), `sessions#subagents`,
    `recovery` if it lists what a start repairs.
  - Acceptance: after any restart, every owner that waited on lost work is
    told so with its next turn, and nothing starts by itself.
  - Hanafubuki: none beyond R1's item.

- **R8 — the background task manager and the status indicator** ([4][5];
  D20, §3.12).
  - Scope:
    - `/tasks` and its view (tree, terminals, details, close with
      confirmation, resume a paused wake);
    - the `background-tasks` item in the default status line;
    - the waiting and paused lines;
    - `open_tasks` on `leader t`, and its `?` row;
    - the `/ps` and `/subagents` hints;
    - the command center's `N updates`.
  - Files: `tui/src/tasks/` (new), `commands/{tasks,terminals,mod}.rs`,
    `commands/subagents/mod.rs`, `appearance/{items,mod}.rs`,
    `agent_bridge/status.rs`, `crates/config/src/tui.rs`
    (`DEFAULT_STATUS_LINE`), `keymap/catalogue.rs`, `shortcut_help.rs`,
    `status_indicator_widget.rs`, `agents_center/` (the updates count).
  - Tests: snapshots of the manager (a tree of two levels, terminals, the
    detail pane, the confirmations, paused); `x` closes and the session
    is woken (`app/tests/tasks.rs` (new)); the status item's values and
    its absence at zero (`appearance/tests.rs`); the default status line
    (`config/src/tui/tests.rs`); the keymap resolution and the overlay
    snapshot; `/ps`'s cell.
  - Goldens: none.
  - Docs: `terminal-interface` (a "Background tasks" section `#tasks`;
    `#status-line`'s item; `#keymap`'s action; `#shortcuts`' row;
    `#subagents` and the `/ps`, `/stop` rows); `config-reference#tui`
    (`tui.statusLine`'s default); `tui.keymap`'s list where it names
    actions.
  - Acceptance: the person sees how many agents and terminals work, lists
    them in one view, inspects and closes any, and sees that a suspended
    session resumes by itself.
  - Hanafubuki: none (the terminal interface is not its).

## 7. Risks

- **Model behaviour.** Some models keep calling `wait_agent` or polling
  anyway, or end their turn while expecting to be woken with the option
  off. The guidance differs by policy, and the goldens pin it. Tests
  cannot measure how a real model follows it; R2's notes record what was
  tried by hand.
- **Unprompted turns cost money.** Every wake is a model call nobody typed.
  The pause, the runaway bound and the held child completion (D8) keep
  the count to "one per unit of finished work", but a tree of 8 agents
  each with terminals can still wake often. The bound is 100 woken turns
  per tree, a constant (Q5).
- **Terminal end detection** relies on the "call in flight" test and on the
  monitor ordering a status before the disposal (§3.8). A third ordering
  this plan missed would show up as a duplicate update or a missed wake.
  R5's tests cover the three known orders; a missed wake would leave a
  session waiting until its next turn, never forever silently, since the
  status item shows the terminal ended.
- **Two signals for the dispatcher** (`WakeEventQueued`, `TurnFinished`)
  and the host's own submit race by design. Every path ends in "drain if
  not empty and idle", which is idempotent. A test hammers it with
  concurrent events.
- **The command timeout** (10 minutes by default) makes a development
  server a guaranteed wake, and holds `exec` and a sub-agent's completion
  until it. A person who raises `commandTimeoutMinutes` lengthens both,
  up to 24 hours (§1.1, D12).
- **The forwarding sink** puts child events on the host's thread. It
  forwards a handful of kinds only, and `emit` must not block (as every
  sink).
- **Windows runners' slow history writes**: every event is a durable
  delta. A burst of events (`/stop` with many terminals) is many small
  appends. The keyed replacement bounds the count; tests wait on events,
  not time.
- **Size**: `agent_bridge/mod.rs` (596) and `stage_turn_admission` (196)
  need their splits first (R3, R1). If a split does not land cleanly, the
  milestone reports it rather than raising a ceiling.

## 8. Not in this plan

- Supporting `SubagentStart`/`SubagentStop` user hooks (Q10).
- A model tool to close a sub-agent or to see another agent's terminals.
- Waking side conversations or reviews (D19).
- Background wake for MCP tasks or anything other than sub-agents and
  terminals.
- Raising the 30-minute turn deadline or the per-turn budget.
- A woken turn started by the Agent on a host's behalf (D4).
- Changing `exec_command`'s arguments (D11).
