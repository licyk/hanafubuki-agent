//! A move or a delete (`move_path`, `delete_path`): `Moved a → b`,
//! `Deleted x`, with nothing more to show.

use hanafubuki_agent::AgentMutationKind;
use ratatui::style::Stylize;

use super::{ToolCall, View};

pub(super) fn view(call: &ToolCall, kind: AgentMutationKind, paths: &[String]) -> View {
    let verb = match kind {
        AgentMutationKind::Moved => "Moved",
        AgentMutationKind::Deleted => "Deleted",
    };
    View::title(
        vec![
            verb.bold(),
            " ".into(),
            call.subject_or(&paths.join(" → ")).into(),
        ],
        false,
    )
}
