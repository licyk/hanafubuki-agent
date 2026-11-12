//! The command center's reads, made off the interface's loop on a thread of
//! its own while the center is open: the pages of sessions, a session's
//! children and the start and end of a session's latest exchange, each a
//! [`CenterJob`] answered with a [`CenterRead`]. Between jobs the thread
//! watches the sessions the Agent holds ([`WATCH_INTERVAL`], four times a
//! second): their revision, an approval or a question each waits on and
//! what each is doing, sent as [`CenterRead::Live`] when any of it changed,
//! from which the center knows what to read again.
//!
//! The facade has no event stream for every session, so the watch reads the
//! Agent's sessions as they are, as the subagents' watch reads theirs; it
//! touches no store.

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use hanafubuki_agent::{
    AgentActivityPhase, AgentHandle, AgentHistorySortDirection, AgentRevision, AgentSessionId,
    AgentSessionListRequest, AgentSessionRuntimeSnapshot, AgentSessionSummary,
    AgentTranscriptPageRequest, ConversationItem,
};

use super::AgentBridge;
use crate::app::WorkspaceEvent;
use crate::app_event::{AppEvent, AppEventSender};

/// How often the watch reads the sessions the Agent holds.
const WATCH_INTERVAL: Duration = Duration::from_millis(250);
/// The items read back from a session's end for its latest exchange.
const EXCHANGE_ITEMS: usize = 64;
/// The characters a preview keeps.
const PREVIEW_CHARS: usize = 512;

/// A read the center asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CenterJob {
    /// The sessions from the first page on, until `wanted` are read.
    Roots {
        epoch: u64,
        request: AgentSessionListRequest,
        wanted: usize,
    },
    Children {
        epoch: u64,
        parent: AgentSessionId,
        request: AgentSessionListRequest,
    },
    /// The session's latest prompt and answer.
    Exchange(AgentSessionId),
}

/// What a read found, or what the watch saw.
#[derive(Debug)]
pub(crate) enum CenterRead {
    Roots {
        epoch: u64,
        read: Result<RootsRead, String>,
    },
    Children {
        epoch: u64,
        parent: AgentSessionId,
        read: Result<Vec<AgentSessionSummary>, String>,
    },
    Exchange {
        session_id: AgentSessionId,
        exchange: Exchange,
    },
    /// The sessions the Agent holds, by id.
    Live(Vec<LiveSession>),
}

#[derive(Debug)]
pub(crate) struct RootsRead {
    pub(crate) sessions: Vec<AgentSessionSummary>,
    /// More pages follow.
    pub(crate) more: bool,
}

/// The start and end of a session's latest exchange; none where the items
/// read hold none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Exchange {
    pub(crate) prompt: Option<String>,
    pub(crate) answer: Option<String>,
}

/// A session the Agent holds, as the watch sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LiveSession {
    pub(crate) session_id: AgentSessionId,
    pub(crate) parent: Option<AgentSessionId>,
    pub(crate) revision: AgentRevision,
    pub(crate) waiting: Option<Waiting>,
    /// What its turn does now: an activity's action and subject.
    pub(crate) activity: Option<String>,
}

/// What a session waits on you for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Waiting {
    /// An approval, with what it is for.
    Approval(String),
    /// The answer to a question.
    Question(String),
}

/// The reads' thread, which ends when this is dropped.
pub(crate) struct CenterReader {
    jobs: Sender<CenterJob>,
}

impl CenterReader {
    pub(crate) fn send(&self, job: CenterJob) {
        let _ = self.jobs.send(job);
    }
}

impl AgentBridge {
    /// Starts the command center's reads; `app_event_tx` takes the answers.
    pub(crate) fn center_reader(&self, app_event_tx: AppEventSender) -> CenterReader {
        let (jobs, receiver) = channel();
        let agent = self.agent.clone();
        std::thread::spawn(move || run(&agent, &receiver, &app_event_tx));
        CenterReader { jobs }
    }
}

