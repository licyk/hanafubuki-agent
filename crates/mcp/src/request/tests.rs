use super::*;
use crate::interaction::ElicitationPauseState;

#[tokio::test]
async fn active_timeout_counts_only_unpaused_protocol_time() {
    let pause_state = ElicitationPauseState::new();
    let pause = pause_state.enter();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(75)).await;
        drop(pause);
    });

    let result = active_time_timeout(Duration::from_millis(50), pause_state.subscribe(), async {
        tokio::time::sleep(Duration::from_millis(90)).await;
        "completed"
    })
    .await;
    assert_eq!(result, Ok("completed"));
}

#[tokio::test]
async fn nested_elicitation_guards_resume_only_after_the_last_drop() {
    let pause_state = ElicitationPauseState::new();
    let first = pause_state.enter();
    let second = pause_state.enter();
    let task = tokio::spawn({
        let pause_state = pause_state.clone();
        async move {
            active_time_timeout(
                Duration::from_millis(20),
                pause_state.subscribe(),
                std::future::pending::<()>(),
            )
            .await
        }
    });

    tokio::time::sleep(Duration::from_millis(30)).await;
    drop(first);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!task.is_finished());
    drop(second);
    assert_eq!(task.await.unwrap(), Err(()));
}

#[tokio::test]
async fn active_timeout_expires_after_the_pause_resumes() {
    let pause_state = ElicitationPauseState::new();
    let pause = pause_state.enter();
    let task = tokio::spawn({
        let pause_state = pause_state.clone();
        async move {
            active_time_timeout(
                Duration::from_millis(20),
                pause_state.subscribe(),
                std::future::pending::<()>(),
            )
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!task.is_finished());
    drop(pause);
    assert_eq!(task.await.unwrap(), Err(()));
}
