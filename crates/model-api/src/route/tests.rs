use super::*;

fn key(protocol: ModelApiProtocol, endpoint: &str, model: &str) -> ModelRouteKey {
    ModelRouteKey::new(protocol, &ModelEndpoint::parse(endpoint).unwrap(), model)
}

#[test]
fn a_key_never_exposes_the_endpoint_or_its_query() {
    let key = key(
        ModelApiProtocol::Responses,
        "https://example.test/v1?api_key=route-secret",
        "fixture-model",
    );
    let debug = format!("{key:?}");
    let persisted = serde_json::to_string(&key).unwrap();
    for text in [&debug, &persisted] {
        assert!(text.contains("fixture-model"), "{text}");
        assert!(!text.contains("example.test"), "{text}");
        assert!(!text.contains("route-secret"), "{text}");
        assert!(!text.contains("api_key"), "{text}");
    }
    assert!(debug.contains("Responses"));
    assert_eq!(
        serde_json::from_str::<ModelRouteKey>(&persisted).unwrap(),
        key
    );
}

#[test]
fn a_route_is_its_protocol_canonical_endpoint_and_trimmed_model() {
    let route = key(ModelApiProtocol::Responses, "https://one.test/v1", "model");
    assert_eq!(
        key(
            ModelApiProtocol::Responses,
            "https://one.test/v1/",
            " model "
        ),
        route
    );
    assert_eq!(route.model(), "model");
    for other in [
        key(
            ModelApiProtocol::ChatCompletions,
            "https://one.test/v1",
            "model",
        ),
        key(ModelApiProtocol::Responses, "https://two.test/v1", "model"),
        key(
            ModelApiProtocol::Responses,
            "https://one.test/v1?a=b",
            "model",
        ),
        key(ModelApiProtocol::Responses, "https://one.test/v1", "other"),
    ] {
        assert_ne!(other, route);
    }
    assert_eq!(
        key(ModelApiProtocol::Responses, "https://one.test/v1", "other").endpoint(),
        route.endpoint()
    );
}
