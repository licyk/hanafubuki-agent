use super::*;

#[test]
fn shutdown_cancels_and_joins_named_worker() {
    let group = BackgroundWorkerGroup::default();
    group
        .spawn("observer", |token| {
            while !token.is_cancelled() {
                thread::yield_now();
            }
        })
        .unwrap();
    assert!(group
        .shutdown(Instant::now() + Duration::from_secs(1))
        .is_empty());
}

#[test]
fn shutdown_rejects_late_worker_registration() {
    let group = BackgroundWorkerGroup::default();
    assert!(group
        .shutdown(Instant::now() + Duration::from_secs(1))
        .is_empty());
    assert!(group.spawn("late", |_| {}).is_err());
}

#[test]
fn shutdown_cancels_a_caller_supplied_token() {
    let group = BackgroundWorkerGroup::default();
    let cancellation = CancellationToken::default();
    let observed = cancellation.clone();
    group
        .spawn_with_cancellation("external-token", cancellation, |token| {
            while !token.is_cancelled() {
                thread::yield_now();
            }
        })
        .unwrap();

    assert!(group
        .shutdown(Instant::now() + Duration::from_secs(1))
        .is_empty());
    assert!(observed.is_cancelled());
}

#[test]
fn dropping_last_group_handle_cancels_detached_workers() {
    let (stopped_tx, stopped_rx) = std::sync::mpsc::channel();
    let group = BackgroundWorkerGroup::default();
    group
        .spawn("drop-observer", move |token| {
            while !token.is_cancelled() {
                thread::yield_now();
            }
            stopped_tx.send(()).unwrap();
        })
        .unwrap();
    drop(group);
    stopped_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("dropping the worker group must request cancellation");
}

#[test]
fn a_panicking_worker_is_reported_and_the_group_keeps_working() {
    let group = BackgroundWorkerGroup::default();
    let panicked = group
        .spawn_observed("panicking", |_| panic!("worker body failed"))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !panicked.has_exited() {
        assert!(Instant::now() < deadline, "the worker never ended");
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        panicked.panic_message().as_deref(),
        Some("worker body failed")
    );

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let healthy = group
        .spawn_observed("healthy", move |_| done_tx.send(()).unwrap())
        .unwrap();
    done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    while !healthy.has_exited() {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(healthy.panic_message(), None);

    assert_eq!(
        group.shutdown(Instant::now() + Duration::from_secs(1)),
        vec![WorkerShutdownDiagnostic {
            worker: "panicking".to_string(),
            message: "worker panicked: worker body failed".to_string(),
        }]
    );
}

#[test]
fn only_a_contained_group_marks_its_member_threads() {
    for (group, expected) in [
        (BackgroundWorkerGroup::contained(), true),
        (BackgroundWorkerGroup::default(), false),
    ] {
        let (sender, receiver) = std::sync::mpsc::channel();
        group
            .spawn("containment probe", move |_| {
                let _ = sender.send(crate::panic::is_panic_contained());
            })
            .unwrap();
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
            expected
        );
        assert!(group
            .shutdown(Instant::now() + Duration::from_secs(5))
            .is_empty());
    }
}

#[test]
fn a_group_with_a_panic_handler_tells_it_which_member_panicked() {
    let (reports, heard) = std::sync::mpsc::channel();
    let reports = parking_lot::Mutex::new(reports);
    let group = BackgroundWorkerGroup::contained_with_panic_handler(Arc::new(
        move |worker: &str, message: &str| {
            let _ = reports
                .lock()
                .send((worker.to_string(), message.to_string()));
        },
    ));
    group.spawn("returns", |_| {}).unwrap();
    group
        .spawn("panics", |_| panic!("member body failed"))
        .unwrap();

    assert_eq!(
        heard.recv_timeout(Duration::from_secs(5)).unwrap(),
        ("panics".to_string(), "member body failed".to_string())
    );
    let _ = group.shutdown(Instant::now() + Duration::from_secs(1));
    assert!(
        heard.try_recv().is_err(),
        "a member that returned was reported"
    );
}
