use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Barrier;
use tokio::sync::Notify;

#[tokio::test]
async fn reviewed_reads_can_overlap() {
    let gate = AgentToolExecutionGate::new(2).unwrap();
    let entered = Arc::new(Barrier::new(3));
    let active = Arc::new(AtomicUsize::new(0));

    let first = run_read(gate.clone(), entered.clone(), active.clone());
    let second = run_read(gate, entered.clone(), active.clone());
    let observer = async {
        entered.wait().await;
        assert_eq!(active.load(Ordering::SeqCst), 2);
    };

    let (first, second, ()) = tokio::join!(first, second, observer);
    first.unwrap();
    second.unwrap();
}

async fn run_read(
    gate: AgentToolExecutionGate,
    entered: Arc<Barrier>,
    active: Arc<AtomicUsize>,
) -> Result<(), AgentToolExecutionGateError> {
    gate.run(AgentToolExecutionClass::ConcurrentRead, async move {
        active.fetch_add(1, Ordering::SeqCst);
        entered.wait().await;
        active.fetch_sub(1, Ordering::SeqCst);
    })
    .await
}

#[tokio::test]
async fn exclusive_work_waits_for_all_reads() {
    let gate = AgentToolExecutionGate::new(3).unwrap();
    let read_entered = Arc::new(Barrier::new(2));
    let release_read = Arc::new(Barrier::new(2));
    let exclusive_entered = Arc::new(AtomicUsize::new(0));

    let read = gate.run(AgentToolExecutionClass::ConcurrentRead, {
        let read_entered = read_entered.clone();
        let release_read = release_read.clone();
        async move {
            read_entered.wait().await;
            release_read.wait().await;
        }
    });
    let exclusive = async {
        read_entered.wait().await;
        let result = gate
            .run(AgentToolExecutionClass::Exclusive, async {
                exclusive_entered.store(1, Ordering::SeqCst);
            })
            .await;
        result
    };
    let release = async {
        tokio::task::yield_now().await;
        assert_eq!(exclusive_entered.load(Ordering::SeqCst), 0);
        release_read.wait().await;
    };

    let (read, exclusive, ()) = tokio::join!(read, exclusive, release);
    read.unwrap();
    exclusive.unwrap();
    assert_eq!(exclusive_entered.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn outputs_are_taken_as_their_futures_complete() {
    let second_completed = Arc::new(Notify::new());
    let first = {
        let second_completed = second_completed.clone();
        Box::pin(async move {
            second_completed.notified().await;
            1usize
        }) as Pin<Box<dyn Future<Output = usize>>>
    };
    let second = Box::pin(async move {
        second_completed.notify_one();
        2usize
    }) as Pin<Box<dyn Future<Output = usize>>>;

    let mut set = CompletionSet::new(vec![first, second]);
    assert_eq!(set.next().await, Some((1, 2)));
    assert_eq!(set.next().await, Some((0, 1)));
    assert_eq!(set.next().await, None);
}

#[tokio::test]
async fn a_dropped_permit_admits_the_next_exclusive_call() {
    let gate = AgentToolExecutionGate::new(2).unwrap();
    let first = gate
        .enter(AgentToolExecutionClass::Exclusive)
        .await
        .unwrap();
    let second = gate.enter(AgentToolExecutionClass::Exclusive);
    tokio::pin!(second);
    assert!(futures_poll_once(second.as_mut()).await.is_none());
    drop(first);
    second.await.unwrap();
}

/// Polls `future` once: its output when ready, `None` otherwise.
async fn futures_poll_once<F: Future + Unpin>(mut future: F) -> Option<F::Output> {
    std::future::poll_fn(|context| match Pin::new(&mut future).poll(context) {
        Poll::Ready(output) => Poll::Ready(Some(output)),
        Poll::Pending => Poll::Ready(None),
    })
    .await
}
