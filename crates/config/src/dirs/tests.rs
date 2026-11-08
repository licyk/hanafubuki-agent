use super::*;

#[test]
fn the_home_holds_both_directories() {
    let home = std::env::temp_dir().join("hanafubuki-agent-home");
    assert_eq!(
        AgentDirs::resolve(Some(home.clone())).unwrap(),
        AgentDirs {
            config_dir: home.clone(),
            data_dir: home,
        }
    );
    let relative = AgentDirs::resolve(Some(PathBuf::from("agent-home"))).unwrap();
    assert!(relative.config_dir.is_absolute());
    assert!(relative.config_dir.ends_with("agent-home"));
}

#[test]
fn without_a_home_the_operating_systems_directories_are_used() {
    let dirs = AgentDirs::resolve(None).unwrap();
    assert_eq!(
        dirs.config_dir,
        dirs::config_dir().unwrap().join("hanafubuki-agent")
    );
    assert_eq!(
        dirs.data_dir,
        dirs::data_dir().unwrap().join("hanafubuki-agent")
    );
}
