use super::*;
use hanafubuki_agent_model_api::{ModelApiProtocol, ModelEndpoint};

fn key(protocol: ModelApiProtocol, endpoint: &str, model: &str) -> ModelRouteKey {
    ModelRouteKey::new(protocol, &ModelEndpoint::parse(endpoint).unwrap(), model)
}

#[test]
fn cache_is_route_scoped_bounded_and_expires_without_sleeping() {
    let mut cache = ModelImageCapabilityCache::default();
    let target = key(ModelApiProtocol::Responses, "https://one.test/v1", "model");
    cache.record_unsupported(target.clone(), 10);
    assert_eq!(
        cache.capability(&target, 10),
        ModelImageInputCapability::Unsupported
    );
    assert_eq!(
        cache.capability(
            &key(
                ModelApiProtocol::ChatCompletions,
                "https://one.test/v1",
                "model"
            ),
            10
        ),
        ModelImageInputCapability::Unknown
    );
    assert_eq!(
        cache.capability(
            &key(ModelApiProtocol::Responses, "https://two.test/v1", "model"),
            10
        ),
        ModelImageInputCapability::Unknown
    );
    assert_eq!(
        cache.capability(
            &key(ModelApiProtocol::Responses, "https://one.test/v1", "other"),
            10
        ),
        ModelImageInputCapability::Unknown
    );

    for index in 0..=MAXIMUM_IMAGE_CAPABILITY_ENTRIES {
        cache.record_unsupported(
            key(
                ModelApiProtocol::Responses,
                "https://bounded.test/v1",
                &format!("model-{index}"),
            ),
            20,
        );
    }
    assert_eq!(cache.entries.len(), MAXIMUM_IMAGE_CAPABILITY_ENTRIES);
    assert_eq!(
        cache.capability(&target, 20),
        ModelImageInputCapability::Unknown,
        "oldest route should be evicted"
    );

    let expiring = key(ModelApiProtocol::Responses, "https://ttl.test/v1", "model");
    cache.record_unsupported(expiring.clone(), 100);
    assert_eq!(
        cache.capability(&expiring, 100 + IMAGE_CAPABILITY_TTL_MILLIS + 1),
        ModelImageInputCapability::Unknown
    );
}
