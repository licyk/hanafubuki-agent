// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `/resume TEXT`: the session TEXT names (Codex's
//! `named_session_lookup.rs`), by its id, else by its title when exactly
//! one session has it. The sessions are those the picker lists: the top
//! level's, not archived, with a turn.

use hanafubuki_agent::{AgentSessionId, AgentSessionListRequest, AgentSessionPage};

/// The history store's largest page.
const PAGE_SIZE: usize = 100;

/// The session `text` names; `None` when none does or several share the
/// title, so the picker searches for it instead.
pub(crate) fn find_session(
    text: &str,
    mut list: impl FnMut(AgentSessionListRequest) -> Result<AgentSessionPage, String>,
) -> Result<Option<AgentSessionId>, String> {
    let text = text.trim();
    let id = text.parse::<u64>().ok().map(AgentSessionId::new);
    let mut titled = Vec::new();
    let mut cursor = None;
    loop {
        let page = list(AgentSessionListRequest {
            page_size: PAGE_SIZE,
            cursor,
            ..AgentSessionListRequest::default()
        })?;
        for session in page
            .sessions
            .into_iter()
            .filter(|session| session.latest_turn_phase.is_some())
        {
            if Some(session.session_id) == id {
                return Ok(id);
            }
            let title = session.title.as_ref().map(|title| title.value.trim());
            if title == Some(text) {
                titled.push(session.session_id);
            }
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    Ok(match titled.as_slice() {
        [only] => Some(*only),
        _ => None,
    })
}
