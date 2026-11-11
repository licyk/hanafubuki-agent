//! The registry's facts per route, its listings per endpoint and its copy
//! of models.dev, with their expiry, the listing merge and its shrink
//! guard, and the bounds.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use hanafubuki_agent_model_api::{
    ModelCatalogEntry, ModelEndpointKey, ModelMetadata, ModelMetadataSource, ModelProviderNames,
    ModelReasoningSupport, ModelRouteKey, Sourced,
};
use serde::{Deserialize, Serialize};

use super::models_dev::ModelsDevCopy;
use super::store::{tolerant_or_default, ListingRecord, ModelMetadataSnapshot, RouteRecord};

pub(super) const DAY_MS: u64 = 24 * 60 * 60 * 1_000;
/// How long a learned fact holds, so a Provider that fixed its model is
/// tried again.
pub(super) const LEARNED_TTL_MS: u64 = 7 * DAY_MS;
/// How long a model missing from its Provider's latest listing keeps what
/// the listings before reported. A model the latest listing names keeps it
/// until a listing no longer does.
pub(super) const REPORTED_TTL_MS: u64 = 7 * DAY_MS;
/// One listing has up to 1000 models; the bound keeps several, and drops
/// the route observed longest ago first.
pub(super) const MAX_ROUTES: usize = 4_096;
/// The endpoints whose latest listing is kept, the one listed longest ago
/// dropped first.
pub(super) const MAX_LISTINGS: usize = 32;

/// What the Provider's listing and its refusals say about one route. Every
/// fact carries its source and when it was observed. (What models.dev says
/// is looked up by model id, not kept per route.)
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RouteFacts {
    #[serde(
        default,
        deserialize_with = "tolerant_or_default",
        skip_serializing_if = "is_unknown"
    )]
    pub(super) reported: ModelMetadata,
    #[serde(
        default,
        deserialize_with = "tolerant_or_default",
        skip_serializing_if = "is_unknown"
    )]
    pub(super) learned: ModelMetadata,
}

/// An endpoint's latest trusted listing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListingState {
    #[serde(default, deserialize_with = "tolerant_or_default")]
    pub(super) listed_at_ms: u64,
    #[serde(default, deserialize_with = "tolerant_or_default")]
    pub(super) models: BTreeSet<String>,
    /// The listing after it came back with fewer than half of `models` and
    /// was not trusted; the next one is, whatever it says.
    #[serde(default, deserialize_with = "tolerant_or_default")]
    pub(super) held: bool,
}

#[derive(Default)]
pub(super) struct RegistryState {
    pub(super) routes: HashMap<ModelRouteKey, RouteFacts>,
    pub(super) listings: HashMap<ModelEndpointKey, ListingState>,
    pub(super) models_dev: Option<Arc<ModelsDevCopy>>,
    /// What the Provider at each endpoint is called, as this run was told
    /// when it bound a session or read a model's facts there: it picks the
    /// Provider's own entries in models.dev. Not kept between runs.
    pub(super) provider_names: HashMap<ModelEndpointKey, ModelProviderNames>,
}

impl RegistryState {
    /// The state `snapshot` describes at `now_ms`: each fact stamped with
    /// its bucket's source and observed no later than now, the expired
    /// facts dropped, within the bounds.
    pub(super) fn restore(snapshot: ModelMetadataSnapshot, now_ms: u64) -> Self {
        let mut state = Self {
            routes: snapshot
                .routes
                .into_iter()
                .map(|record| (record.route, record.facts.normalized(now_ms)))
                .collect(),
            listings: snapshot
                .listings
                .into_iter()
                .map(|record| (record.endpoint, record.listing))
                .collect(),
            // A copy fetched "later" than now (a clock set back) counts
            // from now, so it does not outlive its week.
            models_dev: snapshot.models_dev.map(|copy| {
                if copy.fetched_at_ms <= now_ms {
                    copy
                } else {
                    Arc::new(ModelsDevCopy {
                        fetched_at_ms: now_ms,
                        catalog: copy.catalog.clone(),
                    })
                }
            }),
            provider_names: HashMap::new(),
        };
        state.prune(now_ms);
        state.bound(None);
        state
    }

    pub(super) fn snapshot(&self) -> ModelMetadataSnapshot {
        ModelMetadataSnapshot {
            routes: self
                .routes
                .iter()
                .map(|(route, facts)| RouteRecord {
                    route: route.clone(),
                    facts: facts.clone(),
                })
                .collect(),
            listings: self
                .listings
                .iter()
                .map(|(endpoint, listing)| ListingRecord {
                    endpoint: endpoint.clone(),
                    listing: listing.clone(),
                })
                .collect(),
            models_dev: self.models_dev.clone(),
        }
    }

