//! The image input a session's model takes: the person's policy over what
//! the model metadata knows of the model's route. A session takes it when
//! it is created, recovered or given a model, and again before each turn's
//! admission, steering, sampling step, tool call and snapshot, so a fact a
//! listing reported or a refusal taught since applies at once.

use hanafubuki_agent_model_api::{ModelInputCapabilities, ModelRouteKey};

use super::model_facts::configured_image_input;
use super::*;

impl AgentService {
    /// The image input a session configured as `config` takes over
    /// `model`: none when the person disabled it, else none when the route
    /// is known not to read images, else images are sent (the model reads
    /// them, or nothing says, and a refusal falls back once). A transport
    /// that names no route has nothing known of it.
    pub(super) fn session_input_capabilities(
        &self,
        model: &dyn ModelPort,
        config: &AgentSessionConfig,
    ) -> ModelInputCapabilities {
        self.input_capabilities_at(model.route_key(&config.model).as_ref(), config)
    }

    /// Gives the session the image input its route resolves to now.
    pub(super) async fn refresh_session_image_input_capability(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentError> {
        let cell = self.session(session_id)?;
        let mut record = cell.lock_after_persistence().await?;
        self.refresh_image_input(&mut record);
        Ok(())
    }

    /// `refresh_session_image_input_capability` for synchronous callers off
    /// the async runtime.
    pub(super) fn refresh_session_image_input_capability_blocking(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentError> {
        let cell = self.session(session_id)?;
        let mut record = cell.lock_after_persistence_blocking()?;
        self.refresh_image_input(&mut record);
        Ok(())
    }

    /// A session whose transport names no route keeps what it has: the
    /// refusal it fell back from (`record_unsupported_image_input`) stays.
    pub(super) fn refresh_image_input(&self, record: &mut SessionRecord) {
        let settings = &record.settings;
        let Some(route) = settings.model.route_key(&settings.config.model) else {
            return;
        };
        let capabilities = self.input_capabilities_at(Some(&route), &settings.config);
        record.settings.effective_input_capabilities = capabilities;
    }

    fn input_capabilities_at(
        &self,
        route: Option<&ModelRouteKey>,
        config: &AgentSessionConfig,
    ) -> ModelInputCapabilities {
        let observed = route.map_or(ModelImageInputCapability::Unknown, |route| {
            self.inner.model_metadata.observed_image_input(
                route,
                configured_image_input(config),
                now_millis_u64(),
            )
        });
        config.input_capabilities.effective(observed)
    }
}

#[cfg(test)]
mod tests;
