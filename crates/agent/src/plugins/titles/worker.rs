use std::sync::Arc;

use hanafubuki_agent_contracts::BlockingService;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use tokio::sync::mpsc;

use super::generation::{generate_title, wait_for_cancellation};
use super::TitleWork;
use crate::titles::AgentTitleApplication;

/// Generates the queued titles one after another until the plugin stops.
pub(super) async fn run_title_worker(
    titles: AgentTitleApplication,
    blocking: Arc<BlockingService>,
    mut receiver: mpsc::Receiver<TitleWork>,
    cancellation: CancellationToken,
) {
    loop {
        let work = tokio::select! {
            work = receiver.recv() => match work {
                Some(work) => work,
                None => return,
            },
            () = wait_for_cancellation(cancellation.clone()) => return,
        };
        title_turn(&titles, &blocking, work, &cancellation).await;
    }
}

/// Reads the turn's prompt and the session's Provider, asks the model for a
/// title and commits it if the session still has the provisional one.
/// Every failure leaves the provisional title in place.
async fn title_turn(
    titles: &AgentTitleApplication,
    blocking: &BlockingService,
    work: TitleWork,
    cancellation: &CancellationToken,
) {
    let TitleWork {
        session_id,
        turn_id,
        generation,
        provisional_title,
    } = work;
    let reader = titles.clone();
    let request = match blocking
        .run("agent title request", move || {
            reader.title_request(session_id, turn_id)
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|request| request)
    {
        Ok(Some(request)) => request,
        Ok(None) => {
            log::debug!(
                "Agent automatic title skipped: the turn's message is gone session={session_id} turn={turn_id} generation={generation}"
            );
            return;
        }
        Err(error) => {
            log::debug!(
                "Agent automatic title Provider resolution failed session={session_id}: {error}"
            );
            return;
        }
    };
    let model_session = match request.model_session() {
        Ok(model_session) => model_session,
        Err(error) => {
            log::debug!(
                "Agent automatic title transport preparation failed session={session_id}: {error}"
            );
            return;
        }
    };
    let title = match generate_title(
        model_session,
        request.model,
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
            return;
        }
    };
    // A plugin turned off while the model answered commits nothing.
    if cancellation.is_cancelled() {
        return;
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
}
