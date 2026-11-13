use std::collections::HashSet;
use std::sync::Arc;

use hanafubuki_agent_contracts::BlockingService;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use tokio::sync::mpsc;

use super::generation::{generate_title, TITLE_INSTRUCTIONS};
use super::TitleWork;
use crate::plugins::structured_request::wait_for_cancellation;
use crate::titles::AgentTitleApplication;

/// Generates the queued titles one after another until the plugin stops.
/// A session settled in this run is not looked at again: the model gave it
/// a title, even one equal to the provisional title (a short first
/// message), or its title is no longer the provisional one.
pub(super) async fn run_title_worker(
    titles: AgentTitleApplication,
    blocking: Arc<BlockingService>,
    mut receiver: mpsc::Receiver<TitleWork>,
    cancellation: CancellationToken,
) {
    let mut settled = HashSet::new();
    loop {
        let work = tokio::select! {
            work = receiver.recv() => match work {
                Some(work) => work,
                None => return,
            },
            () = wait_for_cancellation(cancellation.clone()) => return,
        };
        if settled.contains(&work.session_id) {
            continue;
        }
        let session_id = work.session_id;
        if title_turn(&titles, &blocking, work, &cancellation).await {
            settled.insert(session_id);
        }
    }
}

/// Reads the session's first message and its Provider, asks the model for
/// a title while the session's title is that message's provisional one, and
/// commits it if the session still has it. Returns whether the session is
/// settled: the model gave a title, or the title is not provisional. Every
/// failure leaves the provisional title in place, for the next user turn
/// to try again.
async fn title_turn(
    titles: &AgentTitleApplication,
    blocking: &BlockingService,
    work: TitleWork,
    cancellation: &CancellationToken,
) -> bool {
    let TitleWork {
        session_id,
        turn_id,
        generation,
        title: provisional_title,
    } = work;
    let reader = titles.clone();
    let expected = provisional_title.clone();
    let request = match blocking
        .run("agent title request", move || {
            reader.title_request(session_id, &expected)
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|request| request)
    {
        Ok(Some(request)) => request,
        Ok(None) => {
            log::debug!(
                "Agent automatic title skipped: the title is not provisional session={session_id} turn={turn_id} generation={generation}"
            );
            return true;
        }
        Err(error) => {
            log::debug!(
                "Agent automatic title Provider resolution failed session={session_id}: {error}"
            );
            return false;
        }
    };
    let model_session = match request.model.model_session() {
        Ok(model_session) => model_session,
        Err(error) => {
            log::debug!(
                "Agent automatic title transport preparation failed session={session_id}: {error}"
            );
            return false;
        }
    };
    let title = match generate_title(
        model_session,
        request.model.model,
        TITLE_INSTRUCTIONS,
        request.prompt,
        cancellation.clone(),
    )
    .await
    {
        Ok(title) => title,
        Err(error) => {
            if !cancellation.is_cancelled() {
                log::debug!(
                    "Agent automatic title generation failed session={session_id}: {error}"
                );
            }
            return false;
        }
    };
    // A plugin turned off while the model answered commits nothing.
    if cancellation.is_cancelled() {
        return false;
    }
    let writer = titles.clone();
    let committed = blocking
        .run("agent title commit", move || {
            writer.replace_title(session_id, &provisional_title, title)
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|committed| committed);
    match committed {
        Ok(true) => {}
        Ok(false) => log::debug!(
            "Agent automatic title dropped: the session's title changed session={session_id}"
        ),
        Err(error) => {
            log::debug!("Agent automatic title commit failed session={session_id}: {error}")
        }
    }
    true
}
