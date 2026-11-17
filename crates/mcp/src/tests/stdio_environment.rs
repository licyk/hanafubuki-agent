use super::*;

#[tokio::test]
async fn stdio_inherits_only_selected_names_and_keeps_explicit_overlays() {
    const MARKER: &str = "HANAFUBUKI_MCP_ENV_TEST";
    if std::env::var_os(MARKER).is_none() {
        let temp = tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::stdio_environment::stdio_inherits_only_selected_names_and_keeps_explicit_overlays", "--nocapture"])
            .env(MARKER, temp.path())
            .env("HB_ENV_ALLOWED", "inherited")
            .env("HB_ENV_PASSTHROUGH", "selected")
            .env("HB_ENV_BLOCKED", "must-not-reach-server")
            .env("HB_ENV_SECRET", "old-secret")
            .env("HTTP_PROXY", "http://parent.invalid:8080")
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let report =
        std::path::PathBuf::from(std::env::var_os(MARKER).unwrap()).join("environment.json");
    let fixture = r#"
import json, os, sys
with open(sys.argv[1], 'w', encoding='utf-8') as f:
    json.dump({k: os.environ.get(k) for k in ['HB_ENV_PASSTHROUGH', 'HB_ENV_ALLOWED', 'HB_ENV_BLOCKED', 'HB_ENV_SECRET', 'HTTP_PROXY', 'PATH']}, f)
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request: continue
    if request.get('method') == 'initialize':
        result = {'protocolVersion': request['params']['protocolVersion'], 'capabilities': {}, 'serverInfo': {'name': 'env-test', 'version': '1'}}
    else: result = {}
    print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}), flush=True)
"#;
    let mut config = stdio_config();
    let McpTransportConfig::Stdio(stdio) = &mut config.transport else {
        unreachable!()
    };
    stdio.program = python_program().into();
    stdio.argv = vec![
        "-u".into(),
        "-c".into(),
        fixture.into(),
        report.to_string_lossy().into_owned(),
    ];
    stdio.environment_variables = vec![
        "HB_ENV_PASSTHROUGH".into(),
        "HB_ENV_ALLOWED".into(),
        "HB_ENV_SECRET".into(),
    ];
    stdio.public_environment = BTreeMap::from([("HB_ENV_ALLOWED".into(), "configured".into())]);
    stdio.credential_environment.clear();
    let overlay = hanafubuki_agent_foundation::environment::ChildProcessEnvironment::default();
    overlay
        .replace(std::collections::HashMap::from([(
            "HTTP_PROXY".into(),
            Some("http://configured.invalid:9000".into()),
        )]))
        .unwrap();
    let executor = Arc::new(ExecutorManager::new(overlay));
    let workers = BackgroundWorkerGroup::default();
    let connection = McpStdioConnection::connect(
        &config,
        McpResolvedSecretEnvironment::new(BTreeMap::from([(
            "HB_ENV_SECRET".into(),
            "resolved-secret".into(),
        )]))
        .unwrap(),
        1,
        executor.clone(),
        workers.clone(),
        McpInteractionHub::default(),
        CancellationToken::default(),
    )
    .await
    .unwrap();
    let env: serde_json::Value = serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
    assert_eq!(env["HB_ENV_PASSTHROUGH"], "selected");
    assert_eq!(env["HB_ENV_ALLOWED"], "configured");
    assert_eq!(env["HB_ENV_SECRET"], "resolved-secret");
    assert!(env["HB_ENV_BLOCKED"].is_null());
    assert_eq!(env["HTTP_PROXY"], "http://configured.invalid:9000");
    assert!(env["PATH"].as_str().is_some_and(|value| !value.is_empty()));
    connection.shutdown(Duration::from_secs(5)).await.unwrap();
    assert_eq!(executor_registry_counts(&executor), (0, 0, 0));
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}
