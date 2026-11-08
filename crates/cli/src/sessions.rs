//! `sessions list|show|archive|delete`, through the running Agent's history.

use hanafubuki_agent::{
    AgentSessionId, AgentSessionLifecycle, AgentSessionListRequest, AgentSessionSummary,
    ConversationItem,
};

use crate::agent::with_agent;
use crate::cli::{GlobalArgs, SessionsCommand};
use crate::exit::Failure;

/// How many items of a transcript `show` prints, the latest.
const SHOWN_ITEMS: usize = 200;

pub(crate) fn run(global: &GlobalArgs, command: SessionsCommand) -> Result<(), Failure> {
    match command {
        SessionsCommand::List { archived, json } => with_agent(global, |_, agent| {
            let lifecycle = if archived {
                AgentSessionLifecycle::Archived
            } else {
                AgentSessionLifecycle::Available
            };
            let mut request = AgentSessionListRequest {
                lifecycle,
                ..AgentSessionListRequest::default()
            };
            loop {
                let page = agent.list_sessions_page(request.clone()).map_err(failure)?;
                for session in &page.sessions {
                    if json {
                        println!("{}", to_json(session)?);
                    } else {
                        println!("{}", describe(session));
                    }
                }
                match page.next_cursor {
                    Some(cursor) => request.cursor = Some(cursor),
                    None => return Ok(()),
                }
            }
        }),
        SessionsCommand::Show { id, json } => {
            let id = session_id(&id)?;
            with_agent(global, |_, agent| {
                let opened = agent.open_session(id, SHOWN_ITEMS).map_err(failure)?;
                if json {
                    println!("{}", to_json(&opened.transcript)?);
                    return Ok(());
                }
                let runtime = &opened.runtime;
                let title = runtime.title.as_ref().map(|title| title.value.as_str());
                println!(
                    "session {}  {}  {}",
                    runtime.id,
                    runtime.model,
                    title.unwrap_or("(untitled)")
                );
                if opened.transcript.older_cursor.is_some() {
                    println!("(older items are not shown)");
                }
                for item in &opened.transcript.items {
                    if let Some(line) = transcript_line(&item.content) {
                        println!("{line}");
                    }
                }
                Ok(())
            })
        }
        SessionsCommand::Archive { id } => {
            let id = session_id(&id)?;
            with_agent(global, |_, agent| {
                let session = agent.open_session(id, 1).map_err(failure)?.runtime;
                agent
                    .archive_session(id, session.generation, session.revision)
                    .map(|_| ())
                    .map_err(failure)
            })
        }
        SessionsCommand::Delete { id } => {
            let id = session_id(&id)?;
            with_agent(global, |_, agent| {
                let session = agent.open_session(id, 1).map_err(failure)?.runtime;
                agent
                    .delete_session_confirmed(id, session.generation, session.revision)
                    .map(|_| ())
                    .map_err(failure)
            })
        }
    }
}

fn session_id(id: &str) -> Result<AgentSessionId, Failure> {
    id.parse()
        .map(AgentSessionId::new)
        .map_err(|_| Failure::usage(format!("`{id}` is not a session id")))
}

fn describe(session: &AgentSessionSummary) -> String {
    let updated = i64::try_from(session.updated_at)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    let title = session.title.as_ref().map(|title| title.value.as_str());
    format!(
        "{}  {updated}  {}  {}",
        session.session_id,
        session.model,
        title.unwrap_or("(untitled)")
    )
}

/// One line of a transcript item a person reads; `None` for the items only
/// the model reads (context, world state).
fn transcript_line(item: &ConversationItem) -> Option<String> {
    match item {
        ConversationItem::UserMessage { text, .. } => Some(format!("user: {text}")),
        ConversationItem::SteeringMessage { text, .. } => Some(format!("user (steering): {text}")),
        ConversationItem::AssistantMessage { text, .. } => Some(format!("assistant: {text}")),
        ConversationItem::ToolCall { name, .. } => Some(format!("→ {name}")),
        ConversationItem::ToolOutput { output, .. } => Some(
            if output.is_error {
                "  failed"
            } else {
                "  done"
            }
            .to_string(),
        ),
        _ => None,
    }
}

fn to_json(value: &impl serde::Serialize) -> Result<String, Failure> {
    serde_json::to_string(value).map_err(failure)
}

fn failure(error: impl ToString) -> Failure {
    Failure::new(error.to_string())
}
