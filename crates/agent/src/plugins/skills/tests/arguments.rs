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
