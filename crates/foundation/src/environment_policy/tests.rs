use super::*;

fn vars() -> BTreeMap<String, String> {
    [
        ("PATH", "/bin"),
        ("HOME", "/home/test"),
        ("API_TOKEN", "fake"),
        ("MySetting", "old"),
        ("KEEP_1", "yes"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect()
}

#[test]
fn policies_and_filter_order_match_codex() {
    assert_eq!(EnvironmentPolicy::default().apply(vars()), vars());
    let mut policy = EnvironmentPolicy {
        inherit: EnvironmentInherit::None,
        ..Default::default()
    };
    assert!(policy.apply(vars()).is_empty());
    policy.inherit = EnvironmentInherit::Core;
    assert!(!policy.apply(vars()).contains_key("API_TOKEN"));
    assert!(policy.apply(vars()).contains_key("PATH"));
    policy.inherit = EnvironmentInherit::All;
    policy.ignore_default_excludes = false;
    policy.exclude = vec!["my*".into()];
    policy.set.insert("MySetting".into(), "new".into());
    policy.include_only = vec!["mysetting".into(), "keep_?".into(), "api_token".into()];
    assert_eq!(
        policy.apply(vars()),
        BTreeMap::from([
            ("MySetting".into(), "new".into()),
            ("KEEP_1".into(), "yes".into())
        ])
    );
    policy.set.insert("API_TOKEN".into(), "configured".into());
    assert_eq!(policy.apply(vars())["API_TOKEN"], "configured");
}

#[test]
fn patterns_are_case_insensitive_and_support_wildcards() {
    assert!(matches("*secret*", "MY_SECRET_VALUE"));
    assert!(matches("Ä?*", "äbc"));
    assert!(!matches("A?", "A"));
    assert!(!matches("A", "AB"));
    assert!(matches("*", ""));
    assert!(matches("A*B?C", "a__bxc"));
}
