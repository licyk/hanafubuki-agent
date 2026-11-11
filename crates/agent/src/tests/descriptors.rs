//! Every row of the plugin host has exactly one descriptor, which its
//! status shows; a host's row without one keeps the Agent from being built.

use super::host::{
    agent_plugin_descriptors, AgentPluginDescriptor, AgentPluginGroup, AgentPluginRow,
};

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn every_row_has_exactly_one_descriptor_and_its_status_shows_it() {
    let (service, paths, _) = service_with_config("plugin-descriptors", |_, _| {});
    let statuses = service.plugin_status();
    let rows = statuses
        .iter()
        .map(|status| status.name.as_str())
        .collect::<Vec<_>>();
    let described = agent_plugin_descriptors()
        .iter()
        .map(|descriptor| descriptor.name)
        .collect::<Vec<_>>();
    assert_eq!(rows, described);
    for status in &statuses {
        let descriptor = agent_plugin_descriptors()
            .into_iter()
            .find(|descriptor| descriptor.name == status.name)
            .unwrap();
        assert_eq!(status.summary, descriptor.summary);
        assert_eq!(
            status.options.as_ref().map(|options| &options.schema),
            descriptor.options.as_ref(),
            "{}",
            status.name
        );
    }
    let _ = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hosts_row_needs_its_own_descriptor() {
    let paths = test_paths("plugin-descriptor-missing");
    let row = AgentPluginRow::unconfigured(
        AgentPluginGroup::Capability,
        kardia_core::PluginDef::new(|_, _| async { Ok(()) }).name("host-ui"),
    );
    let refused = AgentApplicationService::new_with_test_plugins(
        &paths,
        AgentConfig::new(&paths.config_dir),
        AgentStoragePaths::under(paths.data_dir.join("agent-test")),
        SkillRoots {
            system: paths.data_dir.join("agent-test/skills-bundled"),
            user: paths.data_dir.join("agent-test/skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("agent-test/skills-state.json"),
        },
        vec![(row, AgentPluginDescriptor::new("host-other", "another row"))],
    )
    .err()
    .unwrap();
    assert!(refused.contains("`host-ui` has no descriptor"), "{refused}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
