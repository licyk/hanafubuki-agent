//! A package's git download goes through the proxy of the Agent's
//! processes: `git ls-remote` of a remote at a host that does not resolve
//! reaches the proxy the executor's overlay names, asking for the remote's
//! whole URL.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc;

use hanafubuki_agent_foundation::environment::ChildProcessEnvironment;
use hanafubuki_agent_process::executor::ExecutorManager;

use super::remote_revision;

/// The names the Agent's overlay gives the proxy, as git reads them (curl
/// takes `http_proxy` in lower case only); Windows names are not
/// case-sensitive.
fn proxy_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["HTTP_PROXY", "ALL_PROXY"]
    } else {
        &["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
    }
}

#[test]
fn a_remote_is_reached_through_the_processes_proxy() {
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", proxy.local_addr().unwrap());
    let (requested, request_line) = mpsc::channel();
    std::thread::spawn(move || {
        let (stream, _) = proxy.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        requested.send(line).unwrap();
        let mut stream = stream;
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    });
    let overlay = ChildProcessEnvironment::default();
    overlay
        .replace(
            proxy_names()
                .iter()
                .map(|name| (name.to_string(), Some(address.clone())))
                .collect::<HashMap<_, _>>(),
        )
        .unwrap();
    let executor = ExecutorManager::new(overlay);
    let cwd = std::env::temp_dir();

    let listed = remote_revision(&executor, &cwd, "http://packages.invalid/plugin.git", None);

    assert!(listed.is_err(), "the proxy answers 404: {listed:?}");
    let line = request_line
        .recv_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    assert!(
        line.starts_with("GET http://packages.invalid/plugin.git/info/refs"),
        "{line}"
    );
}