    /// What models.dev says of `model`, at `route`'s Provider when the
    /// run knows what it is called: nothing without a live copy.
    pub(super) fn models_dev_facts(
        &self,
        route: Option<&ModelRouteKey>,
        model: &str,
        now_ms: u64,
    ) -> ModelMetadata {
        let Some(copy) = self.models_dev.as_ref().filter(|copy| copy.live(now_ms)) else {
            return ModelMetadata::default();
        };
        let names = route.and_then(|route| self.provider_names.get(route.endpoint()));
        copy.catalog.metadata(names, model, copy.fetched_at_ms)
    }

    /// Merges `entries`, the listing of `endpoint`, per model: a listed
    /// model's reported facts become the entry's, unknown ones included; a
    /// model the listing leaves out keeps its facts until they expire.
    /// A listing with fewer than half the models of the one before (a page
    /// cut short, more likely than a Provider dropping half its models) is
    /// not trusted, once; one that names other models as many is. Returns
    /// whether anything changed.
    pub(super) fn record_listing(
        &mut self,
        endpoint: &ModelEndpointKey,
        entries: &[ModelCatalogEntry],
        now_ms: u64,
    ) -> bool {
        let reported: Vec<(ModelRouteKey, ModelMetadata)> = entries
            .iter()
            .map(|entry| (endpoint.route(&entry.id), reported_facts(entry, now_ms)))
            .collect();
        let models: BTreeSet<String> = reported
            .iter()
            .map(|(route, _)| route.model().to_owned())
            .collect();
        if let Some(previous) = self.listings.get_mut(endpoint) {
            if models.len() * 2 < previous.models.len() && !previous.held {
                log::warn!(
                    "a model listing came back with {} of its {} models; it is recorded only when the next listing says so too",
                    models.len(),
                    previous.models.len()
                );
                previous.held = true;
                return true;
            }
        }
        self.listings.insert(
            endpoint.clone(),
            ListingState {
                listed_at_ms: now_ms,
                models,
                held: false,
            },
        );
        for (route, facts) in reported {
            self.routes.entry(route).or_default().reported = facts;
        }
        self.prune(now_ms);
        self.bound(None);
        true
    }

    /// Applies `learn` to what `route` taught the Agent.
    pub(super) fn learn(
        &mut self,
        route: &ModelRouteKey,
        now_ms: u64,
        learn: impl FnOnce(&mut ModelMetadata),
    ) -> bool {
        let mut learned = self
            .routes
            .get(route)
            .map(|facts| facts.learned.clone())
            .unwrap_or_default();
        learn(&mut learned);
        self.routes.entry(route.clone()).or_default().learned = learned;
        self.prune(now_ms);
        self.bound(Some(route));
        true
    }

    /// Drops the learned facts of `endpoint`'s routes (only `model`'s,
    /// when named). Returns how many routes had any.
    pub(super) fn forget_learned(
        &mut self,
        endpoint: &ModelEndpointKey,
        model: Option<&str>,
        now_ms: u64,
    ) -> usize {
        let mut forgotten = 0;
        for (route, facts) in &mut self.routes {
            let named =
                route.endpoint() == endpoint && model.is_none_or(|model| route.model() == model);
            if named && !is_unknown(&facts.learned) {
                facts.learned = ModelMetadata::default();
                forgotten += 1;
            }
        }
        self.prune(now_ms);
        forgotten
    }

    /// `route`'s facts that hold at `now_ms`.
    pub(super) fn live_facts(&self, route: &ModelRouteKey, now_ms: u64) -> Option<RouteFacts> {
        let mut facts = self.routes.get(route)?.clone();
        facts.expire(listed(&self.listings, route), now_ms);
        Some(facts)
    }

    /// Drops the facts that expired, the routes left with none, and a copy
    /// of models.dev past its week.
    fn prune(&mut self, now_ms: u64) {
        self.models_dev.take_if(|copy| !copy.live(now_ms));
        let listings = &self.listings;
        self.routes.retain(|route, facts| {
            facts.expire(listed(listings, route), now_ms);
            !facts.is_empty()
        });
    }

    /// Keeps at most `MAX_ROUTES` routes and `MAX_LISTINGS` listings,
    /// dropping those observed longest ago, never `keep`.
    fn bound(&mut self, keep: Option<&ModelRouteKey>) {
        if self.routes.len() > MAX_ROUTES {
            let mut oldest: Vec<(u64, ModelRouteKey)> = self
                .routes
                .iter()
                .filter(|(route, _)| Some(*route) != keep)
                .map(|(route, facts)| (facts.last_observed_ms(), route.clone()))
                .collect();
            oldest.sort_unstable_by_key(|(observed, _)| *observed);
            let excess = self.routes.len() - MAX_ROUTES;
            for (_, route) in oldest.into_iter().take(excess) {
                self.routes.remove(&route);
            }
        }
        if self.listings.len() > MAX_LISTINGS {
            let mut oldest: Vec<(u64, ModelEndpointKey)> = self
                .listings
                .iter()
                .map(|(endpoint, listing)| (listing.listed_at_ms, endpoint.clone()))
                .collect();
            oldest.sort_unstable_by_key(|(listed, _)| *listed);
            let excess = self.listings.len() - MAX_LISTINGS;
            for (_, endpoint) in oldest.into_iter().take(excess) {
                self.listings.remove(&endpoint);
            }
        }
    }
}

