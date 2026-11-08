use super::AgentStoragePaths;

#[test]
fn derives_every_durable_agent_store_from_one_root() {
    let root = std::path::PathBuf::from("/application-data/data/agent");
    let paths = AgentStoragePaths::under(root.clone());

    assert_eq!(paths.root, root);
    assert_eq!(paths.history_root, root);
    assert_eq!(paths.memory_root, root.join("memory"));
    assert_eq!(paths.skills_root, root.join("skills"));
    assert_eq!(paths.skill_state_file, root.join("skills/state.json"));
    assert_eq!(paths.start_marker_file, root.join("start-marker.json"));
    assert_eq!(paths.crash_record_file, root.join("crash-record.json"));
    assert_eq!(paths.process_groups_file, root.join("process-groups.json"));
}
