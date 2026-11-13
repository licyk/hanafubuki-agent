// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The actions the keymap binds, by context (their specs are the
//! [`catalogue`](super::catalogue)), and the rules on their keys: which
//! actions may share one, which contexts are read together, which keys the
//! interface keeps. Codex's catalogue (`keymap.rs`'s built-in defaults,
//! `leader_defaults.rs`) cut down to what this interface does, without its
//! voice key. A context is where its keys are
//! read: `global` anywhere, `chat` and `composer` while the composer has the
//! keys, `editor` in every text field, `transcript` in the Fullscreen
//! conversation beside the composer, `pager` in the transcript overlay and
//! the `/diff` pager, `agents` in the command center (Codex's
//! `KeymapContext::Agents`).

use super::catalogue::ACTIONS;

/// Where an action's keys are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Context {
    Global,
    Chat,
    Composer,
    Editor,
    Transcript,
    Pager,
    Agents,
}

impl Context {
    pub(crate) const ALL: [Context; 7] = [
        Context::Global,
        Context::Chat,
        Context::Composer,
        Context::Editor,
        Context::Transcript,
        Context::Pager,
        Context::Agents,
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
            Context::Agents => "agents",
        }
    }

    pub(crate) fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|context| context.name() == name)
    }

    /// Whether its keys are read beside the composer, where a key that
    /// types text must stay text.
    pub(crate) fn beside_text(self) -> bool {
        !matches!(self, Context::Pager | Context::Agents)
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
    OpenAgents,
    InterruptTurn,
    EditPrevious,
    EditQueuedMessage,
    ToggleThinking,
    OpenPresets,
    CycleMode,
    ReasoningDown,
    ReasoningUp,
    Submit,
    Queue,
    HistoryPrevious,
    HistoryNext,
    HistorySearch,
    ShowShortcuts,
    PreviousAgent,
    NextAgent,
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
    AgentsSearch,
    AgentsNewTask,
    AgentsNewWorktree,
    AgentsFork,
    AgentsRename,
    AgentsStop,
    AgentsPin,
    AgentsHide,
    AgentsChildren,
    AgentsArchive,
    AgentsDelete,
    AgentsToggleGrouping,
}

pub(crate) struct ActionSpec {
    pub(crate) action: Action,
    pub(crate) context: Context,
    /// The name `tui.keymap.<context>.<name>` uses.
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    /// Its keys when the configuration names none.
    pub(crate) defaults: &'static [&'static str],
    /// Read only on an empty composer, so it may take a key that types text
    /// (`?`): there is no text to type it into then.
    pub(crate) empty_composer_only: bool,
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
/// first or last line, word motion on an empty composer).
pub(crate) const SHARED_KEYS: [(Action, Action); 7] = [
    (Action::InterruptTurn, Action::EditPrevious),
    (Action::Submit, Action::InsertNewline),
    (Action::HistoryPrevious, Action::MoveUp),
    (Action::HistoryNext, Action::MoveDown),
    (Action::OpenTranscript, Action::CloseTranscript),
    (Action::PreviousAgent, Action::MoveWordLeft),
    (Action::NextAgent, Action::MoveWordRight),
];

/// The contexts whose keys are read together, on one path of the input:
/// beside the composer, in a pager, and in the command center, which takes
/// every key while it is open.
pub(crate) const PATHS: [&[Context]; 3] = [
    &[
        Context::Global,
        Context::Chat,
        Context::Composer,
        Context::Editor,
        Context::Transcript,
    ],
    &[Context::Global, Context::Pager],
    &[Context::Agents],
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
