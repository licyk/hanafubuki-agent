//! models.dev (`https://models.dev/api.json`), a public catalogue of models
//! by provider, as an optional source of what a model can do: its context
//! window, its largest output, whether it reasons and whether it reads
//! images. It is a third party's data: read tolerantly (a provider or a
//! model that does not read is left out, a fact that does not read is
//! unknown) and kept add-only, so a "no" is dropped and only the limits
//! and a "yes" are kept.
//!
//! A model is looked up by its normalized id (lowercase, without a
//! `vendor/` prefix, a trailing `-latest` or a trailing `-YYYYMMDD` date),
//! under the Provider's own entry first ([`ModelProviderNames`]), else under
//! every provider that lists it; where several entries answer, the smallest
//! limit and any "yes" are taken.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use reqwest::header::ACCEPT;
use reqwest::Url;
use serde::de::{DeserializeOwned, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::port::{ModelError, ModelResult};
use crate::streaming::wait_cancelled;
use crate::ModelHttpProxy;
use crate::{
    ModelEndpoint, ModelMetadata, ModelMetadataSource, ModelReasoningSupport, Sourced,
    PLAUSIBLE_CONTEXT_WINDOW,
};

/// Where the catalogue is published.
pub const MODELS_DEV_URL: &str = "https://models.dev/api.json";

/// The Agent and its version, and nothing of the person.
const USER_AGENT: &str = concat!("hanafubuki-agent/", env!("CARGO_PKG_VERSION"));
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The whole request, body included.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// What models.dev says of each model, by provider, as the Agent keeps it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelsDevCatalog {
    #[serde(default, deserialize_with = "tolerant_entries")]
    providers: BTreeMap<String, ModelsDevProvider>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelsDevProvider {
    /// The host of the provider's API, when models.dev names one.
    #[serde(
        default,
        deserialize_with = "tolerant",
        skip_serializing_if = "Option::is_none"
    )]
    api_host: Option<String>,
    /// By normalized id.
    #[serde(default, deserialize_with = "tolerant_entries")]
    models: BTreeMap<String, ModelsDevModel>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelsDevModel {
    #[serde(
        default,
        deserialize_with = "tolerant",
        skip_serializing_if = "Option::is_none"
    )]
    context_window: Option<u64>,
    #[serde(
        default,
        deserialize_with = "tolerant",
        skip_serializing_if = "Option::is_none"
    )]
    max_output_tokens: Option<u64>,
    #[serde(default, deserialize_with = "tolerant", skip_serializing_if = "is_no")]
    reasoning: bool,
    #[serde(default, deserialize_with = "tolerant", skip_serializing_if = "is_no")]
    image_input: bool,
}

/// What a configured Provider is called: its id and its base URL's host,
/// which pick its own entries in models.dev.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelProviderNames {
    id: String,
    host: Option<String>,
}

impl ModelProviderNames {
    pub fn new(provider_id: &str, endpoint: &ModelEndpoint) -> Self {
        Self {
            id: provider_id.trim().to_ascii_lowercase(),
            host: endpoint.host().map(str::to_ascii_lowercase),
        }
    }

    /// Whether models.dev's provider `key` is this Provider: its key is the
    /// Provider's id or a word of it, or a label of the base URL's host, or
    /// its API is at that host.
    fn own(&self, key: &str, provider: &ModelsDevProvider) -> bool {
        let words = self.id.split(|c: char| !c.is_ascii_alphanumeric());
        self.id == key
            || words.into_iter().any(|word| word == key)
            || self.host.as_deref().is_some_and(|host| {
                provider.api_host.as_deref() == Some(host)
                    || host.split('.').any(|label| label == key)
            })
    }
}

impl ModelsDevCatalog {
    /// The catalogue in models.dev's own form (`{provider: {api, models:
    /// {id: {limit, reasoning, modalities}}}}`); fails only when it is not
    /// a JSON object of providers, or names no model the Agent can use.
    pub fn from_api_json(body: &[u8]) -> ModelResult<Self> {
        let raw: RawCatalog = serde_json::from_slice(body)
            .map_err(|_| ModelError::Protocol("models.dev answered no catalogue".into()))?;
        if raw.skipped > 0 {
            log::debug!(
                "models.dev: {} entries do not read and are left out",
                raw.skipped
            );
        }
        let providers: BTreeMap<String, ModelsDevProvider> = raw
            .providers
            .into_iter()
            .map(|(key, provider)| (key.to_ascii_lowercase(), provider.trimmed()))
            .filter(|(_, provider)| !provider.models.is_empty())
            .collect();
        if providers.is_empty() {
            return Err(ModelError::Protocol(
                "models.dev's catalogue names no model".into(),
            ));
        }
        Ok(Self { providers })
    }

