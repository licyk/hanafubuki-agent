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

use super::*;
use crate::EngineConfig;
#[test]
fn quota_survives_restart_without_secret_material_but_auth_does_not() {
    let directory = std::env::temp_dir().join(format!("hana-cooldown-{}", uuid::Uuid::new_v4()));
    let mut config = SearchOptions::default();
    config.engines.insert(
        E::Exa,
        EngineConfig {
            api_keys: vec!["private-key-alpha".into(), "private-key-beta".into()],
            ..Default::default()
        },
    );
    let runtime = EngineRuntime::new(&config, Some(&directory), &crate::WebSearchProxy::Disabled);
    runtime.record(
        E::Exa,
        0,
        Duration::ZERO,
        &Err(WebSearchError::ProviderBackoff {
            class: FailureClass::Quota,
            delay: Duration::from_secs(90),
        }),
    );
    runtime.record(
        E::Exa,
        1,
        Duration::ZERO,
        &Err(WebSearchError::HttpStatus(401)),
    );
    assert!(matches!(
        runtime.attempts(E::Exa),
        Err(WebSearchError::CoolingDown)
    ));
    drop(runtime);
    let runtime = EngineRuntime::new(&config, Some(&directory), &crate::WebSearchProxy::Disabled);
    let keys = runtime.attempts(E::Exa).unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].1, "private-key-beta");
    drop(runtime);
    let bytes = std::fs::read(directory.join("cooldowns.db")).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("private-key"));
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn serp_bot_wall_pauses_siblings_and_anysearch_rejected_key_can_use_anonymous() {
    let mut config = SearchOptions::default();
    config.engines.insert(
        E::Anysearch,
        EngineConfig {
            api_keys: vec!["bad-key".into()],
            ..Default::default()
        },
    );
    let runtime = EngineRuntime::new(&config, None, &crate::WebSearchProxy::Disabled);
    runtime.record(
        E::So360,
        0,
        Duration::ZERO,
        &Err(WebSearchError::ProviderFailure(FailureClass::BotWall)),
    );
    assert!(matches!(
        runtime.attempts(E::Sogou),
        Err(WebSearchError::CoolingDown)
    ));
    runtime.record(
        E::Anysearch,
        0,
        Duration::ZERO,
        &Err(WebSearchError::HttpStatus(401)),
    );
    assert_eq!(
        runtime.attempts(E::Anysearch).unwrap(),
        vec![(usize::MAX, String::new())]
    );
}
#[test]
fn upstream_reset_hints_are_bounded_and_region_is_distinct() {
    assert_eq!(
        super::super::backoff::delay("quota resets in 2 hours"),
        Some(Duration::from_secs(7200))
    );
    assert_eq!(
        super::super::backoff::delay("retry-after: 1 ms"),
        Some(Duration::from_secs(5))
    );
    assert_eq!(
        super::super::backoff::delay("monthly limit"),
        Some(Duration::from_secs(86400))
    );
    assert_eq!(
        super::super::backoff::classify(403, "not currently available in your region").0,
        FailureClass::Region
    );
}
