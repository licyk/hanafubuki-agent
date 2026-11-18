use super::*;
#[test]
fn reports_retained_feedback_and_skips_an_unreadable_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outcomes.jsonl");
    std::fs::write(&path, b"broken\n").unwrap();
    let ledger = Ledger::new(path.clone());
    ledger
        .record(Feedback {
            advice_id: "private-id".into(),
            adoption: Adoption::Followed,
            validation_status: Validation::Passed,
        })
        .unwrap();
    let stats = ledger.stats().unwrap();
    assert_eq!((stats.total, stats.malformed), (1, 1));
    assert_eq!(stats.adoption[&Adoption::Followed], 1);
    assert_eq!(stats.validation[&Validation::Passed], 1);
    assert!(!std::fs::read_to_string(path)
        .unwrap()
        .contains("private-id"));
}