    /// How many models it describes, over every provider.
    pub fn model_count(&self) -> usize {
        self.providers
            .values()
            .map(|provider| provider.models.len())
            .sum()
    }

    /// What it says of `model` at the Provider `names` describes (none: a
    /// Provider it cannot name), each fact from models.dev and observed at
    /// `observed_at_ms`.
    pub fn metadata(
        &self,
        names: Option<&ModelProviderNames>,
        model: &str,
        observed_at_ms: u64,
    ) -> ModelMetadata {
        let id = normalized_model_id(model);
        let own: Vec<ModelsDevModel> = self
            .providers
            .iter()
            .filter(|(key, provider)| names.is_some_and(|names| names.own(key, provider)))
            .filter_map(|(_, provider)| provider.models.get(&id).copied())
            .collect();
        let found = if own.is_empty() {
            self.providers
                .values()
                .filter_map(|provider| provider.models.get(&id).copied())
                .reduce(ModelsDevModel::merged)
        } else {
            own.into_iter().reduce(ModelsDevModel::merged)
        };
        found.map_or_else(ModelMetadata::default, |model| {
            model.metadata(observed_at_ms)
        })
    }
}

impl ModelsDevModel {
    /// What two entries of one model say together: the smaller limit (one
    /// set too small only compacts early) and any "yes".
    fn merged(self, other: Self) -> Self {
        fn smaller(left: Option<u64>, right: Option<u64>) -> Option<u64> {
            match (left, right) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (left, right) => left.or(right),
            }
        }
        Self {
            context_window: smaller(self.context_window, other.context_window),
            max_output_tokens: smaller(self.max_output_tokens, other.max_output_tokens),
            reasoning: self.reasoning || other.reasoning,
            image_input: self.image_input || other.image_input,
        }
    }

    fn metadata(self, observed_at_ms: u64) -> ModelMetadata {
        let sourced = |value| Sourced {
            value,
            source: ModelMetadataSource::Catalog,
            observed_at_ms,
        };
        ModelMetadata {
            context_window: self.context_window.map(sourced),
            max_output_tokens: self.max_output_tokens.map(sourced),
            reasoning: self.reasoning.then_some(Sourced {
                value: ModelReasoningSupport::Supported {
                    efforts: Vec::new(),
                    default: None,
                },
                source: ModelMetadataSource::Catalog,
                observed_at_ms,
            }),
            image_input: self.image_input.then_some(Sourced {
                value: true,
                source: ModelMetadataSource::Catalog,
                observed_at_ms,
            }),
            structured_output: None,
        }
    }
}

/// Downloads the catalogue at `url` (`MODELS_DEV_URL`, else a test's), with
/// the Agent's User-Agent and no credential, within a minute and its own
/// size bound; cancelled with `cancellation`.
pub async fn fetch_models_dev(
    url: &str,
    proxy: &ModelHttpProxy,
    cancellation: &CancellationToken,
) -> ModelResult<ModelsDevCatalog> {
    let fetched = tokio::select! {
        fetched = tokio::time::timeout(REQUEST_TIMEOUT, fetch(url, proxy)) => fetched,
        _ = wait_cancelled(cancellation) => return Err(ModelError::Cancelled),
    };
    fetched.map_err(|_| ModelError::Protocol("models.dev did not answer in time".into()))?
}

