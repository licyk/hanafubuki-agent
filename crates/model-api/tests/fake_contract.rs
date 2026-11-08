use std::future::Future;
use std::task::{Context, Poll, Waker};

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_model_api::test_support::{
    deterministic_assistant_events, deterministic_user_request, FakeModelExchange,
    FakeModelTransport,
};
use hanafubuki_agent_model_api::ModelPort;

fn ready<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = Box::pin(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("deterministic fake returned a pending future"),
    }
}

#[test]
fn fake_transport_replays_deterministic_events_and_records_request() {
    let request = deterministic_user_request("hello");
    let expected_events = deterministic_assistant_events("world");
    let transport = FakeModelTransport::new([FakeModelExchange::new(
        request.clone(),
        expected_events.clone(),
    )]);

    let mut stream =
        ready(transport.stream(request.clone(), CancellationToken::default())).unwrap();
    let mut observed = Vec::new();
    while let Some(event) = ready(stream.next()) {
        observed.push(event);
    }

    assert_eq!(observed, expected_events);
    assert_eq!(transport.requests(), vec![request]);
    assert_eq!(transport.remaining_exchanges(), 0);
}
