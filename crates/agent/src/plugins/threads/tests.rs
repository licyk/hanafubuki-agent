use super::*;

#[test]
fn conversation_subject_uses_the_numeric_thread_id() {
    assert_eq!(
        ThreadPresenter.subject("read_thread", &json!({"threadId": 70})),
        Some("session 70".into())
    );
    assert_eq!(ThreadPresenter.subject("read_thread", &json!({})), None);
}
