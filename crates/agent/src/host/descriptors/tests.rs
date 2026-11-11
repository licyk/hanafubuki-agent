use super::*;

#[test]
fn a_row_without_a_descriptor_and_a_name_described_twice_are_refused() {
    let descriptors = AgentPluginDescriptors::new(vec![AgentPluginDescriptor::new(
        "host-ui",
        "the host's UI context",
    )])
    .unwrap();
    assert!(descriptors
        .check_rows(["web", "host-ui"].into_iter())
        .is_ok());
    let missing = descriptors
        .check_rows(["web", "host-other"].into_iter())
        .unwrap_err();
    assert!(
        missing.contains("`host-other` has no descriptor"),
        "{missing}"
    );

    let twice = AgentPluginDescriptors::new(vec![AgentPluginDescriptor::new("web", "again")])
        .err()
        .unwrap();
    assert!(twice.contains("`web` has two descriptors"), "{twice}");
}

#[test]
fn every_option_of_a_descriptor_has_a_default_its_schema_accepts() {
    for descriptor in agent_plugin_descriptors() {
        assert!(!descriptor.summary.is_empty(), "{}", descriptor.name);
        for option in descriptor.options.iter().flat_map(|schema| &schema.options) {
            let default = option
                .default
                .as_ref()
                .expect("a row's option has a default");
            assert_eq!(
                option.check(&default.to_json()).as_ref(),
                Ok(default),
                "{}.{}",
                descriptor.name,
                option.key
            );
        }
    }
}
