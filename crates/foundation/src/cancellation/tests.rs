use super::CancellationToken;
use std::time::{Duration, Instant};

#[test]
fn cancellation_wakes_waiters_before_the_timeout() {
    let token = CancellationToken::default();
    let waiter = token.clone();
    let started = Instant::now();
    let handle = std::thread::spawn(move || waiter.wait_timeout(Duration::from_secs(30)));
    std::thread::sleep(Duration::from_millis(20));
    token.cancel();
    assert!(handle.join().unwrap());
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn a_child_is_cancelled_with_its_parent_but_not_the_reverse() {
    let parent = CancellationToken::default();
    let first = parent.child();
    let grandchild = first.child();
    let second = parent.child();

    second.cancel();
    assert!(!parent.is_cancelled());
    assert!(!first.is_cancelled());

    parent.cancel();
    assert!(first.is_cancelled());
    assert!(grandchild.is_cancelled());
    assert!(parent.child().is_cancelled());
}
