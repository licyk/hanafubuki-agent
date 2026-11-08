use super::*;

#[test]
fn the_policy_is_deny_ask_or_the_tools_allowed() {
    assert_eq!(ApprovePolicy::parse("deny").unwrap(), ApprovePolicy::Deny);
    assert_eq!(ApprovePolicy::parse("ask").unwrap(), ApprovePolicy::Ask);
    assert_eq!(
        ApprovePolicy::parse("allow:exec_command, apply_patch").unwrap(),
        ApprovePolicy::Allow(BTreeSet::from([
            "apply_patch".to_string(),
            "exec_command".to_string()
        ]))
    );
    for policy in ["allow", "allow:", "allow:,", "yes"] {
        assert!(ApprovePolicy::parse(policy).is_err(), "{policy}");
    }
}