impl RouteFacts {
    /// Drops the facts that no longer hold at `now_ms`: learned ones after
    /// `LEARNED_TTL_MS`, reported ones `REPORTED_TTL_MS` after the route
    /// stopped being `listed`.
    fn expire(&mut self, listed: bool, now_ms: u64) {
        if !listed {
            expire(&mut self.reported, now_ms, REPORTED_TTL_MS);
        }
        expire(&mut self.learned, now_ms, LEARNED_TTL_MS);
    }

    fn normalized(self, now_ms: u64) -> Self {
        Self {
            reported: stamped(self.reported, ModelMetadataSource::Reported, now_ms),
            learned: stamped(self.learned, ModelMetadataSource::Learned, now_ms),
        }
    }

    fn is_empty(&self) -> bool {
        is_unknown(&self.reported) && is_unknown(&self.learned)
    }

    fn last_observed_ms(&self) -> u64 {
        [&self.reported, &self.learned]
            .into_iter()
            .flat_map(observed_times)
            .max()
            .unwrap_or(0)
    }
}

/// Whether the latest listing of `route`'s endpoint names it.
fn listed(listings: &HashMap<ModelEndpointKey, ListingState>, route: &ModelRouteKey) -> bool {
    listings
        .get(route.endpoint())
        .is_some_and(|listing| listing.models.contains(route.model()))
}

/// What `entry` reports, observed at `now_ms`.
fn reported_facts(entry: &ModelCatalogEntry, now_ms: u64) -> ModelMetadata {
    let reasoning = entry.reasoning.map(|reasons| {
        if reasons {
            ModelReasoningSupport::Supported {
                efforts: entry.supported_reasoning_efforts.clone(),
                default: entry.default_reasoning_effort.clone(),
            }
        } else {
            ModelReasoningSupport::Unsupported
        }
    });
    ModelMetadata {
        context_window: entry.context_window.map(|value| reported(value, now_ms)),
        max_output_tokens: entry.max_output_tokens.map(|value| reported(value, now_ms)),
        reasoning: reasoning.map(|value| reported(value, now_ms)),
        image_input: entry.image_input.map(|value| reported(value, now_ms)),
    }
}

fn reported<T>(value: T, now_ms: u64) -> Sourced<T> {
    Sourced {
        value,
        source: ModelMetadataSource::Reported,
        observed_at_ms: now_ms,
    }
}

pub(super) fn is_unknown(metadata: &ModelMetadata) -> bool {
    *metadata == ModelMetadata::default()
}

/// Drops the facts of `metadata` observed more than `ttl_ms` before
/// `now_ms`.
fn expire(metadata: &mut ModelMetadata, now_ms: u64, ttl_ms: u64) {
    fn expire_fact<T>(fact: &mut Option<Sourced<T>>, now_ms: u64, ttl_ms: u64) {
        fact.take_if(|fact| now_ms.saturating_sub(fact.observed_at_ms) > ttl_ms);
    }
    expire_fact(&mut metadata.context_window, now_ms, ttl_ms);
    expire_fact(&mut metadata.max_output_tokens, now_ms, ttl_ms);
    expire_fact(&mut metadata.reasoning, now_ms, ttl_ms);
    expire_fact(&mut metadata.image_input, now_ms, ttl_ms);
}

/// `metadata` with every fact's source `source`, and observed no later than
/// `now_ms`: a saved fact is never taken for another source's, and a clock
/// set back does not keep it past its expiry.
fn stamped(metadata: ModelMetadata, source: ModelMetadataSource, now_ms: u64) -> ModelMetadata {
    fn stamp<T>(
        fact: Option<Sourced<T>>,
        source: ModelMetadataSource,
        now_ms: u64,
    ) -> Option<Sourced<T>> {
        fact.map(|fact| Sourced {
            source,
            observed_at_ms: fact.observed_at_ms.min(now_ms),
            ..fact
        })
    }
    ModelMetadata {
        context_window: stamp(metadata.context_window, source, now_ms),
        max_output_tokens: stamp(metadata.max_output_tokens, source, now_ms),
        reasoning: stamp(metadata.reasoning, source, now_ms),
        image_input: stamp(metadata.image_input, source, now_ms),
    }
}

fn observed_times(metadata: &ModelMetadata) -> impl Iterator<Item = u64> {
    [
        metadata
            .context_window
            .as_ref()
            .map(|fact| fact.observed_at_ms),
        metadata
            .max_output_tokens
            .as_ref()
            .map(|fact| fact.observed_at_ms),
        metadata.reasoning.as_ref().map(|fact| fact.observed_at_ms),
        metadata
            .image_input
            .as_ref()
            .map(|fact| fact.observed_at_ms),
    ]
    .into_iter()
    .flatten()
}
