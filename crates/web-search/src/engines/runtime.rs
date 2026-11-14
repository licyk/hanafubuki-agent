// Ported from dsh-free-search 6ab5168 (MIT); adapted to hanafubuki-agent.
//
// The MIT License (MIT)
//
// Copyright (c) 2026 dsh-free-search contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

// Derived from dsh-free-search 6ab5168 (MIT), engine_runtime.js and key rotation.

use parking_lot::Mutex;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

use super::{FailureClass, SearchOptions, WebSearchProvider as E};
use crate::WebSearchError;

pub(crate) struct EngineRuntime {
    pub(crate) gates: BTreeMap<E, Arc<Semaphore>>,
    state: Mutex<BTreeMap<E, State>>,
    store: super::store::CooldownStore,
}

struct State {
    keys: Vec<String>,
    next_key: usize,
    cooldowns: BTreeMap<usize, Cooldown>,
    engine_cooldown: Option<Cooldown>,
    stats: EngineStatistics,
}

#[derive(Clone)]
struct Cooldown {
    until: Option<Instant>,
    class: FailureClass,
}
impl Cooldown {
    fn active(&self) -> bool {
        self.until.is_none_or(|end| end > Instant::now())
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatistics {
    pub calls: u64,
    pub successes: u64,
    pub empty: u64,
    pub failures: u64,
    pub total_milliseconds: u64,
    pub last_failure: Option<FailureClass>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineDiagnostic {
    pub engine: E,
    pub enabled: bool,
    pub credentials_available: bool,
    pub cooling_down: bool,
    pub cooling_keys: usize,
    pub cooldown_reason: Option<FailureClass>,
    pub statistics: EngineStatistics,
}

impl EngineRuntime {
    pub(crate) fn new(
        config: &SearchOptions,
        directory: Option<&std::path::Path>,
        proxy: &crate::WebSearchProxy,
    ) -> Self {
        let store = super::store::CooldownStore::new(directory, proxy);
        let state = E::ALL
            .iter()
            .copied()
            .map(|engine| {
                let keys = config
                    .engines
                    .get(&engine)
                    .cloned()
                    .unwrap_or_default()
                    .keys(engine);
                let cooldowns = keys
                    .iter()
                    .enumerate()
                    .filter_map(|(i, key)| {
                        store.get(engine, key).map(|(delay, class)| {
                            (
                                i,
                                Cooldown {
                                    until: Some(Instant::now() + delay),
                                    class,
                                },
                            )
                        })
                    })
                    .collect();
                let engine_cooldown = store.get(engine, "").map(|(delay, class)| Cooldown {
                    until: Some(Instant::now() + delay),
                    class,
                });
                (
                    engine,
                    State {
                        keys,
                        next_key: 0,
                        cooldowns,
                        engine_cooldown,
                        stats: EngineStatistics::default(),
                    },
                )
            })
            .collect();
        Self {
            gates: E::ALL
                .iter()
                .copied()
                .map(|e| (e, Arc::new(Semaphore::new(if e.serp() { 1 } else { 2 }))))
                .collect(),
            state: Mutex::new(state),
            store,
        }
    }

    pub(crate) fn attempts(&self, engine: E) -> Result<Vec<(usize, String)>, WebSearchError> {
        let mut states = self.state.lock();
        let state = states
            .get_mut(&engine)
            .ok_or(WebSearchError::InvalidConfiguration(
                "not a concrete search engine",
            ))?;
        if state.engine_cooldown.as_ref().is_some_and(Cooldown::active) {
            return Err(WebSearchError::CoolingDown);
        }
        if state.keys.is_empty() {
            return if engine.keyless() || engine == E::Agy {
                Ok(vec![(0, String::new())])
            } else {
                Err(WebSearchError::ProviderFailure(FailureClass::Auth))
            };
        }
        let len = state.keys.len();
        let mut keys = (0..len)
            .map(|i| (state.next_key + i) % len)
            .filter(|i| !state.cooldowns.get(i).is_some_and(Cooldown::active))
            .map(|i| (i, state.keys[i].clone()))
            .collect::<Vec<_>>();
        if let Some((index, _)) = keys.first() {
            state.next_key = (index + 1) % len;
        }
        if engine == E::Anysearch {
            keys.push((usize::MAX, String::new()));
        }
        if keys.is_empty() {
            Err(WebSearchError::CoolingDown)
        } else {
            Ok(keys)
        }
    }

    pub(crate) fn record(
        &self,
        engine: E,
        key: usize,
        elapsed: Duration,
        result: &Result<usize, WebSearchError>,
    ) {
        let mut states = self.state.lock();
        let Some(state) = states.get_mut(&engine) else {
            return;
        };
        state.stats.calls += 1;
        state.stats.total_milliseconds = state
            .stats
            .total_milliseconds
            .saturating_add(elapsed.as_millis().min(u64::MAX as u128) as u64);
        match result {
            Ok(0) => state.stats.empty += 1,
            Ok(_) => state.stats.successes += 1,
            Err(WebSearchError::Cancelled) => {}
            Err(error) => {
                state.stats.failures += 1;
                let class = error.class();
                state.stats.last_failure = Some(class);
                if key == usize::MAX {
                    return;
                }
                let until = match class {
                    FailureClass::Region => Some(Instant::now() + Duration::from_secs(86400)),
                    FailureClass::Auth => None,
                    FailureClass::Quota => Some(Instant::now() + Duration::from_secs(45 * 60)),
                    FailureClass::BotWall => Some(
                        Instant::now() + Duration::from_secs(if engine.serp() { 600 } else { 60 }),
                    ),
                    FailureClass::Transient if matches!(error, WebSearchError::HttpStatus(429)) => {
                        Some(Instant::now() + Duration::from_secs(60))
                    }
                    _ if matches!(error, WebSearchError::ProviderBackoff { .. }) => {
                        Some(Instant::now())
                    }
                    _ => return,
                };
                let until = if let WebSearchError::ProviderBackoff { delay, .. } = error {
                    Some(Instant::now() + *delay)
                } else {
                    until
                };
                let cooldown = Cooldown { until, class };
                if let Some(until) = until {
                    self.store.put(
                        engine,
                        if state.keys.is_empty()
                            || matches!(class, FailureClass::BotWall | FailureClass::Region)
                        {
                            ""
                        } else {
                            &state.keys[key]
                        },
                        until.saturating_duration_since(Instant::now()),
                        class,
                    );
                }
                if state.keys.is_empty()
                    || matches!(class, FailureClass::BotWall | FailureClass::Region)
                {
                    state.engine_cooldown = Some(cooldown);
                } else {
                    state.cooldowns.insert(key, cooldown);
                }
                if engine.serp() && class == FailureClass::BotWall {
                    for (sibling, state) in states.iter_mut().filter(|(e, _)| e.serp()) {
                        let delay = Duration::from_secs(600);
                        state.engine_cooldown = Some(Cooldown {
                            until: Some(Instant::now() + delay),
                            class,
                        });
                        self.store.put(*sibling, "", delay, class);
                    }
                }
            }
        }
    }

    pub(crate) fn diagnostics(&self, options: &SearchOptions) -> Vec<EngineDiagnostic> {
        let states = self.state.lock();
        E::ALL
            .iter()
            .filter_map(|engine| {
                let state = states.get(engine)?;
                Some(EngineDiagnostic {
                    engine: *engine,
                    enabled: options.enabled(*engine),
                    credentials_available: engine.keyless() || !state.keys.is_empty(),
                    cooling_down: state.engine_cooldown.as_ref().is_some_and(Cooldown::active),
                    cooling_keys: state
                        .cooldowns
                        .values()
                        .filter(|entry| entry.active())
                        .count(),
                    cooldown_reason: state
                        .engine_cooldown
                        .as_ref()
                        .filter(|entry| entry.active())
                        .map(|entry| entry.class),
                    statistics: state.stats.clone(),
                })
            })
            .collect()
    }
}

impl WebSearchError {
    pub(crate) fn class(&self) -> FailureClass {
        match self {
            Self::ProviderFailure(class) | Self::ProviderBackoff { class, .. } => *class,
            Self::InvalidConfiguration(_) | Self::HttpStatus(401 | 403) => FailureClass::Auth,
            Self::HttpStatus(402) => FailureClass::Quota,
            Self::HttpStatus(429 | 500..=599) | Self::Timeout | Self::RequestFailed => {
                FailureClass::Transient
            }
            Self::InvalidResponse | Self::ResponseTooLarge => FailureClass::InvalidResponse,
            _ => FailureClass::Unknown,
        }
    }
}

#[cfg(test)]
mod tests;
