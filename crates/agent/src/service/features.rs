//! The facade's feature calls: operations a plugin row serves through the
//! service it provides under its name, so that a failing feature costs only
//! itself (`site/content/docs/plugin-rules.en.mdx`, "Host, configuration
//! and status").

use std::any::Any;
use std::future::Future;
use std::sync::Arc;

use hanafubuki_agent_host::FaultPoint;
use hanafubuki_agent_kernel::{AgentError, AgentSessionId};

use super::AgentApplicationService;
use crate::plugins::{AgentRecap, RecapService, TitleService, RECAP_PLUGIN, TITLES_PLUGIN};

impl AgentApplicationService {
    /// A brief catch-up on `session_id` from its latest exchanges, by the
    /// session's model; `FeatureUnavailable` while the `recap` plugin is not
    /// running. Nothing is committed.
    pub async fn recap_session(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentRecap, AgentError> {
        self.feature(RECAP_PLUGIN, |recaps: Arc<RecapService>| async move {
            recaps.recap(session_id).await
        })
        .await
    }

    /// A title for `session_id`'s conversation as it stands, by the
    /// session's model, for the user to accept or edit; `FeatureUnavailable`
    /// while the `titles` plugin is not running. Nothing is committed.
    pub async fn suggest_session_title(
        &self,
        session_id: AgentSessionId,
    ) -> Result<String, AgentError> {
        self.feature(TITLES_PLUGIN, |titles: Arc<TitleService>| async move {
            titles.suggest(session_id).await
        })
        .await
    }

    /// Runs `call` on the service of type `T` the row `feature` provides
    /// under its name, resolved for this call only.
    ///
    /// A row that is not running is `FeatureUnavailable`, with why. A panic
    /// in `call` is that row's fault: the host stops the plugin and starts
    /// it again under its restart policy, and this call is
    /// `FeatureUnavailable`; nothing else is touched.
    pub(crate) async fn feature<T, F, Fut, R>(
        &self,
        feature: &'static str,
        call: F,
    ) -> Result<R, AgentError>
    where
        T: Any + Send + Sync,
        F: FnOnce(Arc<T>) -> Fut,
        Fut: Future<Output = Result<R, AgentError>>,
    {
        self.host.ensure_started().await;
        let service =
            self.host
                .service::<T>(feature)
                .map_err(|reason| AgentError::FeatureUnavailable {
                    feature: feature.to_string(),
                    reason,
                })?;
        self.host
            .faults()
            .guard_async(feature, FaultPoint::new("feature call"), || call(service))
            .await
            .unwrap_or_else(|_| {
                Err(AgentError::FeatureUnavailable {
                    feature: feature.to_string(),
                    reason: format!("the {feature} Agent plugin stopped after an internal error"),
                })
            })
    }
}
