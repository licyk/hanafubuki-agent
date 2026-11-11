use super::*;

fn tool<'a>(name: &'a str, owner: &'a str, provider: Option<&'a str>) -> ToolIdentity<'a> {
    ToolIdentity {
        name,
        owner,
        provider,
    }
}

fn patterns(texts: &[&str]) -> BTreeSet<ToolPattern> {
    texts
        .iter()
        .map(|text| ToolPattern::parse(text).unwrap())
        .collect()
}

#[test]
fn patterns_parse_by_kind_and_print_as_written() {
    for (text, pattern) in [
        ("read_file", ToolPattern::Name("read_file".into())),
        ("mcp__github__*", ToolPattern::Name("mcp__github__*".into())),
        ("plugin:web", ToolPattern::Plugin("web".into())),
        (
            "plugin:guest:demo@local",
            ToolPattern::Plugin("guest:demo@local".into()),
        ),
        (
            "provider:mcp.github",
            ToolPattern::Provider("mcp.github".into()),
        ),
    ] {
        assert_eq!(ToolPattern::parse(text).unwrap(), pattern);
        assert_eq!(pattern.to_string(), text);
    }
    for text in [
        "",
        "plugin:",
        "provider:",
        "read file",
        "mode:x",
        &"a".repeat(257),
    ] {
        assert!(ToolPattern::parse(text).is_err(), "{text:?}");
    }
}

#[test]
fn a_name_pattern_matches_with_its_wildcards() {
    let matches = |pattern: &str, name: &str| {
        ToolPattern::parse(pattern)
            .unwrap()
            .matches(&tool(name, "row", None))
    };
    assert!(matches("read_file", "read_file"));
    assert!(!matches("read_file", "read_files"));
    assert!(matches("mcp__github__*", "mcp__github__list_issues"));
    assert!(!matches("mcp__github__*", "mcp__gitlab__list_issues"));
    assert!(matches("*", "anything"));
    assert!(matches("*_file", "read_file"));
    assert!(matches("a*b*c", "abc"));
    assert!(!matches("a*a", "a"));
    assert!(matches("*ab*ab", "abab"));
}

#[test]
fn plugin_and_provider_patterns_match_the_owner_and_the_provider() {
    let web = tool("web_search", "web", None);
    let sourced = tool("mcp__github__list", "mcp", Some("mcp.github"));
    assert!(ToolPattern::parse("plugin:web").unwrap().matches(&web));
    assert!(!ToolPattern::parse("plugin:web").unwrap().matches(&sourced));
    assert!(ToolPattern::parse("provider:mcp.*")
        .unwrap()
        .matches(&sourced));
    assert!(!ToolPattern::parse("provider:*").unwrap().matches(&web));
}

#[test]
fn every_allow_list_must_match_and_a_deny_always_wins() {
    let scope = AgentToolScope {
        allow: vec![
            patterns(&["read_*"]),
            patterns(&["read_file", "web_search"]),
        ],
        deny: BTreeSet::new(),
    };
    assert!(scope.allows(&tool("read_file", "workspace-fs", None)));
    assert!(!scope.allows(&tool("read_dir", "workspace-fs", None)));
    assert!(!scope.allows(&tool("web_search", "web", None)));

    let scope = AgentToolScope {
        allow: vec![patterns(&["plugin:web"]), patterns(&["web_search"])],
        deny: patterns(&["fetch_web_page"]),
    };
    assert!(scope.allows(&tool("web_search", "web", None)));
    assert!(!scope.allows(&tool("web_search", "other", None)));
    assert!(!scope.allows(&tool("fetch_web_page", "web", None)));

    assert!(AgentToolScope::default().allows(&tool("anything", "row", None)));
    let none = AgentToolScope {
        allow: vec![BTreeSet::new()],
        deny: BTreeSet::new(),
    };
    assert!(!none.allows(&tool("anything", "row", None)));
}

#[test]
fn narrowing_keeps_both_sides_restrictions() {
    let parent = AgentCapabilityScope {
        role: Some("parent".into()),
        tools: AgentToolScope {
            allow: vec![patterns(&["read_*", "web_search"])],
            deny: patterns(&["web_search"]),
        },
        plugins_off: ["titles".to_string()].into(),
        permission_mode_cap: Some(AgentPermissionMode::AutoReview),
    };
    let child = AgentCapabilityScope {
        role: Some("child".into()),
        tools: AgentToolScope {
            allow: vec![patterns(&["read_file"])],
            deny: patterns(&["read_dir"]),
        },
        plugins_off: ["goal".to_string()].into(),
        permission_mode_cap: Some(AgentPermissionMode::FullAccess),
    };
    let narrowed = parent.narrow(&child);
    assert_eq!(narrowed.role.as_deref(), Some("child"));
    assert_eq!(narrowed.tools.allow.len(), 2);
    assert_eq!(narrowed.tools.deny, patterns(&["web_search", "read_dir"]));
    assert_eq!(
        narrowed.plugins_off,
        ["goal".to_string(), "titles".to_string()].into()
    );
    assert_eq!(
        narrowed.permission_mode_cap,
        Some(AgentPermissionMode::AutoReview)
    );
    assert!(narrowed.tools.allows(&tool("read_file", "fs", None)));
    assert!(!narrowed.tools.allows(&tool("read_dir", "fs", None)));
    // The same list twice narrows nothing more.
    assert_eq!(narrowed.narrow(&child).tools.allow.len(), 2);
}

