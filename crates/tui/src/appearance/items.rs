// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! The items of the status line and of the terminal title, by the ids
//! `tui.statusLine`, `tui.terminalTitle` and the commands' arguments name
//! them. Codex's account, rate-limit, cost and version items are left out;
//! the permission mode and the goal are added.

/// An item that can be shown: its id, and what it shows.
pub(crate) trait SurfaceItem: Copy + Eq + Sized + 'static {
    const ALL: &'static [Self];

    fn id(self) -> &'static str;

    fn description(self) -> &'static str;

    fn parse(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|item| item.id() == id)
    }
}

/// The ids in `ids` this interface knows, in order and once each; and the
/// ones it does not.
pub(crate) fn parse_items<T: SurfaceItem>(ids: &[String]) -> (Vec<T>, Vec<String>) {
    let mut items = Vec::new();
    let mut unknown = Vec::new();
    for id in ids {
        match T::parse(id) {
            Some(item) if !items.contains(&item) => items.push(item),
            Some(_) => {}
            None if !unknown.contains(id) => unknown.push(id.clone()),
            None => {}
        }
    }
    (items, unknown)
}

pub(crate) fn ids<T: SurfaceItem>(items: &[T]) -> Vec<String> {
    items.iter().map(|item| item.id().to_string()).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum StatusLineItem {
    Model,
    ModelWithReasoning,
    Reasoning,
    CurrentDir,
    GitBranch,
    ContextRemaining,
    ContextUsed,
    UsedTokens,
    TotalInputTokens,
    TotalOutputTokens,
    SessionId,
    SessionTitle,
    TaskProgress,
    PermissionMode,
    Goal,
    Compression,
}

impl SurfaceItem for StatusLineItem {
    const ALL: &'static [Self] = &[
        Self::Model,
        Self::ModelWithReasoning,
        Self::Reasoning,
        Self::CurrentDir,
        Self::GitBranch,
        Self::ContextRemaining,
        Self::ContextUsed,
        Self::UsedTokens,
        Self::TotalInputTokens,
        Self::TotalOutputTokens,
        Self::SessionId,
        Self::SessionTitle,
        Self::TaskProgress,
        Self::PermissionMode,
        Self::Goal,
        Self::Compression,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::ModelWithReasoning => "model-with-reasoning",
            Self::Reasoning => "reasoning",
            Self::CurrentDir => "current-dir",
            Self::GitBranch => "git-branch",
            Self::ContextRemaining => "context-remaining",
            Self::ContextUsed => "context-used",
            Self::UsedTokens => "used-tokens",
            Self::TotalInputTokens => "total-input-tokens",
            Self::TotalOutputTokens => "total-output-tokens",
            Self::SessionId => "session-id",
            Self::SessionTitle => "session-title",
            Self::TaskProgress => "task-progress",
            Self::PermissionMode => "permission-mode",
            Self::Goal => "goal",
            Self::Compression => "compression",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Model => "Current model name",
            Self::ModelWithReasoning => "Current model name with reasoning effort",
            Self::Reasoning => "Current reasoning effort",
            Self::CurrentDir => "Current working directory",
            Self::GitBranch => "Current Git branch (omitted when unavailable)",
            Self::ContextRemaining => "Context window remaining (omitted when unknown)",
            Self::ContextUsed => "Context window used (omitted when unknown)",
            Self::UsedTokens => "Tokens used in the session (omitted when zero)",
            Self::TotalInputTokens => "Input tokens used in the session",
            Self::TotalOutputTokens => "Output tokens used in the session",
            Self::SessionId => "Session id (omitted until the session starts)",
            Self::SessionTitle => "Session title (omitted until it has one)",
            Self::TaskProgress => "Latest task plan's progress (omitted until there is one)",
            Self::PermissionMode => "Permission mode of the session",
            Self::Goal => "Goal and its status (omitted when none is set)",
            Self::Compression => {
                "Hierarchical compression's active blocks and tokens saved (omitted without it)"
            }
        }
    }
}

impl StatusLineItem {
    /// The theme scopes whose colour the item takes, in order of
    /// preference, and the colour without one (Codex's accents).
    pub(crate) fn accent(self) -> (&'static [&'static str], ratatui::style::Color) {
        use ratatui::style::Color;
        match self {
            Self::Model | Self::ModelWithReasoning | Self::Reasoning => (
                &["entity.name.type", "support.type", "variable"],
                Color::Cyan,
            ),
            Self::CurrentDir => (&["string", "markup.underline.link"], Color::Green),
            Self::GitBranch => (&["entity.name.function", "entity.name.tag"], Color::Magenta),
            Self::ContextRemaining
            | Self::ContextUsed
            | Self::UsedTokens
            | Self::TotalInputTokens
            | Self::TotalOutputTokens
            | Self::Compression => (&["constant.numeric", "constant"], Color::Green),
            Self::SessionId => (&["comment", "constant.other"], Color::Cyan),
            Self::SessionTitle | Self::Goal => {
                (&["markup.heading", "entity.name.section"], Color::Magenta)
            }
            Self::TaskProgress => (&["markup.inserted", "constant.numeric"], Color::Green),
            Self::PermissionMode => (&["storage.modifier", "keyword.operator"], Color::Cyan),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TerminalTitleItem {
    AppName,
    ProjectName,
    CurrentDir,
    Activity,
    RunState,
    SessionTitle,
    Model,
    TaskProgress,
}

impl SurfaceItem for TerminalTitleItem {
    const ALL: &'static [Self] = &[
        Self::AppName,
        Self::ProjectName,
        Self::CurrentDir,
        Self::Activity,
        Self::RunState,
        Self::SessionTitle,
        Self::Model,
        Self::TaskProgress,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::AppName => "app-name",
            Self::ProjectName => "project-name",
            Self::CurrentDir => "current-dir",
            Self::Activity => "activity",
            Self::RunState => "run-state",
            Self::SessionTitle => "session-title",
            Self::Model => "model",
            Self::TaskProgress => "task-progress",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::AppName => "The application's name",
            Self::ProjectName => "Name of the current directory",
            Self::CurrentDir => "Current working directory",
            Self::Activity => "Spinner while a turn runs",
            Self::RunState => "Ready or Working",
            Self::SessionTitle => "Session title (omitted until it has one)",
            Self::Model => "Current model name",
            Self::TaskProgress => "Latest task plan's progress (omitted until there is one)",
        }
    }
}

impl TerminalTitleItem {
    /// What comes before the item: a space beside the spinner, ` | `
    /// between the others.
    pub(crate) fn separator_after(self, previous: Option<Self>) -> &'static str {
        match previous {
            None => "",
            Some(previous) if previous == Self::Activity || self == Self::Activity => " ",
            Some(_) => " | ",
        }
    }
}
