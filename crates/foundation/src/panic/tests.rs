use std::future::Future;
use std::task::{Context, Poll, Waker};

use super::*;

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("the future was expected to be ready"),
    }
}

#[test]
fn a_panic_becomes_its_message() {
    assert_eq!(
        catch_panic(|| panic!("static text")),
        Err::<(), _>("static text".into())
    );
    let detail = 7;
    assert_eq!(
        catch_panic(|| panic!("formatted {detail}")),
        Err::<(), _>("formatted 7".into())
    );
    assert_eq!(
        catch_panic(|| std::panic::panic_any(42_u32)),
        Err::<(), _>("panicked".into())
    );
    assert_eq!(catch_panic(|| 3), Ok(3));
}

#[test]
fn a_long_message_is_cut_on_a_character_boundary() {
    let long = "é".repeat(MAX_PANIC_MESSAGE_BYTES);
    let message = catch_panic(|| std::panic::panic_any(long.clone())).unwrap_err();
    assert!(message.len() <= MAX_PANIC_MESSAGE_BYTES);
    assert!(long.starts_with(&message));
}

#[test]
fn a_future_that_panics_while_polled_becomes_its_message() {
    assert_eq!(
        ready(catch_panic_async(async { panic!("polled") })),
        Err::<(), _>("polled".into())
    );
    assert_eq!(ready(catch_panic_async(async { 5 })), Ok(5));
}

#[test]
fn a_guard_contains_the_panics_raised_inside_it() {
    assert!(!is_panic_contained());
    assert_eq!(catch_panic(is_panic_contained), Ok(true));
    let nested =
        catch_panic(|| catch_panic(is_panic_contained).map(|inner| (inner, is_panic_contained())));
    assert_eq!(nested, Ok(Ok((true, true))));
    let _ = catch_panic(|| panic!("released"));
    assert!(
        !is_panic_contained(),
        "a guard that caught a panic is closed"
    );
}

#[test]
fn each_poll_of_a_guarded_future_is_contained() {
    assert_eq!(
        ready(catch_panic_async(async { is_panic_contained() })),
        Ok(true)
    );
    assert!(!is_panic_contained());
}

#[test]
fn a_marked_thread_is_contained_until_unmarked() {
    std::thread::spawn(|| {
        assert!(!is_panic_contained());
        mark_thread_contained();
        assert!(is_panic_contained());
        let spawned = std::thread::spawn(is_panic_contained).join().unwrap();
        assert!(!spawned, "a thread a contained thread spawns is not marked");
        unmark_thread_contained();
        assert!(!is_panic_contained());
    })
    .join()
    .unwrap();
}

thread_local! {
    static CAUGHT_ELSEWHERE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn caught_elsewhere() -> bool {
    CAUGHT_ELSEWHERE.with(std::cell::Cell::get)
}

#[test]
fn a_registered_check_contains_the_panics_on_the_threads_it_names() {
    register_containment_check(caught_elsewhere);
    std::thread::spawn(|| {
        assert!(!is_panic_contained());
        CAUGHT_ELSEWHERE.with(|caught| caught.set(true));
        assert!(is_panic_contained());
    })
    .join()
    .unwrap();
    assert!(!is_panic_contained(), "another thread's mark is its own");
}

#[test]
fn the_fault_log_counts_every_panic_and_keeps_the_latest() {
    let log = PanicFaultLog::new(2);
    for index in 0..3 {
        log.record(PanicFault::now(
            format!("thread-{index}"),
            "src/lib.rs:1:1".to_string(),
            format!("panic {index}"),
            index % 2 == 0,
        ));
    }
    let summary = log.summary();
    assert_eq!(summary.count, 3);
    assert_eq!(
        summary
            .recent
            .iter()
            .map(|fault| (fault.message.as_str(), fault.contained))
            .collect::<Vec<_>>(),
        [("panic 1", false), ("panic 2", true)]
    );
}
