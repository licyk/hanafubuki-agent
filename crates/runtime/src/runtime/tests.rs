use std::time::Duration;

use hanafubuki_agent_foundation::blocking::run_blocking_operation;
use hanafubuki_agent_foundation::panic::is_panic_contained;

use super::*;

fn thread_is_contained() -> (bool, Option<String>) {
    (
        is_panic_contained(),
        std::thread::current().name().map(str::to_string),
    )
}

fn agent_thread() -> (bool, Option<String>) {
    (true, Some(AGENT_RUNTIME_THREAD_NAME.to_string()))
}

#[test]
fn workers_and_the_blocking_pool_are_contained_agent_threads() {
    let runtime = ContainedRuntime::start(AGENT_RUNTIME_THREAD_NAME).unwrap();
    let port = runtime.blocking_port();
    let task_runtime = runtime.clone();
    let (worker, pooled, ported, ambient) = runtime
        .block_on(async move {
            let worker = thread_is_contained();
            let pooled = task_runtime
                .run_blocking(thread_is_contained)
                .await
                .unwrap();
            let ported =
                run_blocking_operation(port.as_ref(), "containment probe", thread_is_contained)
                    .await
                    .unwrap();
            // What Agent code spawns on its own, through the ambient runtime.
            let ambient = tokio::task::spawn_blocking(thread_is_contained)
                .await
                .unwrap();
            (worker, pooled, ported, ambient)
        })
        .unwrap();
    for observed in [worker, pooled, ported, ambient] {
        assert_eq!(observed, agent_thread());
    }
    assert!(!is_panic_contained(), "the caller is not marked");
    runtime.shutdown(Duration::from_secs(5)).unwrap();
}

#[test]
fn a_panic_is_the_work_s_failure_and_the_runtime_keeps_running() {
    let runtime = ContainedRuntime::start(AGENT_RUNTIME_THREAD_NAME).unwrap();
    assert_eq!(
        runtime.block_on(async { panic!("task panic") }),
        Err::<(), _>(RuntimeTaskFailure::Panicked("task panic".to_string()))
    );
    let task_runtime = runtime.clone();
    let blocking = runtime
        .block_on(async move {
            task_runtime
                .run_blocking(|| -> () { panic!("blocking panic") })
                .await
        })
        .unwrap();
    assert_eq!(
        blocking,
        Err(RuntimeTaskFailure::Panicked("blocking panic".to_string()))
    );
    let port = runtime.blocking_port();
    let ported = runtime
        .block_on(async move {
            run_blocking_operation(port.as_ref(), "probe", || -> () { panic!("port panic") }).await
        })
        .unwrap();
    assert_eq!(
        ported,
        Err("probe failed: panicked: port panic".to_string())
    );
    assert_eq!(runtime.block_on(async { 7 }), Ok(7));
    runtime.shutdown(Duration::from_secs(5)).unwrap();
}

#[test]
fn work_after_shutdown_fails_as_stopped() {
    let runtime = ContainedRuntime::start(AGENT_RUNTIME_THREAD_NAME).unwrap();
    runtime.shutdown(Duration::from_secs(5)).unwrap();
    assert_eq!(
        runtime.block_on(async { 1 }),
        Err(RuntimeTaskFailure::Stopped)
    );
    assert_eq!(runtime.shutdown(Duration::from_secs(5)), Ok(()));
}

#[test]
fn a_bounded_wait_returns_at_its_deadline_even_with_every_worker_blocked() {
    let runtime = ContainedRuntime::start(AGENT_RUNTIME_THREAD_NAME).unwrap();
    assert_eq!(
        runtime.block_on_until(async { 3 }, Instant::now() + Duration::from_secs(5)),
        Some(Ok(3))
    );
    // Work that never ends, on every worker thread.
    let (release, released) = std::sync::mpsc::channel::<()>();
    let released = Arc::new(Mutex::new(released));
    for _ in 0..MAX_WORKER_THREADS {
        let released = released.clone();
        runtime.handle().spawn(async move {
            let _ = released.lock().recv();
        });
    }
    let started = Instant::now();
    assert_eq!(
        runtime.block_on_until(
            std::future::pending::<()>(),
            started + Duration::from_millis(200)
        ),
        None
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    drop(release);
    runtime.shutdown(Duration::from_secs(5)).unwrap();
    assert_eq!(
        runtime.block_on_until(async { 1 }, Instant::now() + Duration::from_secs(1)),
        Some(Err(RuntimeTaskFailure::Stopped))
    );
}
