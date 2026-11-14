// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Every action's spec, in [`Action`]'s order: its context, its name in
//! `tui.keymap`, what it does and its default keys (Codex's built-in
//! defaults, `keymap.rs` and `leader_defaults.rs`).

use super::actions::{Action, ActionSpec, Context};

macro_rules! action {
    ($action:ident, $context:ident, $name:literal, $description:literal, [$($key:literal),*]) => {
        action!($action, $context, $name, $description, [$($key),*], false)
    };
    ($action:ident, $context:ident, $name:literal, $description:literal, [$($key:literal),*], empty_composer_only) => {
        action!($action, $context, $name, $description, [$($key),*], true)
    };
    ($action:ident, $context:ident, $name:literal, $description:literal, [$($key:literal),*], $empty:literal) => {
        ActionSpec {
            action: Action::$action,
            context: Context::$context,
            name: $name,
            description: $description,
            defaults: &[$($key),*],
            empty_composer_only: $empty,
        }
    };
}

/// Every action, in [`Action`]'s order.
pub(crate) static ACTIONS: [ActionSpec; 76] = [
    action!(
        Leader,
        Global,
        "leader",
        "The key that starts a leader chord (`leader p`)",
        ["ctrl-x"]
    ),
    action!(
        OpenTranscript,
        Global,
        "open_transcript",
        "Show the conversation in detail (Fullscreen) or open the transcript (Scrollback)",
        ["ctrl-t"]
    ),
    action!(
        FindTranscript,
        Global,
        "find_transcript",
        "Find text in the conversation",
        ["f3"]
    ),
    action!(
        FocusActivity,
        Global,
        "focus_activity",
        "Focus the tool calls' groups to open or close them",
        ["f4"]
    ),
    action!(
        SelectTranscript,
        Global,
        "select_transcript",
        "Select in the conversation with the keys (Fullscreen)",
        ["leader ["]
    ),
    action!(
        FocusPaneLeft,
        Global,
        "focus_pane_left",
        "Return to the main conversation from a side conversation or a subagent",
        ["leader left"]
    ),
    action!(
        FocusPaneRight,
        Global,
        "focus_pane_right",
        "Focus the side conversation",
        ["leader right"]
    ),
    action!(
        ToggleSideConversation,
        Global,
        "toggle_side_conversation",
        "Switch between the main and the side conversation",
        ["leader tab"]
    ),
    action!(
        MoveDividerLeft,
        Global,
        "move_divider_left",
        "Move the panes' divider left: the side pane grows (Fullscreen)",
        ["leader <"]
    ),
    action!(
        MoveDividerRight,
        Global,
        "move_divider_right",
        "Move the panes' divider right: the side pane shrinks (Fullscreen)",
        ["leader >"]
    ),
    action!(
        EqualizePaneWidths,
        Global,
        "equalize_pane_widths",
        "Give the two panes equal widths (Fullscreen)",
        ["leader ="]
    ),
    action!(
        TogglePaneZoom,
        Global,
        "toggle_pane_zoom",
        "The focused pane alone, or both again (Fullscreen)",
        ["leader z"]
    ),
    action!(
        HidePane,
        Global,
        "hide_pane",
        "Close the side conversation, or leave the subagent shown",
        ["leader x"]
    ),
    action!(
        OpenAgents,
        Global,
        "open_agents",
        "Open the command center (Left on an empty composer opens it too)",
        []
    ),
    action!(
        OpenTasks,
        Global,
        "open_tasks",
        "Open background tasks",
        ["leader t"]
    ),
    action!(
        InterruptTurn,
        Chat,
        "interrupt_turn",
        "Interrupt the running turn",
        ["esc"]
    ),
    action!(
        EditPrevious,
        Chat,
        "edit_previous",
        "Twice on an empty composer: edit an earlier message",
        ["esc"]
    ),
    action!(
        EditQueuedMessage,
        Chat,
        "edit_queued_message",
        "Put the last queued message back in the composer",
        ["shift-left", "alt-up"]
    ),
    action!(
        ToggleThinking,
        Chat,
        "toggle_thinking",
        "Expand or collapse every thinking",
        ["ctrl-o"]
    ),
    action!(
        OpenPresets,
        Chat,
        "open_presets",
        "Open the presets",
        ["leader p"]
    ),
    action!(
        CycleMode,
        Chat,
        "cycle_mode",
        "Switch between the default and the plan mode",
        ["shift-tab"]
    ),
    action!(
        ReasoningDown,
        Chat,
        "reasoning_down",
        "Lower the reasoning effort by a level",
        ["alt-,"]
    ),
    action!(
        ReasoningUp,
        Chat,
        "reasoning_up",
        "Raise the reasoning effort by a level",
        ["alt-."]
    ),
    action!(Submit, Composer, "submit", "Send the message", ["enter"]),
    action!(
        Queue,
        Composer,
        "queue",
        "While a turn runs: queue the message for after it",
        ["tab"]
    ),
    action!(
        HistoryPrevious,
        Composer,
        "history_previous",
        "The previous message sent",
        ["up", "ctrl-p"]
    ),
    action!(
        HistoryNext,
        Composer,
        "history_next",
        "The next message sent",
        ["down", "ctrl-n"]
    ),
    action!(
        PasteImage,
        Composer,
        "paste_image",
        "Attach the clipboard's image to the message (Ctrl+Alt+V for WSL, whose terminal pastes text with Ctrl+V)",
        ["ctrl-v", "ctrl-alt-v"]
    ),
    action!(
        ExternalEditor,
        Composer,
        "external_editor",
        "Edit the draft in the external editor (VISUAL, else EDITOR)",
        ["ctrl-g"]
    ),
    action!(
        HistorySearch,
        Composer,
        "history_search",
        "Search the messages sent, this run's and earlier sessions'",
        ["ctrl-r"]
    ),
    action!(
        ShowShortcuts,
        Composer,
        "show_shortcuts",
        "On an empty composer: show or hide the shortcuts",
        ["?"],
        empty_composer_only
    ),
    action!(
        PreviousAgent,
        Composer,
        "previous_agent",
        "On an empty composer: show the previous of the session and its subagents",
        ["alt-left"],
        empty_composer_only
    ),
    action!(
        NextAgent,
        Composer,
        "next_agent",
        "On an empty composer: show the next of the session and its subagents",
        ["alt-right"],
        empty_composer_only
    ),
    action!(
        InsertNewline,
        Editor,
        "insert_newline",
        "A new line",
        [
            "ctrl-j",
            "ctrl-m",
            "enter",
            "shift-enter",
            "alt-enter",
            "ctrl-enter"
        ]
    ),
    action!(
        MoveLeft,
        Editor,
        "move_left",
        "Move left",
        ["left", "ctrl-b"]
    ),
    action!(
        MoveRight,
        Editor,
        "move_right",
        "Move right",
        ["right", "ctrl-f"]
    ),
    action!(MoveUp, Editor, "move_up", "Move up", ["up", "ctrl-p"]),
    action!(
        MoveDown,
        Editor,
        "move_down",
        "Move down",
        ["down", "ctrl-n"]
    ),
    action!(
        MoveWordLeft,
        Editor,
        "move_word_left",
        "Move to the previous word",
        ["alt-b", "alt-left", "ctrl-left"]
    ),
    action!(
        MoveWordRight,
        Editor,
        "move_word_right",
        "Move past the next word",
        ["alt-f", "alt-right", "ctrl-right"]
    ),
    action!(
        MoveLineStart,
        Editor,
        "move_line_start",
        "Move to the line's start",
        ["home", "ctrl-a"]
    ),
    action!(
        MoveLineEnd,
        Editor,
        "move_line_end",
        "Move to the line's end",
        ["end", "ctrl-e"]
    ),
    action!(
        DeleteBackward,
        Editor,
        "delete_backward",
        "Delete the previous character",
        ["backspace", "shift-backspace", "ctrl-h"]
    ),
    action!(
        DeleteForward,
        Editor,
        "delete_forward",
        "Delete the next character",
        ["delete", "shift-delete", "ctrl-d"]
    ),
    action!(
        DeleteBackwardWord,
        Editor,
        "delete_backward_word",
        "Delete the previous word",
        ["alt-backspace", "ctrl-backspace", "ctrl-w", "ctrl-alt-h"]
    ),
    action!(
        DeleteForwardWord,
        Editor,
        "delete_forward_word",
        "Delete the next word",
        ["alt-delete", "ctrl-delete", "alt-d"]
    ),
    action!(
        KillLineStart,
        Editor,
        "kill_line_start",
        "Delete to the line's start",
        ["ctrl-u"]
    ),
    action!(
        KillLineEnd,
        Editor,
        "kill_line_end",
        "Delete to the line's end",
        ["ctrl-k"]
    ),
    action!(
        Yank,
        Editor,
        "yank",
        "Put back what was deleted last",
        ["ctrl-y"]
    ),
    action!(
        TranscriptPageUp,
        Transcript,
        "page_up",
        "Scroll the conversation up a page",
        ["page-up"]
    ),
    action!(
        TranscriptPageDown,
        Transcript,
        "page_down",
        "Scroll the conversation down a page",
        ["page-down"]
    ),
    action!(
        TranscriptTop,
        Transcript,
        "jump_top",
        "Go to the conversation's start",
        ["ctrl-home", "alt-<", "alt-shift-,"]
    ),
    action!(
        TranscriptBottom,
        Transcript,
        "jump_bottom",
        "Go to the latest output",
        ["ctrl-end", "alt->", "alt-shift-."]
    ),
    action!(
        StartSelection,
        Transcript,
        "start_selection",
        "Start selecting at the view's first row",
        ["ctrl-space"]
    ),
    action!(
        ScrollUp,
        Pager,
        "scroll_up",
        "Scroll up a line",
        ["up", "k"]
    ),
    action!(
        ScrollDown,
        Pager,
        "scroll_down",
        "Scroll down a line",
        ["down", "j"]
    ),
    action!(
        PageUp,
        Pager,
        "page_up",
        "Scroll up a page",
        ["page-up", "shift-space", "ctrl-b"]
    ),
    action!(
        PageDown,
        Pager,
        "page_down",
        "Scroll down a page",
        ["page-down", "space", "ctrl-f"]
    ),
    action!(
        HalfPageUp,
        Pager,
        "half_page_up",
        "Scroll up half a page",
        ["ctrl-u"]
    ),
    action!(
        HalfPageDown,
        Pager,
        "half_page_down",
        "Scroll down half a page",
        ["ctrl-d"]
    ),
    action!(JumpTop, Pager, "jump_top", "Go to the start", ["home"]),
    action!(JumpBottom, Pager, "jump_bottom", "Go to the end", ["end"]),
    action!(Close, Pager, "close", "Close the pager", ["q", "esc"]),
    action!(
        CloseTranscript,
        Pager,
        "close_transcript",
        "Show the transcript in detail, then close it",
        ["ctrl-t"]
    ),
    action!(
        AgentsSearch,
        Agents,
        "search",
        "Search the sessions' titles",
        ["f3", "/"]
    ),
    action!(
        AgentsNewTask,
        Agents,
        "new_task",
        "Start a new session",
        ["n"]
    ),
    action!(
        AgentsNewWorktree,
        Agents,
        "new_worktree",
        "Start a new session in a new worktree",
        ["w"]
    ),
    action!(AgentsFork, Agents, "fork", "Fork the session", ["f"]),
    action!(AgentsRename, Agents, "rename", "Rename the session", ["r"]),
    action!(
        AgentsStop,
        Agents,
        "stop",
        "Interrupt the session's turn",
        ["x"]
    ),
    action!(
        AgentsPin,
        Agents,
        "toggle_pin",
        "Pin the session, or unpin it",
        ["p"]
    ),
    action!(AgentsHide, Agents, "hide", "Hide the session", ["h"]),
    action!(
        AgentsChildren,
        Agents,
        "children",
        "Select the session's subagents and reviews",
        ["c"]
    ),
    action!(
        AgentsArchive,
        Agents,
        "archive",
        "Archive the session, or restore it under Archived",
        ["a"]
    ),
    action!(
        AgentsDelete,
        Agents,
        "delete",
        "Delete the session",
        ["backspace"]
    ),
    action!(
        AgentsToggleGrouping,
        Agents,
        "toggle_grouping",
        "Group by project, directory, status or model",
        ["g"]
    ),
];
