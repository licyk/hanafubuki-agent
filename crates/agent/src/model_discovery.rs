//! Model discovery: the Agent lists its Providers' models by itself, in the
//! background, so what a listing reports (a model's context window) is
//! known before anyone opens a model list. One worker in the Agent's worker
//! group lists one Provider at a time, through the kernel's
//! `list_provider_models` (the Provider's own endpoint, token and headers,
//! as its requests), which records the listing in the model metadata and
//! so in `model-metadata.json`.
//!
//! A listing is asked for at start (the default Provider) and whenever a
//! session is bound to a Provider: created on it, given one of its models
//! (`/model`, a role's model) or about to run a turn on it. Asking never
//! waits. A Provider is listed again only once a day after a listing, and
//! once every 15 minutes after a failed one. The day counts from the
//! endpoint's last recorded listing, which the model metadata keeps across
//! runs, so a new run does not list again; the failures are this run's. A
//! failure is logged without what the Provider said, and is never a turn's
//! error. `modelMetadata.discovery = "off"` stops it all; the listings the
//! person asks for still record.
//!
//! The same worker, at the same requests, downloads models.dev's catalogue
//! when `modelMetadata.modelsDev` is on (off by default: a request to a
//! third party), at most once a day after a download and every 15 minutes
//! after a failed one, the day counted from the copy the model metadata
//! keeps. A failure is silent (logged at debug, without what models.dev
//! said): the copy kept is used until it is a week old. Turned off, the
//! copy is dropped.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hanafubuki_agent_config::{AgentConfig, AgentModelDiscovery, AgentSettings};
use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_kernel::{fetch_models_dev, AgentService, ModelApiProtocol, MODELS_DEV_URL};

use crate::model_api_protocol;
use crate::service::{resolve_settings_provider, ResolvedAgentProvider};

/// How long the worker waits for a request before it looks whether the
/// Agent is stopping.
const POLL: Duration = Duration::from_millis(100);

/// When the Agent lists a Provider, or downloads models.dev, by itself.
#[derive(Clone, Debug)]
pub(crate) struct ModelDiscoveryPolicy {
    /// Whether the worker runs; when it does, `modelMetadata.discovery`
    /// decides each listing and `modelMetadata.modelsDev` each download.
    enabled: bool,
    /// How long after a listing (or download) it is not done again.
    after_success: Duration,
    /// How long after a failed one.
    after_failure: Duration,
    /// Where models.dev's catalogue is downloaded from.
    models_dev_url: Cow<'static, str>,
}

impl ModelDiscoveryPolicy {
    /// The Agent's: a day after a listing, 15 minutes after a failure.
    pub(crate) const AUTOMATIC: Self = Self {
        enabled: true,
        after_success: Duration::from_secs(24 * 60 * 60),
        after_failure: Duration::from_secs(15 * 60),
        models_dev_url: Cow::Borrowed(MODELS_DEV_URL),
    };

    /// No discovery whatever the settings: the test Agents', so a test's
    /// scripted model server sees only the requests the test makes.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) const OFF: Self = Self {
        enabled: false,
        after_success: Duration::ZERO,
        after_failure: Duration::ZERO,
        models_dev_url: Cow::Borrowed(""),
    };

    /// Discovery with the periods a test gives, models.dev at an address
    /// where nothing listens (never the network).
    #[cfg(test)]
    pub(crate) fn every(after_success: Duration, after_failure: Duration) -> Self {
        Self {
            enabled: true,
            after_success,
            after_failure,
            models_dev_url: Cow::Borrowed("http://127.0.0.1:9/api.json"),
        }
    }

    /// This policy, models.dev at `url`.
    #[cfg(test)]
    pub(crate) fn with_models_dev(self, url: String) -> Self {
        Self {
            models_dev_url: Cow::Owned(url),
            ..self
        }
    }
}