async fn fetch(url: &str, proxy: &ModelHttpProxy) -> ModelResult<ModelsDevCatalog> {
    let builder = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(USER_AGENT);
    let client = proxy
        .apply(builder)?
        .build()
        .map_err(|_| ModelError::InvalidRequest("failed to build the models.dev client".into()))?;
    let unreachable = |_| ModelError::Protocol("models.dev could not be reached".into());
    let mut response = client
        .get(url)
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(unreachable)?;
    if !response.status().is_success() {
        return Err(ModelError::Protocol(format!(
            "models.dev answered {}",
            response.status().as_u16()
        )));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(unreachable)? {
        if body.len().saturating_add(chunk.len()) > MAX_BODY_BYTES {
            return Err(ModelError::Protocol(format!(
                "models.dev's catalogue exceeded {MAX_BODY_BYTES} bytes"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    ModelsDevCatalog::from_api_json(&body)
}

/// `model` as models.dev's ids are compared: lowercase, after its last
/// `/`, without a trailing `-latest`, then without a trailing `-YYYYMMDD`.
fn normalized_model_id(model: &str) -> String {
    let model = model.trim().to_ascii_lowercase();
    let model = model.rsplit('/').next().unwrap_or_default();
    let model = model.strip_suffix("-latest").unwrap_or(model);
    let dated = model
        .rsplit_once('-')
        .filter(|(_, date)| date.len() == 8 && date.bytes().all(|byte| byte.is_ascii_digit()));
    dated.map_or(model, |(model, _)| model).to_owned()
}

/// models.dev's document: its providers, each read on its own.
struct RawCatalog {
    providers: Vec<(String, RawProvider)>,
    skipped: usize,
}

impl<'de> Deserialize<'de> for RawCatalog {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Providers;
        impl<'de> Visitor<'de> for Providers {
            type Value = RawCatalog;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object of providers")
            }

            // One provider at a time, so the document is never held whole
            // as a `Value`.
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RawCatalog, A::Error> {
                let mut catalog = RawCatalog {
                    providers: Vec::new(),
                    skipped: 0,
                };
                while let Some(key) = map.next_key::<String>()? {
                    let value: Value = map.next_value()?;
                    match serde_json::from_value::<RawProvider>(value) {
                        Ok(provider) => {
                            catalog.skipped += provider.models.1;
                            catalog.providers.push((key, provider));
                        }
                        Err(_) => catalog.skipped += 1,
                    }
                }
                Ok(catalog)
            }
        }
        deserializer.deserialize_map(Providers)
    }
}

#[derive(Deserialize)]
struct RawProvider {
    #[serde(default, deserialize_with = "tolerant")]
    api: Option<String>,
    /// The models that read, and how many did not.
    #[serde(default, deserialize_with = "tolerant_raw_models")]
    models: ReadEntries<RawModel>,
}

#[derive(Deserialize)]
struct RawModel {
    #[serde(default, deserialize_with = "tolerant")]
    reasoning: Option<bool>,
    #[serde(default, deserialize_with = "tolerant")]
    limit: Option<RawLimit>,
    #[serde(default, deserialize_with = "tolerant")]
    modalities: Option<RawModalities>,
}

#[derive(Deserialize)]
struct RawLimit {
    #[serde(default, deserialize_with = "tolerant")]
    context: Option<u64>,
    #[serde(default, deserialize_with = "tolerant")]
    output: Option<u64>,
}

#[derive(Deserialize)]
struct RawModalities {
    #[serde(default)]
    input: Value,
}

impl RawProvider {
    /// The provider as the Agent keeps it: its API's host, and its models
    /// by normalized id, the entries one id gathers merged.
    fn trimmed(self) -> ModelsDevProvider {
        let mut models = BTreeMap::<String, ModelsDevModel>::new();
        for (id, model) in self.models.0 {
            let model = model.trimmed();
            if model == ModelsDevModel::default() {
                continue;
            }
            let id = normalized_model_id(&id);
            let merged = match models.get(&id) {
                Some(known) => known.merged(model),
                None => model,
            };
            models.insert(id, merged);
        }
        ModelsDevProvider {
            api_host: self
                .api
                .and_then(|api| Url::parse(&api).ok())
                .and_then(|api| api.host_str().map(str::to_ascii_lowercase)),
            models,
        }
    }
}

impl RawModel {
    fn trimmed(self) -> ModelsDevModel {
        let (context, output) = self
            .limit
            .map_or((None, None), |limit| (limit.context, limit.output));
        let reads_images = self
            .modalities
            .as_ref()
            .and_then(|modalities| modalities.input.as_array())
            .is_some_and(|input| input.iter().any(|kind| kind.as_str() == Some("image")));
        ModelsDevModel {
            context_window: context.filter(|tokens| PLAUSIBLE_CONTEXT_WINDOW.contains(tokens)),
            max_output_tokens: output.filter(|tokens| *tokens > 0),
            reasoning: self.reasoning == Some(true),
            image_input: reads_images,
        }
    }
}

fn is_no(value: &bool) -> bool {
    !*value
}

/// A field that does not read is unknown.
fn tolerant<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// A map whose entries are read one by one, an entry that does not read
/// left out; anything but an object is empty.
fn tolerant_entries<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let (entries, _) = tolerant_pairs(Value::deserialize(deserializer)?);
    Ok(entries.into_iter().collect())
}

fn tolerant_raw_models<'de, D>(deserializer: D) -> Result<ReadEntries<RawModel>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(tolerant_pairs(Value::deserialize(deserializer)?))
}

/// The entries of an object that read, and how many did not.
type ReadEntries<T> = (Vec<(String, T)>, usize);

/// The entries of `value` that read, and how many did not.
fn tolerant_pairs<T: DeserializeOwned>(value: Value) -> ReadEntries<T> {
    let Value::Object(entries) = value else {
        return (Vec::new(), 0);
    };
    let total = entries.len();
    let read: Vec<(String, T)> = entries
        .into_iter()
        .filter_map(|(key, value)| Some((key, serde_json::from_value(value).ok()?)))
        .collect();
    let skipped = total - read.len();
    (read, skipped)
}

#[cfg(test)]
mod tests;