fn run(agent: &AgentHandle, jobs: &Receiver<CenterJob>, app_event_tx: &AppEventSender) {
    let mut last = None;
    let mut next_watch = Instant::now();
    loop {
        let wait = next_watch.saturating_duration_since(Instant::now());
        match jobs.recv_timeout(wait) {
            Ok(job) => send(app_event_tx, read(agent, job)),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if Instant::now() < next_watch {
            continue;
        }
        next_watch = Instant::now() + WATCH_INTERVAL;
        let live = live_sessions(agent);
        if last.as_ref() != Some(&live) {
            send(app_event_tx, CenterRead::Live(live.clone()));
            last = Some(live);
        }
    }
}

fn send(app_event_tx: &AppEventSender, read: CenterRead) {
    let event = WorkspaceEvent::AgentsCenter(Box::new(read));
    app_event_tx.send(AppEvent::Workspace(event));
}

fn read(agent: &AgentHandle, job: CenterJob) -> CenterRead {
    match job {
        CenterJob::Roots {
            epoch,
            request,
            wanted,
        } => CenterRead::Roots {
            epoch,
            read: read_roots(agent, request, wanted),
        },
        CenterJob::Children {
            epoch,
            parent,
            request,
        } => CenterRead::Children {
            epoch,
            parent,
            read: agent
                .list_sessions_page(request)
                .map(|page| page.sessions)
                .map_err(|error| format!("the sessions were not listed: {error}")),
        },
        CenterJob::Exchange(session_id) => CenterRead::Exchange {
            session_id,
            exchange: read_exchange(agent, session_id),
        },
    }
}

fn read_roots(
    agent: &AgentHandle,
    mut request: AgentSessionListRequest,
    wanted: usize,
) -> Result<RootsRead, String> {
    let mut sessions: Vec<AgentSessionSummary> = Vec::new();
    loop {
        let page = agent
            .list_sessions_page(request.clone())
            .map_err(|error| format!("the sessions were not listed: {error}"))?;
        sessions.extend(page.sessions);
        let Some(cursor) = page.next_cursor else {
            return Ok(RootsRead {
                sessions,
                more: false,
            });
        };
        if sessions.len() >= wanted {
            return Ok(RootsRead {
                sessions,
                more: true,
            });
        }
        request.cursor = Some(cursor);
    }
}

/// The latest message you sent and the latest answer, among the session's
/// last items; a failed read reads as neither.
fn read_exchange(agent: &AgentHandle, session_id: AgentSessionId) -> Exchange {
    let page = agent.read_transcript_page(AgentTranscriptPageRequest {
        session_id,
        page_size: EXCHANGE_ITEMS,
        cursor: None,
        direction: AgentHistorySortDirection::Descending,
        through_revision: None,
    });
    let mut exchange = Exchange::default();
    for item in page.map(|page| page.items).unwrap_or_default() {
        match item.content {
            ConversationItem::UserMessage { text, .. } if exchange.prompt.is_none() => {
                exchange.prompt = preview(&text);
            }
            ConversationItem::AssistantMessage { text, .. } if exchange.answer.is_none() => {
                exchange.answer = preview(&text);
            }
            _ => {}
        }
    }
    exchange
}

fn live_sessions(agent: &AgentHandle) -> Vec<LiveSession> {
    let mut live: Vec<LiveSession> = agent
        .session_snapshots()
        .into_iter()
        .filter(|snapshot| !snapshot.ephemeral)
        .map(live_session)
        .collect();
    live.sort_by_key(|session| session.session_id);
    live
}

fn live_session(snapshot: AgentSessionRuntimeSnapshot) -> LiveSession {
    let running = snapshot
        .turn
        .as_ref()
        .is_some_and(|turn| !turn.phase.is_terminal());
    let mut activities = snapshot
        .active_activities
        .iter()
        .filter(|_| running)
        .filter(|activity| !activity.phase.is_terminal());
    let approval = activities
        .clone()
        .find(|activity| activity.phase == AgentActivityPhase::WaitingForApproval)
        .map(|activity| {
            let presentation = &activity.presentation;
            presentation
                .subject
                .clone()
                .unwrap_or_else(|| label(presentation.action.as_str()))
        });
    let question = snapshot
        .pending_user_input
        .as_ref()
        .and_then(|request| request.questions.first())
        .map(|question| question.question.clone());
    let waiting = approval
        .map(Waiting::Approval)
        .or(question.map(Waiting::Question));
    let activity = activities.next_back().map(|activity| {
        let presentation = &activity.presentation;
        let action = label(presentation.action.as_str());
        match &presentation.subject {
            Some(subject) => format!("{action}: {subject}"),
            None => action,
        }
    });
    LiveSession {
        session_id: snapshot.id,
        parent: snapshot.parent_session_id,
        revision: snapshot.revision,
        waiting,
        activity,
    }
}

/// An action's name as words: `run_command` is `Run command`.
fn label(action: &str) -> String {
    let words = action.replace('_', " ");
    let mut characters = words.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().chain(characters).collect())
        .unwrap_or_default()
}

/// `text` as a preview: its first characters, without control characters
/// but its line breaks; none when it is blank.
fn preview(text: &str) -> Option<String> {
    let text: String = text
        .chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .take(PREVIEW_CHARS)
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}
