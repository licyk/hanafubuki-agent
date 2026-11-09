// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The actions the keymap binds, by context, with their default keys:
//! Codex's catalogue (`keymap.rs`'s built-in defaults, `leader_defaults.rs`)
//! cut down to what this interface does. A context is where its keys are
//! read: `global` anywhere, `chat` and `composer` while the composer has the
//! keys, `editor` in every text field, `transcript` in the Fullscreen
//! conversation beside the composer, `pager` in the transcript overlay and
//! the `/diff` pager.

/// Where an action's keys are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Context {
    Global,
    Chat,
    Composer,
    Editor,
    Transcript,
    Pager,
}

impl Context {
    pub(crate) const ALL: [Context; 6] = [
        Context::Global,
        Context::Chat,
        Context::Composer,
        Context::Editor,
        Context::Transcript,
        Context::Pager,
    ];

    /// The name `tui.keymap.<context>` uses.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Context::Global => "global",
            Context::Chat => "chat",
            Context::Composer => "composer",
            Context::Editor => "editor",
            Context::Transcript => "transcript",
            Context::Pager => "pager",
        }
    }

    pub(crate) fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|context| context.name() == name)
    }

    /// Whether its keys are read beside the composer, where a key that
    /// types text must stay text.
    pub(crate) fn beside_text(self) -> bool {
        self != Context::Pager
    }
}

/// An action the keymap binds; its [`ActionSpec`] is `ACTIONS[action as
/// usize]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Action {
    Leader,
    OpenTranscript,
    FindTranscript,
    FocusActivity,
    SelectTranscript,
    FocusPaneLeft,
    FocusPaneRight,
    ToggleSideConversation,
    MoveDividerLeft,
    MoveDividerRight,
    EqualizePaneWidths,
    TogglePaneZoom,
    HidePane,
    InterruptTurn,
    EditPrevious,
    ToggleThinking,
    OpenPresets,
    Submit,
    HistoryPrevious,
    HistoryNext,
    InsertNewline,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveWordLeft,
    MoveWordRight,
    MoveLineStart,
    MoveLineEnd,
    DeleteBackward,
    DeleteForward,
    DeleteBackwardWord,
    DeleteForwardWord,
    KillLineStart,
    KillLineEnd,
    Yank,
    TranscriptPageUp,
    TranscriptPageDown,
    TranscriptTop,
    TranscriptBottom,
    StartSelection,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    JumpTop,
    JumpBottom,
    Close,
    CloseTranscript,
}

pub(crate) struct ActionSpec {
    pub(crate) action: Action,
    pub(crate) context: Context,
    /// The name `tui.keymap.<context>.<name>` uses.
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    /// Its keys when the configuration names none.
    pub(crate) defaults: &'static [&'static str],
}

impl Action {
    pub(crate) fn spec(self) -> &'static ActionSpec {
        &ACTIONS[self as usize]
    }

    /// The action `<context>.<name>` names.
    pub(crate) fn named(context: Context, name: &str) -> Option<Self> {
        ACTIONS
            .iter()
            .find(|spec| spec.context == context && spec.name == name)
            .map(|spec| spec.action)
    }

    /// `context.name`, as warnings and `/keymap` name it.
    pub(crate) fn path(self) -> String {
        let spec = self.spec();
        format!("{}.{}", spec.context.name(), spec.name)
    }
}

/// Pairs of actions that may share a key: their handlers never both see
/// it (one runs in a state the other does not), or the first hands the
/// key on to the second when it does not apply (history, at the composer's
/// first or last line).
pub(crate) const SHARED_KEYS: [(Action, Action); 5] = [
    (Action::InterruptTurn, Action::EditPrevious),
    (Action::Submit, Action::InsertNewline),
    (Action::HistoryPrevious, Action::MoveUp),
    (Action::HistoryNext, Action::MoveDown),
    (Action::OpenTranscript, Action::CloseTranscript),
];

/// The contexts whose keys are read together, on one path of the input:
/// beside the composer, and in a pager.
pub(crate) const PATHS: [&[Context]; 2] = [
    &[
        Context::Global,
        Context::Chat,
        Context::Composer,
        Context::Editor,
        Context::Transcript,
    ],
    &[Context::Global, Context::Pager],
];

/// Keys the interface keeps for itself, with the contexts they shadow:
/// Ctrl-C (interrupt, clear, deny, quit; close a pager) and Ctrl-D (quit
/// from an empty composer, before the editor deletes with it).
pub(crate) const RESERVED: [(&str, &str, &[Context]); 2] = [
    ("ctrl-c", "interrupting or quitting", &Context::ALL),
    (
        "ctrl-d",
        "quitting",
        &[
            Context::Global,
            Context::Chat,
            Context::Composer,
            Context::Transcript,
        ],
    ),
];

macro_rules! action {
    ($action:ident, $context:ident, $name:literal, $description:literal, [$($key:literal),*]) => {
        ActionSpec {
            action: Action::$action,
            context: Context::$context,
            name: $name,
            description: $description,
            defaults: &[$($key),*],
        }
    };
}

/// Every action, in [`Action`]'s order.
pub(crate) static ACTIONS: [ActionSpec; 51] = [
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
        ToggleThinking,
        Chat,
        "toggle_thinking",
        "Show or hide the model's thinking",
        ["ctrl-o"]
    ),
    action!(
        OpenPresets,
        Chat,
        "open_presets",
        "Open the presets",
        ["leader p"]
    ),
    action!(Submit, Composer, "submit", "Send the message", ["enter"]),
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
        "Close the transcript",
        ["ctrl-t"]
    ),
];
