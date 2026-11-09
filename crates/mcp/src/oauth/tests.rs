use super::*;
use crate::test_support::oauth::{oauth_config, start_oauth_fixture};

#[test]
fn callback_must_match_the_exact_active_redirect() {
    let redirect = validate_redirect_uri("http://127.0.0.1:43123/mcp/callback").unwrap();
    validate_callback(
        &redirect,
        "http://127.0.0.1:43123/mcp/callback?code=one&state=two",
    )
    .unwrap();
    for callback in [
        "http://127.0.0.1:43124/mcp/callback?code=one&state=two",
        "http://localhost:43123/mcp/callback?code=one&state=two",
        "http://127.0.0.1:43123/other?code=one&state=two",
        "http://user@127.0.0.1:43123/mcp/callback?code=one&state=two",
        "http://127.0.0.1:43123/mcp/callback?code=one&state=two#fragment",
    ] {
        assert!(
            validate_callback(&redirect, callback).is_err(),
            "{callback}"
        );
    }
}

#[test]
fn oauth_credential_key_is_stable_and_does_not_embed_the_server_id() {
    let server_id = McpServerId::parse("private-server-name").unwrap();
    let first = oauth_credential_key(&server_id).unwrap();
    let second = oauth_credential_key(&server_id).unwrap();
    assert_eq!(first, second);
    assert!(first.as_str().starts_with("oauth."));
    assert!(!first.as_str().contains(server_id.as_str()));
}

#[test]
fn redirect_rejects_credentials_query_fragment_and_non_http_schemes() {
    for redirect in [
        "file:///tmp/callback",
        "http://user@localhost/callback",
        "http://localhost/callback?query=1",
        "http://localhost/callback#fragment",
    ] {
        assert!(validate_redirect_uri(redirect).is_err(), "{redirect}");
    }
}

#[tokio::test]
async fn oauth_fixture_covers_discovery_pkce_exchange_refresh_and_logout() {
    let (base, server) = start_oauth_fixture().await;
    let config = oauth_config(&base);
    let store = Arc::new(crate::MemoryMcpSecretStore::default());
    let manager = McpOAuthManager::new(
        store.clone(),
        crate::build_mcp_http_client(crate::McpHttpProxyPolicy::Disabled).unwrap(),
    );
    let redirect = "http://127.0.0.1:53682/mcp/oauth/callback";

    let login = manager
        .start_login(&config, redirect.to_string())
        .await
        .unwrap();
    let authorization_url = url::Url::parse(&login.authorization_url).unwrap();
    let query = authorization_url
        .query_pairs()
        .into_owned()
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        query.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert!(query
        .get("code_challenge")
        .is_some_and(|value| !value.is_empty()));
    let state = query.get("state").unwrap();

    let mut wrong_callback = url::Url::parse(redirect).unwrap();
    wrong_callback
        .query_pairs_mut()
        .append_pair("code", "fixture-code")
        .append_pair("state", state)
        .append_pair("iss", "https://attacker.invalid");
    assert!(manager
        .complete_login(&config, login.flow_id, wrong_callback.into())
        .await
        .is_err());

    let login = manager
        .start_login(&config, redirect.to_string())
        .await
        .unwrap();
    let authorization_url = url::Url::parse(&login.authorization_url).unwrap();
    let query = authorization_url
        .query_pairs()
        .into_owned()
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut callback = url::Url::parse(redirect).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("code", "fixture-code")
        .append_pair("state", query.get("state").unwrap())
        .append_pair("iss", &base);
    let status = manager
        .complete_login(&config, login.flow_id, callback.into())
        .await
        .unwrap();
    assert_eq!(status.phase, McpOAuthPhase::Authorized);
    assert_eq!(
        manager.access_token(&config).await.unwrap().as_deref(),
        Some("access-from-code")
    );

    let refreshed = manager.refresh(&config).await.unwrap();
    assert_eq!(refreshed.phase, McpOAuthPhase::Authorized);
    assert_eq!(
        manager.access_token(&config).await.unwrap().as_deref(),
        Some("access-from-refresh")
    );
    let key = oauth_credential_key(&config.id).unwrap();
    let before_failed_refresh = store.get(&key).await.unwrap().unwrap();
    server.abort();
    let _ = server.await;
    assert!(manager.refresh(&config).await.is_err());
    assert_eq!(
        store.get(&key).await.unwrap().as_deref(),
        Some(before_failed_refresh.as_slice())
    );
    assert_eq!(
        manager.logout(&config).await.unwrap().phase,
        McpOAuthPhase::LoggedOut
    );
    assert!(manager.access_token(&config).await.is_err());
}

#[tokio::test]
async fn oauth_flow_identity_prevents_callback_mixup_and_cancelled_flow_reuse() {
    let (base, server) = start_oauth_fixture().await;
    let mut first = oauth_config(&base);
    first.id = McpServerId::parse("oauth-first").unwrap();
    let mut second = oauth_config(&base);
    second.id = McpServerId::parse("oauth-second").unwrap();
    let store = Arc::new(crate::MemoryMcpSecretStore::default());
    let manager = McpOAuthManager::new(
        store,
        crate::build_mcp_http_client(crate::McpHttpProxyPolicy::Disabled).unwrap(),
    );
    let redirect = "http://127.0.0.1:53682/mcp/oauth/callback";
    let first_login = manager
        .start_login(&first, redirect.to_string())
        .await
        .unwrap();
    let second_login = manager
        .start_login(&second, redirect.to_string())
        .await
        .unwrap();
    let second_authorization = url::Url::parse(&second_login.authorization_url).unwrap();
    let second_state = second_authorization
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
        .unwrap();
    let mut second_callback = url::Url::parse(redirect).unwrap();
    second_callback
        .query_pairs_mut()
        .append_pair("code", "second-code")
        .append_pair("state", &second_state)
        .append_pair("iss", &base);

    assert_eq!(
        manager
            .complete_login(&first, second_login.flow_id, second_callback.to_string())
            .await
            .unwrap_err(),
        McpError::StaleGeneration
    );
    assert_eq!(
        manager
            .cancel_login(&second, second_login.flow_id)
            .unwrap()
            .phase,
        McpOAuthPhase::LoggedOut
    );
    assert_eq!(
        manager
            .complete_login(&second, second_login.flow_id, second_callback.to_string())
            .await
            .unwrap_err(),
        McpError::StaleGeneration
    );
    assert_eq!(
        manager
            .cancel_login(&first, first_login.flow_id)
            .unwrap()
            .phase,
        McpOAuthPhase::LoggedOut
    );

    server.abort();
    let _ = server.await;
}
