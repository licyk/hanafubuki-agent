//! The model Providers' requests go through the host's proxy, and a change
//! of it reaches a session's next turn: the request reaches the proxy
//! naming the Provider's whole URL, at a host that does not resolve.

use hanafubuki_agent_kernel::AgentSessionMode;
use hanafubuki_agent_process::executor::ExecutorManager;
use parking_lot::Mutex;

use super::*;

const PROVIDER: &str = "http://provider.invalid/v1";

/// A host whose proxy changes when the test says.
struct SwitchedProxy(Mutex<AgentProxy>);

impl NetworkSettingsPort for SwitchedProxy {
    fn proxy(&self) -> Result<AgentProxy, String> {
        Ok(self.0.lock().clone())
    }
}

/// `server`'s origin, as a proxy URL.
fn origin(server: &FakeServer) -> AgentProxy {
    AgentProxy::Explicit(
        server
            .base_url
            .splitn(4, '/')
            .take(3)
            .collect::<Vec<_>>()
            .join("/"),
    )
}

#[tokio::test]
async fn a_turn_goes_through_the_hosts_proxy_and_a_change_reaches_the_next_turn() {
    let first = FakeServer::start(vec![(assistant_events("First."), Duration::ZERO)]).await;
    let second = FakeServer::start(vec![(assistant_events("Second."), Duration::ZERO)]).await;
    let paths = test_paths("network-proxy");
    let config = AgentConfig::new(&paths.config_dir);
    configure_test_provider(&config, PROVIDER, None);
    let network = Arc::new(SwitchedProxy(Mutex::new(origin(&first))));
    let mut host = test_host(&paths);
    host.network = network.clone();
    let service = AgentApplicationService::new_with_host(
        config,
        host,
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        ExecutorManager::default(),
    )
    .unwrap();
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();

    service
        .start_turn(session.id, "first".into())
        .await
        .unwrap();
    *network.0.lock() = origin(&second);
    service.apply_settings_to_plugins();
    service
        .start_turn(session.id, "second".into())
        .await
        .unwrap();

    for server in [&first, &second] {
        let [request] = server.requests().try_into().ok().unwrap();
        assert!(
            request.path.starts_with(PROVIDER),
            "{} did not reach the proxy",
            request.path
        );
    }
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
