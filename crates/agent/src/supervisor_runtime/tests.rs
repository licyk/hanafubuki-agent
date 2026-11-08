use super::*;

#[test]
fn supervisor_runtime_enables_network_io() {
    let runtime = build_agent_supervisor_runtime().unwrap();
    runtime.block_on(async {
        let _ = tokio::net::TcpStream::connect("127.0.0.1:9").await;
    });
}
