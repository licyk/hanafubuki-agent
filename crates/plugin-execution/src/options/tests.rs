use super::*;
use std::collections::BTreeMap;

#[test]
fn environment_overrides_preserve_hidden_names_and_values() {
    let options: ExecutionOptions = serde_json::from_value(serde_json::json!({
        "environmentSet": [
            "=C:=C:\\Users\\example",
            "IntelliJ IDEA=D:\\Tools\\IDE",
            "变量=值=后缀",
            "EMPTY=",
            "WITH_EQUALS=one=two"
        ]
    }))
    .unwrap();
    assert_eq!(
        options.shell_environment().unwrap().policy.set,
        BTreeMap::from([
            ("=C:".into(), r"C:\Users\example".into()),
            ("IntelliJ IDEA".into(), r"D:\Tools\IDE".into()),
            ("变量".into(), "值=后缀".into()),
            ("EMPTY".into(), String::new()),
            ("WITH_EQUALS".into(), "one=two".into()),
        ])
    );
}

#[test]
fn environment_override_requires_an_assignment_separator() {
    let options: ExecutionOptions = serde_json::from_value(serde_json::json!({
        "environmentSet": ["NO_ASSIGNMENT"]
    }))
    .unwrap();
    assert_eq!(
        options.shell_environment().unwrap_err(),
        "environmentSet entries must be NAME=VALUE"
    );
}

#[test]
fn unfiltered_keeps_explicit_additions_and_switching_back_restores_filters() {
    let mut options: ExecutionOptions = serde_json::from_value(serde_json::json!({
        "environmentInherit": "unfiltered",
        "environmentIgnoreDefaultExcludes": false,
        "environmentExclude": ["*"],
        "environmentIncludeOnly": ["CONFIGURED"],
        "environmentSet": ["CONFIGURED=new", "EXPLICIT=kept"]
    }))
    .unwrap();
    let inherited = BTreeMap::from([
        ("API_TOKEN".into(), "fake-token".into()),
        ("PRIVATE_VALUE".into(), "parent".into()),
        ("CONFIGURED".into(), "old".into()),
    ]);
    let environment = options
        .shell_environment()
        .unwrap()
        .policy
        .apply(inherited.clone());
    assert_eq!(environment["API_TOKEN"], "fake-token");
    assert_eq!(environment["PRIVATE_VALUE"], "parent");
    assert_eq!(environment["CONFIGURED"], "new");
    assert_eq!(environment["EXPLICIT"], "kept");
    options.environment_inherit = EnvironmentInherit::All;
    assert_eq!(
        options.shell_environment().unwrap().policy.apply(inherited),
        BTreeMap::from([("CONFIGURED".into(), "new".into())])
    );
}
