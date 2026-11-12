// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A session's place in the command center (Codex's `AgentsOverviewGroup`
//! and `TASK_FILTERS`): Needs you, Working, Ready or Inactive, a root's
//! group the most urgent of its own and its children's, and the filters
//! over them, with Archived beside them.
//!
//! Codex's Ready is a thread loaded and idle, its Inactive one not loaded;
//! every session here can go on at once, so Ready is a session whose last
//! turn completed and Inactive one whose last turn was interrupted or
//! cancelled (or a child's failed: its failure asks nothing of you).

use hanafubuki_agent::{AgentSessionLifecycle, AgentSessionSummary, AgentTurnPhase};

/// How urgent a session is, the most urgent first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Group {
    /// Waits for an approval or an answer, or (a root) its turn failed.
    NeedsYou,
    Working,
    Ready,
    Inactive,
}

impl Group {
    /// The group of `summary` by itself; `approval` when its turn waits on
    /// an approval.
    pub(crate) fn of(summary: &AgentSessionSummary, approval: bool) -> Self {
        match summary.latest_turn_phase {
            Some(AgentTurnPhase::WaitingForUserInput) => Self::NeedsYou,
            Some(AgentTurnPhase::Sampling | AgentTurnPhase::RunningTools) if approval => {
                Self::NeedsYou
            }
            Some(AgentTurnPhase::Sampling | AgentTurnPhase::RunningTools) => Self::Working,
            Some(AgentTurnPhase::Failed) if summary.parent_session_id.is_none() => Self::NeedsYou,
            Some(AgentTurnPhase::Completed) => Self::Ready,
            _ => Self::Inactive,
        }
    }

    /// What a heading of the status grouping and a row's status say.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::NeedsYou => "Needs input",
            Self::Working => "Working",
            Self::Ready => "Ready",
            Self::Inactive => "Inactive",
        }
    }
}

/// A root's status as its row shows it: its group with its children's
/// rolled up, and whether its own turn failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Status {
    pub(crate) group: Group,
    pub(crate) error: bool,
}

impl Status {
    pub(crate) fn of<'a>(
        root: &AgentSessionSummary,
        children: impl IntoIterator<Item = &'a AgentSessionSummary>,
        approval: impl Fn(&AgentSessionSummary) -> bool,
    ) -> Self {
        let group = children
            .into_iter()
            .map(|child| Group::of(child, approval(child)))
            .fold(Group::of(root, approval(root)), Group::min);
        let error = root.latest_turn_phase == Some(AgentTurnPhase::Failed);
        Self {
            group,
            error: error && group == Group::NeedsYou,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        if self.error {
            "Error"
        } else {
            self.group.label()
        }
    }
}

/// Which sessions the center lists: all of them, one group's, or the
/// archived ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Filter {
    All,
    Group(Group),
    Archived,
}

impl Filter {
    pub(crate) const ALL: [Self; 6] = [
        Self::All,
        Self::Group(Group::NeedsYou),
        Self::Group(Group::Working),
        Self::Group(Group::Ready),
        Self::Group(Group::Inactive),
        Self::Archived,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Group(Group::NeedsYou) => "Needs you",
            Self::Group(group) => group.label(),
            Self::Archived => "Archived",
        }
    }

    pub(crate) fn lifecycle(self) -> AgentSessionLifecycle {
        match self {
            Self::Archived => AgentSessionLifecycle::Archived,
            _ => AgentSessionLifecycle::Available,
        }
    }

    pub(crate) fn admits(self, status: Status) -> bool {
        match self {
            Self::All | Self::Archived => true,
            Self::Group(group) => status.group == group,
        }
    }

    pub(crate) fn step(self, forward: bool) -> Self {
        let count = Self::ALL.len();
        let index = Self::ALL
            .iter()
            .position(|filter| *filter == self)
            .unwrap_or(0);
        let next = if forward {
            (index + 1) % count
        } else {
            (index + count - 1) % count
        };
        Self::ALL[next]
    }
}
