use super::*;

#[test]
fn the_bundled_skills_are_published_and_repaired() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("system");

    synchronize_system_skills(&root).unwrap();
    assert!(matches_embedded(&root).unwrap());
    assert!(root.join("skill-creator/SKILL.md").is_file());

    std::fs::write(root.join("skill-manager/SKILL.md"), "tampered").unwrap();
    assert!(!matches_embedded(&root).unwrap());
    synchronize_system_skills(&root).unwrap();
    assert!(matches_embedded(&root).unwrap());
}

#[test]
fn every_bundled_skill_parses() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("system");
    synchronize_system_skills(&root).unwrap();

    let scan = crate::discovery::scan_root(&root, crate::model::SkillScope::System, None);
    assert!(scan.issues.is_empty(), "{:?}", scan.issues);
    assert_eq!(
        scan.skills
            .iter()
            .map(|skill| skill.entry.id.as_str().to_string())
            .collect::<Vec<_>>(),
        vec![
            "system:skill-creator".to_string(),
            "system:skill-manager".to_string()
        ]
    );
}