/// Asks the discovery worker for listings; a no-op when discovery is off.
#[derive(Clone, Default)]
pub(crate) struct ModelDiscovery {
    /// The Provider to list, or `None` for the default one.
    requests: Option<Sender<Option<String>>>,
    /// The requests the worker has handled, listed or not.
    #[cfg(test)]
    handled: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl ModelDiscovery {
    /// Starts the worker in `workers` under `policy`, and asks it for the
    /// default Provider's listing.
    pub(crate) fn start(
        policy: ModelDiscoveryPolicy,
        core: AgentService,
        config: AgentConfig,
        workers: &BackgroundWorkerGroup,
    ) -> Result<Self, String> {
        if !policy.enabled {
            return Ok(Self::default());
        }
        let (sender, receiver) = mpsc::channel();
        let discovery = Self {
            requests: Some(sender),
            #[cfg(test)]
            handled: Default::default(),
        };
        let worker = Worker {
            policy,
            core,
            config,
            attempts: HashMap::new(),
            models_dev_attempt: None,
            #[cfg(test)]
            handled: discovery.handled.clone(),
        };
        workers.spawn("agent-model-discovery", move |cancellation| {
            worker.run(&receiver, &cancellation)
        })?;
        discovery.request(None);
        Ok(discovery)
    }

    /// Asks for the listing of the Provider `provider_id` (the default
    /// Provider with none), if it is due; never waits.
    pub(crate) fn request(&self, provider_id: Option<&str>) {
        if let Some(requests) = &self.requests {
            // The worker is gone only once the Agent stops.
            let _ = requests.send(provider_id.map(str::to_string));
        }
    }

    /// How many requests the worker has handled.
    #[cfg(test)]
    pub(crate) fn handled(&self) -> u64 {
        self.handled.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// The worker's last listing of one endpoint, this run.
struct Attempt {
    at: Instant,
    succeeded: bool,
}

struct Worker {
    policy: ModelDiscoveryPolicy,
    core: AgentService,
    config: AgentConfig,
    /// By protocol and base URL.
    attempts: HashMap<(ModelApiProtocol, String), Attempt>,
    models_dev_attempt: Option<Attempt>,
    #[cfg(test)]
    handled: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Worker {
    fn run(mut self, requests: &Receiver<Option<String>>, cancellation: &CancellationToken) {
        let runtime = match crate::supervisor_runtime::build_agent_supervisor_runtime() {
            Ok(runtime) => runtime,
            Err(error) => {
                log::error!(
                    "the model discovery's runtime failed, so no models are discovered: {error}"
                );
                return;
            }
        };
        while !cancellation.is_cancelled() {
            let provider_id = match requests.recv_timeout(POLL) {
                Ok(provider_id) => provider_id,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            if let Ok(settings) = self.config.settings() {
                if let Some(provider) = self.due_provider(&settings, provider_id) {
                    runtime.block_on(self.list(provider, cancellation));
                }
                runtime.block_on(self.refresh_models_dev(&settings, cancellation));
            }
            #[cfg(test)]
            self.handled
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// The Provider `provider_id` names (the default one with none), when
    /// discovery is on and its listing is due.
    fn due_provider(
        &self,
        settings: &AgentSettings,
        provider_id: Option<String>,
    ) -> Option<ResolvedAgentProvider> {
        if settings.model_metadata.discovery == AgentModelDiscovery::Off {
            return None;
        }
        let registry = self.config.providers().ok()?;
        let provider_id = provider_id.or(registry.default_provider_id.clone())?;
        let provider = match resolve_settings_provider(&registry, &provider_id) {
            Ok(provider) => provider,
            Err(_) => {
                log::debug!(
                    "model discovery: Provider {provider_id} is not usable, so it is not listed"
                );
                return None;
            }
        };
        let key = endpoint(&provider);
        let recorded_at_ms = self.core.model_listing_recorded_at(key.0, &key.1);
        let attempt = self.attempts.get(&key);
        due(
            &self.policy,
            attempt,
            recorded_at_ms,
            Instant::now(),
            now_ms(),
        )
        .then_some(provider)
    }

    /// Lists `provider`; the kernel records what it reports.
    async fn list(&mut self, provider: ResolvedAgentProvider, cancellation: &CancellationToken) {
        let key = endpoint(&provider);
        let definition = provider.definition;
        // A failed attempt until it succeeds: a listing that fails, or is
        // cancelled, waits out the failure's period.
        self.attempts.insert(
            key.clone(),
            Attempt {
                at: Instant::now(),
                succeeded: false,
            },
        );
        log::debug!(
            "model discovery: listing the models of Provider {}",
            definition.id
        );
        let listed = self
            .core
            .list_provider_models(
                key.0,
                &definition.base_url,
                provider.credential,
                provider.headers,
                cancellation.clone(),
            )
            .await;
        match listed {
            Ok(_) => {
                if let Some(attempt) = self.attempts.get_mut(&key) {
                    attempt.succeeded = true;
                }
            }
            // The Agent is stopping.
            Err(_) if cancellation.is_cancelled() => {}
            // What the Provider answered is not logged: it is its content,
            // and may echo a credential.
            Err(_) => log::warn!(
                "model discovery: the models of Provider {} could not be listed; it is tried again in {} minutes at the earliest",
                definition.id,
                self.policy.after_failure.as_secs().div_ceil(60)
            ),
        }
    }

    /// Downloads models.dev's catalogue into the model metadata when it is
    /// on and due; drops the copy kept when it is off.
    async fn refresh_models_dev(
        &mut self,
        settings: &AgentSettings,
        cancellation: &CancellationToken,
    ) {
        if !settings.model_metadata.models_dev {
            if self.core.forget_models_dev_catalog() {
                log::debug!("models.dev is turned off, so its copy is dropped");
            }
            return;
        }
        let fetched_at_ms = self.core.models_dev_fetched_at();
        let attempt = self.models_dev_attempt.as_ref();
        if !due(
            &self.policy,
            attempt,
            fetched_at_ms,
            Instant::now(),
            now_ms(),
        ) {
            return;
        }
        self.models_dev_attempt = Some(Attempt {
            at: Instant::now(),
            succeeded: false,
        });
        match fetch_models_dev(&self.policy.models_dev_url, cancellation).await {
            Ok(catalog) => {
                log::debug!("models.dev: {} models fetched", catalog.model_count());
                self.core.record_models_dev_catalog(catalog);
                if let Some(attempt) = self.models_dev_attempt.as_mut() {
                    attempt.succeeded = true;
                }
            }
            Err(_) if cancellation.is_cancelled() => {}
            // A fallback source: its failure is no one's error. The error
            // names what failed, never what models.dev answered.
            Err(error) => log::debug!(
                "models.dev's catalogue was not fetched ({error}); the copy kept, if any, is used until it is a week old"
            ),
        }
    }
}

/// The endpoint `provider` is listed at: its protocol and base URL.
fn endpoint(provider: &ResolvedAgentProvider) -> (ModelApiProtocol, String) {
    (
        model_api_protocol(provider.definition.protocol),
        provider.definition.base_url.clone(),
    )
}

/// Whether an endpoint is listed again: this run's last attempt (if any)
/// is older than its period, and the endpoint's last recorded listing (if
/// any, by any run or any listing the person asked for) older than a
/// successful listing's period. A recorded time ahead of now (a clock set
/// back) does not hold the listing off.
fn due(
    policy: &ModelDiscoveryPolicy,
    attempt: Option<&Attempt>,
    recorded_at_ms: Option<u64>,
    now: Instant,
    now_ms: u64,
) -> bool {
    let attempt_expired = attempt.is_none_or(|attempt| {
        let period = if attempt.succeeded {
            policy.after_success
        } else {
            policy.after_failure
        };
        now.saturating_duration_since(attempt.at) >= period
    });
    let after_success_ms = u64::try_from(policy.after_success.as_millis()).unwrap_or(u64::MAX);
    let recorded_expired =
        recorded_at_ms.is_none_or(|at| at > now_ms || now_ms - at >= after_success_ms);
    attempt_expired && recorded_expired
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
