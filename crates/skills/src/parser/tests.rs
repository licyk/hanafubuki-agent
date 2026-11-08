use super::*;

fn parse(document: &str) -> Result<ParsedSkill, SkillError> {
    parse_skill(document.as_bytes(), "folder-name")
}

#[test]
fn a_name_and_description_are_all_a_skill_needs() {
    let parsed = parse("---\nname: pdf\ndescription: Work with PDFs.\n---\n\nBody.\n").unwrap();
    assert_eq!(parsed.name, "pdf");
    assert_eq!(parsed.description, "Work with PDFs.");
    assert!(parsed.document.starts_with("---"));
    assert!(parsed.document.ends_with("Body."));
}

#[test]
fn an_unnamed_skill_takes_its_folder_name() {
    let parsed = parse("---\ndescription: Does a thing.\n---\nBody.\n").unwrap();
    assert_eq!(parsed.name, "folder-name");
}

#[test]
fn unknown_frontmatter_keys_are_ignored_rather_than_rejected() {
    // Real packages carry keys other tools read. Rejecting them was the whole
    // reason valid Skills reported "invalid skill manifest".
    let parsed = parse(
        "---\nname: aws\ndescription: Deploy things.\nargument-hint: <duration>\nmodel: opus\nauthor: someone\n---\nBody.\n",
    )
    .unwrap();
    assert_eq!(parsed.name, "aws");
}

#[test]
fn prose_that_is_not_quite_yaml_is_repaired() {
    // `description: Build for AWS: ECS` parses as a nested mapping and fails;
    // the line-oriented repair quotes exactly that scalar.
    let parsed =
        parse("---\nname: aws\ndescription: Build for AWS: ECS and Fargate\n---\nBody.\n").unwrap();
    assert_eq!(parsed.description, "Build for AWS: ECS and Fargate");

    let hinted =
        parse("---\ndescription: Do a thing.\nargument-hint: <duration: e.g. 7d>\n---\nB\n")
            .unwrap();
    assert_eq!(hinted.description, "Do a thing.");
}

#[test]
fn angle_brackets_survive_a_description() {
    // The previous parser rejected these outright, which failed packages whose
    // descriptions name placeholders.
    let parsed = parse("---\ndescription: Render <name> into a report.\n---\nB\n").unwrap();
    assert_eq!(parsed.description, "Render <name> into a report.");
}

#[test]
fn a_description_is_required_and_frontmatter_must_be_terminated() {
    assert!(matches!(
        parse("---\nname: x\n---\nBody.\n"),
        Err(SkillError::InvalidManifest(_))
    ));
    assert!(matches!(
        parse("---\ndescription: x\nBody without a close\n"),
        Err(SkillError::InvalidManifest(_))
    ));
    assert!(matches!(
        parse("No frontmatter at all.\n"),
        Err(SkillError::InvalidManifest(_))
    ));
}

#[test]
fn a_byte_order_mark_does_not_hide_the_frontmatter() {
    let parsed = parse("\u{feff}---\ndescription: Does a thing.\n---\nBody.\n").unwrap();
    assert_eq!(parsed.description, "Does a thing.");
}

#[test]
fn allowed_tools_are_an_unknown_key_and_ignored_without_an_issue() {
    let parsed =
        parse("---\ndescription: d\nallowed-tools:\n  - read_file\n  - 7\n---\nB\n").unwrap();
    assert_eq!(parsed.description, "d");
    assert!(parsed.issues.is_empty());
}

#[test]
fn metadata_carries_a_short_description_and_a_version() {
    let parsed = parse(
        "---\ndescription: Long one.\nlicense: MIT\nmetadata:\n  version: 1.2.3\n  short-description: Short one.\n---\nB\n",
    )
    .unwrap();
    assert_eq!(parsed.short_description.as_deref(), Some("Short one."));
    assert_eq!(parsed.version.as_deref(), Some("1.2.3"));
    assert_eq!(parsed.license.as_deref(), Some("MIT"));
}

#[test]
fn a_slug_is_a_usable_folder_name() {
    assert_eq!(slug_from_name("PDF Tools"), "pdf-tools");
    assert_eq!(slug_from_name("  Review — Agent  "), "review-agent");
    assert_eq!(slug_from_name("v2.1"), "v2-1");
    assert_eq!(slug_from_name("!!!"), "");
}

#[test]
fn a_known_key_of_the_wrong_type_is_ignored_with_an_issue() {
    // `version: 1.0` is a YAML number. Decoding the frontmatter as one struct
    // failed the whole Skill for it.
    let parsed = parse(
        "---\nname: pdf\ndescription: Work with PDFs.\nlicense: 3\nmetadata:\n  version: 1.0\n  short-description: Short one.\n---\nB\n",
    )
    .unwrap();
    assert_eq!(parsed.name, "pdf");
    assert_eq!(parsed.version, None);
    assert_eq!(parsed.license, None);
    assert_eq!(parsed.short_description.as_deref(), Some("Short one."));
    assert_eq!(
        parsed.issues,
        vec![
            "ignored license: expected a string, found a number",
            "ignored metadata.version: expected a string, found a number",
        ]
    );

    let flat = parse("---\ndescription: d\nmetadata: v1\nlicense: 5\n---\nB\n").unwrap();
    assert_eq!(flat.version, None);
    assert_eq!(flat.license, None);
    assert_eq!(flat.issues.len(), 2);

    let clean = parse("---\nname: pdf\ndescription: Work with PDFs.\n---\nB\n").unwrap();
    assert!(clean.issues.is_empty());
}

#[test]
fn a_name_or_description_of_the_wrong_type_still_fails_the_skill() {
    for document in [
        "---\nname: 123\ndescription: d\n---\nB\n",
        "---\nname: [a, b]\ndescription: d\n---\nB\n",
        "---\ndescription:\n  nested: map\n---\nB\n",
        "---\n- just\n- a list\n---\nB\n",
    ] {
        assert!(
            matches!(parse(document), Err(SkillError::InvalidManifest(_))),
            "{document}"
        );
    }
}

#[test]
fn an_overlong_name_or_description_is_shortened_on_a_character_boundary() {
    // Three-byte characters, so a byte cut at the bound would split one.
    let name = "名".repeat(100);
    let description = format!("a{}", "説".repeat(1400));
    let parsed = parse(&format!(
        "---\nname: {name}\ndescription: {description}\n---\nB\n"
    ))
    .unwrap();
    assert_eq!(parsed.name.chars().count(), MAX_NAME_CHARS);
    assert!(name.starts_with(&parsed.name));
    assert!(parsed.description.len() <= MAX_DESCRIPTION_BYTES);
    // 1 + 3 * 1365 = 4096: the last character that fits whole.
    assert_eq!(parsed.description.len(), 4096);
    assert!(description.starts_with(&parsed.description));
    assert_eq!(parsed.issues.len(), 2);
    assert!(parsed.issues[0].starts_with("name is longer"));
    assert!(parsed.issues[1].starts_with("description is longer"));

    let uneven = parse(&format!(
        "---\ndescription: ab{}\n---\nB\n",
        "説".repeat(1400)
    ))
    .unwrap();
    // 2 + 3 * 1364 = 4094; one more character would pass the bound.
    assert_eq!(uneven.description.len(), 4094);
}
