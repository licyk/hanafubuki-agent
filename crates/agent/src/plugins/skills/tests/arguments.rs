use super::*;
use crate::plugins::skills::arguments::SkillCatalogAuthority;

#[test]
fn skills_read_takes_a_skill_id_and_a_package_relative_path() {
    let call = parse_call(
        "skills_read",
        json!({ "skill": "user:pdf-tools", "path": "references/api.md" }),
    )
    .expect("a catalog id and a listed path should parse");
    assert!(matches!(call, SkillCall::Read(_)));

    // A host path is not a Skill id, and a path may not escape the package.
    assert!(parse_call(
        "skills_read",
        json!({ "skill": "/mutable/catalog/path", "path": "SKILL.md" }),
    )
    .is_err());
    assert_eq!(
        parse_call(
            "skills_read",
            json!({ "skill": "user:pdf-tools", "path": "../secret" }),
        ),
        Err(ToolCallValidationError::InvalidArgument("path"))
    );
    assert_eq!(
        parse_call(
            "skills_read",
            json!({ "skill": "user:pdf-tools", "cursor": "not a cursor" }),
        ),
        Err(ToolCallValidationError::InvalidArgument("cursor"))
    );
}

#[test]
fn skills_list_accepts_optional_authority_and_cursor() {
    assert!(matches!(
        parse_call("skills_list", json!({"authority":"repo"})).unwrap(),
        SkillCall::List(list) if list.authority == Some(SkillCatalogAuthority::Repo)
    ));
    assert!(parse_call("skills_list", json!({})).is_ok());
    assert!(parse_call("skills_list", json!({"authority":"admin"})).is_err());
    assert_eq!(
        parse_call("skills_list", json!([])),
        Err(ToolCallValidationError::ArgumentsMustBeObject)
    );
}

#[test]
fn turning_a_skill_on_or_off_names_only_the_skill() {
    assert!(matches!(
        parse_call("enable_skill", json!({ "skill": "user:pdf-tools" })).unwrap(),
        SkillCall::Enable(_)
    ));
    assert!(matches!(
        parse_call("disable_skill", json!({ "skill": "user:pdf-tools" })).unwrap(),
        SkillCall::Disable(_)
    ));
    // The catalog resolves the id when the call runs, so a revision the model
    // would have had to guess is not part of the request: it is ignored.
    assert_eq!(
        parse_call(
            "enable_skill",
            json!({ "skill": "user:pdf-tools", "expectedCatalogRevision": 4 })
        )
        .unwrap(),
        parse_call("enable_skill", json!({ "skill": "user:pdf-tools" })).unwrap()
    );
    // A Skill is named by its catalog id and nothing else: a directory is
    // no argument and changes nothing.
    assert_eq!(
        parse_call(
            "enable_skill",
            json!({ "skill": "user:example-skill", "directory": "/etc" })
        )
        .unwrap(),
        parse_call("enable_skill", json!({ "skill": "user:example-skill" })).unwrap()
    );
    // A field it cannot use is named.
    assert_eq!(
        parse_call("enable_skill", json!({ "skill": 7 }))
            .unwrap_err()
            .to_string(),
        "invalid tool argument skill: invalid type: integer `7`, expected a string"
    );
}

#[test]
fn plugin_catalog_ids_are_accepted_by_every_selection_tool() {
    for id in [
        "system:skill-creator",
        "user:team/release",
        "repo:web/lint",
        "plugin:demo@local:search",
        "plugin:demo@local:team/search",
    ] {
        for tool in ["skills_read", "enable_skill", "disable_skill"] {
            assert!(
                parse_call(tool, json!({"skill":id})).is_ok(),
                "{tool}: {id}"
            );
        }
    }
    assert!(
        matches!(parse_call("skills_list", json!({"authority":"plugin"})).unwrap(), SkillCall::List(list) if list.authority == Some(SkillCatalogAuthority::Plugin))
    );
}

#[test]
fn an_empty_skill_list_still_reports_the_configured_user_root() {
    let (skills, root) = empty_skills("empty-user-root");
    let catalog = skills.turn_catalog().unwrap();
    let SkillCall::List(call) = parse_call("skills_list", json!({})).unwrap() else {
        panic!("list call")
    };
    let result = super::super::read::list_skills(&catalog, &call).unwrap();
    let value = result.output.structured_content.unwrap();
    assert_eq!(value["entries"], json!([]));
    let expected =
        hanafubuki_agent_foundation::paths::ResolvedPath::resolve(root.join("user")).unwrap();
    assert_eq!(value["user_root"], expected.native_text());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn a_linked_skill_directory_does_not_change_the_reported_user_root() {
    let (skills, root) = empty_skills("linked-user-root");
    let source = root.join("checkout/release");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("SKILL.md"),
        "---\nname: release\ndescription: Release a package.\n---\nDo the release.\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("user/team")).unwrap();
    std::os::unix::fs::symlink(&source, root.join("user/team/release")).unwrap();
    let catalog = skills.turn_catalog().unwrap();
    let SkillCall::List(call) = parse_call("skills_list", json!({"authority":"user"})).unwrap()
    else {
        panic!("list call")
    };
    let value = super::super::read::list_skills(&catalog, &call)
        .unwrap()
        .output
        .structured_content
        .unwrap();
    assert_eq!(value["entries"][0]["skill"], "user:team/release");
    let source = hanafubuki_agent_foundation::paths::ResolvedPath::resolve(source).unwrap();
    let user =
        hanafubuki_agent_foundation::paths::ResolvedPath::resolve(root.join("user")).unwrap();
    assert_eq!(value["entries"][0]["directory"], source.native_text());
    assert_eq!(value["user_root"], user.native_text());
    std::fs::remove_dir_all(root).unwrap();
}
