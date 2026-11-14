// Ported from dsh-free-search 6ab5168 (MIT); adapted to hanafubuki-agent.
//
// The MIT License (MIT)
//
// Copyright (c) 2026 dsh-free-search contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use super::*;
#[tokio::test]
async fn browser_proxy_rejects_private_connect_and_http_before_connecting() {
    let fetcher = WebPageFetcher::new(&WebSearchConfig::default()).unwrap();
    for request in [
        "CONNECT 127.0.0.1:443 HTTP/1.1\r\n\r\n",
        "GET http://169.254.169.254/ HTTP/1.1\r\nHost: 169.254.169.254\r\n\r\n",
        "GET http://[::1]/ HTTP/1.1\r\n\r\n",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (socket, _) = listener.accept().await.unwrap();
        client.write_all(request.as_bytes()).await.unwrap();
        assert!(
            matches!(
                fetcher
                    .browser_connection(socket, &CancellationToken::default())
                    .await,
                Err(WebFetchError::BlockedTarget)
            ),
            "{request}"
        );
    }
}
#[tokio::test]
async fn browser_proxy_forwards_to_the_validated_socket() {
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = format!("http://{}", origin.local_addr().unwrap());
    let config = WebSearchConfig {
        test_allowed_private_origins: vec![target.clone()],
        ..Default::default()
    };
    let fetcher = WebPageFetcher::new(&config).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (socket, _) = listener.accept().await.unwrap();
    client.write_all(format!("GET {target}/article?q=a HTTP/1.1\r\nHost: {}\r\nProxy-Connection: keep-alive\r\n\r\n",origin.local_addr().unwrap()).as_bytes()).await.unwrap();
    let server = async {
        let (mut socket, _) = origin.accept().await.unwrap();
        let mut bytes = [0; 2048];
        let n = socket.read(&mut bytes).await.unwrap();
        let request = String::from_utf8_lossy(&bytes[..n]);
        assert!(request.starts_with("GET /article?q=a HTTP/1.1"));
        assert!(!request.contains("Proxy-Connection"));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    };
    let read = async {
        let mut output = String::new();
        client.read_to_string(&mut output).await.unwrap();
        assert!(output.ends_with("ok"));
    };
    let cancel = CancellationToken::default();
    tokio::time::timeout(Duration::from_secs(2), async {
        let (result, (), ()) =
            tokio::join!(fetcher.browser_connection(socket, &cancel), server, read);
        result.unwrap();
    })
    .await
    .unwrap();
}