#[test]
fn the_stricter_mode_is_the_one_that_asks_more() {
    use AgentPermissionMode::{AutoReview, FullAccess, Manual};
    for (first, second, stricter) in [
        (Manual, FullAccess, Manual),
        (FullAccess, Manual, Manual),
        (AutoReview, FullAccess, AutoReview),
        (FullAccess, AutoReview, AutoReview),
        (Manual, AutoReview, Manual),
        (FullAccess, FullAccess, FullAccess),
    ] {
        assert_eq!(stricter_permission_mode(first, second), stricter);
    }
}

fn explorer_like() -> AgentCapabilityScope {
    AgentCapabilityScope {
        role: Some("reader".into()),
        tools: AgentToolScope {
            allow: vec![
                patterns(&["read_*", "plugin:web"]),
                patterns(&["read_file"]),
            ],
            deny: patterns(&["provider:mcp.github"]),
        },
        plugins_off: ["titles".to_string()].into(),
        permission_mode_cap: Some(AgentPermissionMode::AutoReview),
    }
}

#[test]
fn a_scope_round_trips_through_its_persisted_form() {
    let scope = explorer_like();
    let value = serde_json::to_value(&scope).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "role": "reader",
            "tools": {
                "allow": [["read_*", "plugin:web"], ["read_file"]],
                "deny": ["provider:mcp.github"]
            },
            "pluginsOff": ["titles"],
            "permissionModeCap": "auto_review"
        })
    );
    assert_eq!(
        serde_json::from_value::<AgentCapabilityScope>(value).unwrap(),
        scope
    );
    // The empty scope writes nothing of what it does not narrow.
    assert_eq!(
        serde_json::to_value(AgentCapabilityScope::default()).unwrap(),
        serde_json::json!({})
    );
}

#[test]
fn a_persisted_scope_is_read_field_by_field_and_never_wider() {
    let scope: AgentCapabilityScope = serde_json::from_value(serde_json::json!({
        "role": 7,
        "tools": {
            "allow": [["read_file", "mode:x", 3], "not a list"],
            "deny": ["web_*", "future:kind"]
        },
        "pluginsOff": ["goal", 1],
        "permissionModeCap": "sometimes",
        "unknown": true
    }))
    .unwrap();
    assert_eq!(scope.role, None);
    assert_eq!(
        scope.tools.allow,
        vec![patterns(&["read_file"]), BTreeSet::new()]
    );
    assert_eq!(scope.tools.deny, patterns(&["web_*"]));
    assert_eq!(scope.plugins_off, ["goal".to_string()].into());
    assert_eq!(scope.permission_mode_cap, None);
    // The allow list that was not a list allows nothing.
    assert!(!scope.tools.allows(&tool("read_file", "fs", None)));
    assert_eq!(
        serde_json::from_value::<AgentCapabilityScope>(serde_json::json!("garbage")).unwrap(),
        AgentCapabilityScope::default()
    );
}

#[test]
fn normalizing_sorts_the_allow_lists_and_keeps_the_core_rows_on() {
    let mut scope = explorer_like();
    scope.tools.allow.push(patterns(&["read_*", "plugin:web"]));
    scope.plugins_off.insert("agent-kernel".to_string());
    scope.plugins_off.insert("agent-host-services".to_string());
    let repaired = scope.normalize();
    assert_eq!(
        scope.tools.allow,
        vec![
            patterns(&["plugin:web", "read_*"]),
            patterns(&["read_file"])
        ]
    );
    assert_eq!(scope.plugins_off, ["titles".to_string()].into());
    assert_eq!(repaired.len(), 1, "{repaired:?}");
    assert!(explorer_like().normalize().is_empty());
}

#[test]
fn a_scope_past_its_bounds_allows_no_tool() {
    let many: BTreeSet<ToolPattern> = (0..=MAX_SCOPE_TOOL_PATTERNS)
        .map(|index| ToolPattern::Name(format!("tool_{index}")))
        .collect();
    for tools in [
        AgentToolScope {
            allow: Vec::new(),
            deny: many.clone(),
        },
        AgentToolScope {
            allow: vec![many],
            deny: BTreeSet::new(),
        },
        AgentToolScope {
            allow: Vec::new(),
            deny: [ToolPattern::Name("x".repeat(MAX_TOOL_PATTERN_BYTES + 1))].into(),
        },
    ] {
        let mut scope = AgentCapabilityScope {
            tools,
            ..AgentCapabilityScope::default()
        };
        assert_eq!(scope.normalize().len(), 1);
        assert_eq!(scope.tools.allow, vec![BTreeSet::new()]);
        assert!(!scope.tools.allows(&tool("tool_1", "row", None)));
    }
}

#[test]
fn a_child_without_a_role_keeps_its_parents_and_the_cap_only_tightens() {
    let parent = explorer_like();
    let narrowed = parent.narrow(&AgentCapabilityScope::default());
    assert_eq!(narrowed, parent);
    use AgentPermissionMode::{AutoReview, FullAccess, Manual};
    assert_eq!(parent.capped_permission_mode(FullAccess), AutoReview);
    assert_eq!(parent.capped_permission_mode(Manual), Manual);
    assert_eq!(
        AgentCapabilityScope::default().capped_permission_mode(FullAccess),
        FullAccess
    );
    let summary = parent.summary().unwrap();
    assert_eq!(
        (
            summary.allow_lists,
            summary.allow_patterns,
            summary.deny_patterns
        ),
        (2, 3, 1)
    );
    assert_eq!(summary.plugins_off, ["titles"]);
    assert_eq!(AgentCapabilityScope::default().summary(), None);
}
