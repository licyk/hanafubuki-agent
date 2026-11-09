//! The `titles` plugin: a session's first user turn gets a provisional
//! title at admission (from the submit command); this plugin replaces it
//! with one the session's model generates. Its observer queues the work and
//! its worker runs it, one title at a time, until the plugin stops. It also
//! provides the title suggestions a rename offers ([`TitleService`]) under
//! its name.

mod generation;
mod suggestion;
mod worker;

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    spawn_scoped_worker, AgentGeneration, AgentNotification, AgentObserver, AgentSessionId,
    AgentTurnId, AgentTurnOrigin, BlockingService, ObserverRegistry, PluginFaults, Register,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use kardia_core::{Context, Disposer, Inject, PluginDef, Service};
use tokio::sync::mpsc;

pub(crate) use suggestion::TitleService;

use crate::titles::AgentTitleApplication;

/// The plugin's name, under which it provides its [`TitleService`].
pub(crate) const TITLES_PLUGIN: &str = "titles";

/// Titles waiting for the worker. A turn whose work does not fit keeps its
/// provisional title.
const TITLE_QUEUE_CAPACITY: usize = 32;

/// Registers the title observer, runs the title worker over `titles` and
/// provides the title suggestions.
pub(crate) fn titles_plugin(titles: AgentTitleApplication) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let titles = titles.clone();
        async move {
            let blocking = ctx.get::<BlockingService>()?.arc().clone();
            let cancellation = CancellationToken::default();
            let stopping = cancellation.clone();
            ctx.defer(
                "title suggestions",
                Disposer::new(move || stopping.cancel()),
            )?;
            ctx.provide_arc_as(
                TITLES_PLUGIN,
                Arc::new(TitleService {
                    conversations: titles.conversations(),
                    blocking: blocking.clone(),
                    cancellation,
                }),
            )?;
            let (sender, receiver) = mpsc::channel(TITLE_QUEUE_CAPACITY);
            spawn_scoped_worker(&ctx, "agent-titles", move |cancellation| {
                worker::run_title_worker(titles, blocking, receiver, cancellation)
            })?;
            let observers = ctx.get::<ObserverRegistry>()?;
            observers.register(Arc::new(TitleObserver { sender }))?;
            Ok(())
        }
    })
    .name(TITLES_PLUGIN)
    .provides([TITLES_PLUGIN])
    .inject(
        Inject::new()
            .with(ObserverRegistry::NAME)
            .with(BlockingService::NAME)
            .with(PluginFaults::NAME),
    )
}

/// One turn whose session took a provisional title at admission.
struct TitleWork {
    session_id: AgentSessionId,
    turn_id: AgentTurnId,
    generation: AgentGeneration,
    /// The automatic title the generated one may replace.
    provisional_title: String,
}

/// Queues a title for every user turn that gave its session a provisional
/// title. A programmatic turn, or a session that already had a title, is
/// left alone.
struct TitleObserver {
    sender: mpsc::Sender<TitleWork>,
}

impl AgentObserver for TitleObserver {
    fn id(&self) -> &str {
        "titles"
    }

    fn observe(&self, notification: &AgentNotification) {
        let AgentNotification::TurnAdmitted {
            session_id,
            turn_id,
            generation,
            origin: AgentTurnOrigin::User,
            provisional_title: Some(provisional_title),
        } = notification
        else {
            return;
        };
        if let Err(error) = self.sender.try_send(TitleWork {
            session_id: *session_id,
            turn_id: *turn_id,
            generation: *generation,
            provisional_title: provisional_title.clone(),
        }) {
            log::debug!("Agent automatic title queue rejected work session={session_id}: {error}");
        }
    }
}

#[cfg(test)]
mod tests;
