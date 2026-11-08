use super::*;

#[test]
fn an_id_is_a_scope_and_a_directory_path() {
    let id = SkillId::new(SkillScope::User, "pdf-tools").unwrap();
    assert_eq!(id.as_str(), "user:pdf-tools");
    assert_eq!(id.scope(), SkillScope::User);
    assert_eq!(id.relative_path(), "pdf-tools");
    assert_eq!(id.folder_name(), "pdf-tools");

    let nested = SkillId::new(SkillScope::Repo, "web/lint").unwrap();
    assert_eq!(nested.relative_path(), "web/lint");
    assert_eq!(nested.folder_name(), "lint");
}

#[test]
fn an_id_accepts_the_folder_names_real_packages_use() {
    // Mixed case, digits, dots, and underscores all appear in published Skills;
    // the old lowercase-and-hyphens rule rejected packages that install fine
    // everywhere else.
    for name in ["PDF_Tools", "skill.v2", "Skill-Creator", "日本語"] {
        SkillId::new(SkillScope::User, name)
            .unwrap_or_else(|error| panic!("{name} should be a valid folder: {error}"));
    }
}

#[test]
fn an_id_refuses_traversal_and_hidden_directories() {
    for relative in ["..", "a/../b", ".hidden", "a//b", "", "a\\b", "a:b"] {
        assert!(
            SkillId::new(SkillScope::User, relative).is_err(),
            "{relative} should be refused"
        );
    }
    assert!(SkillId::from_untrusted("nowhere:thing").is_err());
    assert!(SkillId::from_untrusted("user").is_err());
}

#[test]
fn an_entry_prefers_its_interface_name_and_shortest_description() {
    let mut entry = SkillCatalogEntry {
        id: SkillId::new(SkillScope::User, "demo").unwrap(),
        scope: SkillScope::User,
        name: "demo".to_string(),
        description: "The long description.".to_string(),
        short_description: None,
        license: None,
        version: None,
        allow_implicit_invocation: true,
        interface: SkillInterface::default(),
        dependencies: Vec::new(),
        directory: "/tmp/demo".to_string(),
        editable: true,
        enabled: true,
        availability: SkillAvailability::default(),
        resources: Vec::new(),
    };
    assert_eq!(entry.display_name(), "demo");
    assert_eq!(entry.summary(), "The long description.");

    entry.short_description = Some("Short.".to_string());
    assert_eq!(entry.summary(), "Short.");

    entry.interface.display_name = Some("Demo Skill".to_string());
    entry.interface.short_description = Some("Shorter.".to_string());
    assert_eq!(entry.display_name(), "Demo Skill");
    assert_eq!(entry.summary(), "Shorter.");
}
