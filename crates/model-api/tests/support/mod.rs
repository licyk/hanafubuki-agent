use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedRequest {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Clone)]
pub struct ResponseFragment {
    pub delay: Duration,
    pub bytes: Vec<u8>,
}

impl ResponseFragment {
    pub fn immediate(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            delay: Duration::ZERO,
            bytes: bytes.into(),
        }
    }

    pub fn delayed(delay: Duration, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            delay,
            bytes: bytes.into(),
        }
    }
}

pub struct PlannedResponse {
    pub status: u16,
    pub header_delay: Duration,
    pub headers: Vec<(String, String)>,
    pub fragments: Vec<ResponseFragment>,
}

impl PlannedResponse {
    pub fn sse(fragments: Vec<ResponseFragment>) -> Self {
        Self {
            status: 200,
            header_delay: Duration::ZERO,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            fragments,
        }
    }

    pub fn status(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            header_delay: Duration::ZERO,
            headers: vec![("content-type".into(), "application/json".into())],
            fragments: vec![ResponseFragment::immediate(body)],
        }
    }
}

pub struct FakeHttpServer {
    base_url: String,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    task: JoinHandle<()>,
}

impl FakeHttpServer {
    pub async fn start(plans: Vec<PlannedResponse>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let task = tokio::spawn(async move {
            let mut plans = VecDeque::from(plans);
            while let Some(plan) = plans.pop_front() {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_request(&mut stream).await;
                captured.lock().unwrap().push(request);
                write_response(&mut stream, plan).await;
            }
        });
        Self {
            base_url: format!("http://{address}/v1"),
            requests,
            task,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for FakeHttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer).await.unwrap();
        assert!(count > 0, "client closed before sending request headers");
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(position) = find_subslice(&bytes, b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers_text = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let mut lines = headers_text.split("\r\n");
    let request_line = lines.next().unwrap();
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap().to_string();
    let path = request_parts.next().unwrap().to_string();
    let headers = lines
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (name, value) = line.split_once(':').unwrap();
            (name.to_ascii_lowercase(), value.trim().to_string())
        })
        .collect::<BTreeMap<_, _>>();
    let content_length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>().unwrap())
        .unwrap_or(0);
    while bytes.len() - header_end < content_length {
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer).await.unwrap();
        assert!(count > 0, "client closed before sending request body");
        bytes.extend_from_slice(&buffer[..count]);
    }
    CapturedRequest {
        method,
        path,
        headers,
        body: bytes[header_end..header_end + content_length].to_vec(),
    }
}

async fn write_response(stream: &mut TcpStream, plan: PlannedResponse) {
    if !plan.header_delay.is_zero() {
        tokio::time::sleep(plan.header_delay).await;
    }
    let content_length = plan
        .fragments
        .iter()
        .map(|fragment| fragment.bytes.len())
        .sum::<usize>();
    let reason = match plan.status {
        200 => "OK",
        400 => "Bad Request",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Test Response",
    };
    let mut headers = format!(
        "HTTP/1.1 {} {}\r\ncontent-length: {}\r\nconnection: close\r\n",
        plan.status, reason, content_length
    );
    for (name, value) in plan.headers {
        headers.push_str(&format!("{name}: {value}\r\n"));
    }
    headers.push_str("\r\n");
    if stream.write_all(headers.as_bytes()).await.is_err() {
        return;
    }
    for fragment in plan.fragments {
        if !fragment.delay.is_zero() {
            tokio::time::sleep(fragment.delay).await;
        }
        if stream.write_all(&fragment.bytes).await.is_err() {
            return;
        }
        let _ = stream.flush().await;
    }
    let _ = stream.shutdown().await;
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
