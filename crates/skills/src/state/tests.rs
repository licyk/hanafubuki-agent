use super::*;

fn id(value: &str) -> SkillId {
    SkillId::from_untrusted(value).unwrap()
}

#[test]
fn a_missing_state_file_means_everything_is_on() {
    let temporary = tempfile::tempdir().unwrap();
    let state = load_state(&temporary.path().join("skills.json")).unwrap();
    assert!(state.disabled.is_empty());
    assert!(!state.is_disabled(&id("user:demo")));
}

#[test]
fn turning_a_skill_off_and_on_round_trips() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("skills.json");
    let mut state = load_state(&path).unwrap();

    assert!(state.set_disabled(&id("user:demo"), true));
    assert!(!state.set_disabled(&id("user:demo"), true));
    bump_revision(&mut state);
    save_state(&path, &state).unwrap();

    let reloaded = load_state(&path).unwrap();
    assert!(reloaded.is_disabled(&id("user:demo")));
    assert_eq!(reloaded.revision, 2);

    let mut reloaded = reloaded;
    assert!(reloaded.set_disabled(&id("user:demo"), false));
    save_state(&path, &reloaded).unwrap();
    assert!(!load_state(&path).unwrap().is_disabled(&id("user:demo")));
}

#[test]
fn corrupt_state_is_quarantined_rather_than_fatal() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("skills.json");
    std::fs::write(&path, "{not json").unwrap();
    assert!(load_state(&path).unwrap().disabled.is_empty());
    assert!(path.exists());
}

#[test]
fn an_id_this_build_cannot_parse_is_dropped_not_fatal() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("skills.json");
    std::fs::write(
        &path,
        r#"{"version":3,"revision":5,"disabled":["user:demo","nonsense","user:other"]}"#,
    )
    .unwrap();
    let state = load_state(&path).unwrap();
    assert_eq!(
        state
            .disabled
            .iter()
            .map(SkillId::as_str)
            .collect::<Vec<_>>(),
        vec!["user:demo", "user:other"]
    );
    assert_eq!(state.revision, 5);
}

#[test]
fn the_catalog_revision_follows_the_selection_and_the_entries() {
    let entry = |enabled: bool| SkillCatalogEntry {
        id: id("user:demo"),
        scope: crate::model::SkillScope::User,
        name: "demo".to_string(),
        description: "d".to_string(),
        short_description: None,
        license: None,
        version: None,
        allow_implicit_invocation: true,
        interface: crate::model::SkillInterface::default(),
        dependencies: Vec::new(),
        directory: "/tmp/demo".to_string(),
        editable: true,
        enabled,
        availability: crate::model::SkillAvailability::default(),
        resources: Vec::new(),
    };
    let on = catalog_revision(&[entry(true)], 1);
    assert_ne!(on, catalog_revision(&[entry(false)], 1));
    assert_ne!(on, catalog_revision(&[entry(true)], 2));
    assert_eq!(on, catalog_revision(&[entry(true)], 1));
}
