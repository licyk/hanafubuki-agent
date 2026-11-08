use super::*;

fn parents(line: &str) -> Vec<u64> {
    let mut found = HashSet::new();
    collect_parents(line.as_bytes(), &mut found);
    let mut found = found.into_iter().map(|id| id.value()).collect::<Vec<_>>();
    found.sort_unstable();
    found
}

#[test]
fn a_parent_is_read_from_either_key_whatever_the_spacing() {
    assert_eq!(
        parents(
            r#"{"runtimeCheckpoint":{"forkedFromSessionId":3,"historyBase":{"endOrdinalExclusive":4,"sessionId":2,"endByteOffset":9}}}"#
        ),
        vec![2, 3]
    );
    assert_eq!(
        parents(r#"{"forkedFromSessionId" : 7, "historyBase" : { "sessionId" : 7 }}"#),
        vec![7]
    );
}

#[test]
fn no_parent_is_read_where_none_is_named() {
    assert!(parents(r#"{"forkedFromSessionId":null,"historyBase":null,"sessionId":5}"#).is_empty());
    // A `sessionId` outside a history base is the record's own.
    assert!(parents(r#"{"historyBase":{"endOrdinalExclusive":1},"sessionId":5}"#).is_empty());
    assert!(parents("not json \"forkedFromSessionId\"").is_empty());
}

#[test]
fn a_quarantined_rollout_names_its_session() {
    assert_eq!(
        quarantined_session_id(Path::new(
            "/h/active/.recovery/session-2026-11-05T12-23-48-12.jsonl.4321.0.invalid"
        )),
        Some(AgentSessionId::new(12))
    );
    assert_eq!(
        quarantined_session_id(Path::new("/h/active/.recovery/foreign.jsonl.1.0.invalid")),
        None
    );
    assert_eq!(
        quarantined_session_id(Path::new("/h/active/session-1.jsonl")),
        None
    );
}
