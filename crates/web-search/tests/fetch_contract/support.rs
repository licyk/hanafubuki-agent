use super::*;

pub(super) fn page_url(url: &str) -> url::Url {
    url::Url::parse(url).unwrap()
}

pub(super) struct TestResponse {
    pub(super) status: u16,
    pub(super) content_type: &'static str,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: String,
}

pub(super) struct TestServer {
    pub(super) base_url: String,
    pub(super) requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) async fn start_server(
    handler: impl Fn(&str) -> TestResponse + Send + Sync + 'static,
) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handler = Arc::new(handler);
    let requests = Arc::new(AtomicUsize::new(0));
    let request_count = requests.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let handler = handler.clone();
            let request_count = request_count.clone();
            tokio::spawn(async move {
                let mut request = vec![0u8; 8192];
                let Ok(read) = stream.read(&mut request).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&request[..read]);
                request_count.fetch_add(1, Ordering::SeqCst);
                let target = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let response = handler(target);
                let reason = if (200..300).contains(&response.status) {
                    "OK"
                } else if (300..400).contains(&response.status) {
                    "Redirect"
                } else {
                    "Error"
                };
                let extra_headers = response
                    .headers
                    .iter()
                    .map(|(name, value)| format!("{name}: {value}\r\n"))
                    .collect::<String>();
                let content_length = if response.headers.iter().any(|(name, _)| {
                    name.eq_ignore_ascii_case("content-length")
                        || name.eq_ignore_ascii_case("transfer-encoding")
                }) {
                    String::new()
                } else {
                    format!("Content-Length: {}\r\n", response.body.len())
                };
                let wire = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: {}\r\n{}{}Connection: close\r\n\r\n{}",
                    response.status,
                    reason,
                    response.content_type,
                    content_length,
                    extra_headers,
                    response.body,
                );
                let _ = stream.write_all(wire.as_bytes()).await;
            });
        }
    });
    TestServer {
        base_url: format!("http://{address}"),
        requests,
        task,
    }
}

pub(super) async fn start_stalling_server() -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let request_count = requests.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let request_count = request_count.clone();
            tokio::spawn(async move {
                let mut request = vec![0u8; 8192];
                if stream.read(&mut request).await.is_ok() {
                    request_count.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            });
        }
    });
    TestServer {
        base_url: format!("http://{address}"),
        requests,
        task,
    }
}

pub(super) fn fetch_config(origin: &str) -> WebSearchConfig {
    let mut config = WebSearchConfig::default().with_test_allowed_private_origin(origin);
    config.proxy = WebSearchProxy::Disabled;
    config.minimum_request_interval = Duration::ZERO;
    config
}
