//! A scripted model provider in the test process: a loopback HTTP server
//! that answers each request with the next Responses event stream it was
//! given, and keeps every request it received.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone)]
pub struct CapturedRequest {
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub struct FakeServer {
    pub base_url: String,
    pub requests: Arc<Mutex<Vec<CapturedRequest>>>,
    pub task: tokio::task::JoinHandle<()>,
}

impl FakeServer {
    pub async fn start(responses: Vec<(Vec<u8>, Duration)>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let task = tokio::spawn(async move {
            let mut responses = VecDeque::from(responses);
            while let Some((body, delay)) = responses.pop_front() {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_request(&mut stream).await;
                captured.lock().unwrap().push(request);
                let headers = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(headers.as_bytes()).await.unwrap();
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                let _ = stream.write_all(&body).await;
                let _ = stream.flush().await;
                let _ = stream.shutdown().await;
            }
        });
        Self {
            base_url: format!("http://{address}/v1"),
            requests,
            task,
        }
    }

    pub fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer).await.unwrap();
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end])
        .unwrap()
        .split("\r\n")
        .skip(1)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (name, value) = line.split_once(':').unwrap();
            (name.to_ascii_lowercase(), value.trim().to_string())
        })
        .collect::<BTreeMap<_, _>>();
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    while bytes.len() - header_end < length {
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer).await.unwrap();
        bytes.extend_from_slice(&buffer[..count]);
    }
    CapturedRequest {
        headers,
        body: bytes[header_end..header_end + length].to_vec(),
    }
}

pub fn tool_events(name: &str, arguments: &str) -> Vec<u8> {
    format!(
        concat!(
            "data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"resp-tool\"}}}}\n\n",
            "data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"function_call\",\"id\":\"tool-item\"}}}}\n\n",
            "data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"function_call\",\"id\":\"tool-item\",\"call_id\":\"call-tool\",\"name\":\"{name}\",\"arguments\":{arguments:?}}}}}\n\n",
            "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp-tool\",\"end_turn\":false}}}}\n\n"
        ),
        name = name,
        arguments = arguments,
    )
    .into_bytes()
}

pub fn namespace_tool_events(namespace: &str, name: &str, arguments: &str) -> Vec<u8> {
    format!(
        concat!(
            "data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"resp-mcp-tool\"}}}}\n\n",
            "data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"function_call\",\"id\":\"mcp-tool-item\",\"namespace\":{namespace:?},\"name\":{name:?}}}}}\n\n",
            "data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"function_call\",\"id\":\"mcp-tool-item\",\"call_id\":\"mcp-call-1\",\"namespace\":{namespace:?},\"name\":{name:?},\"arguments\":{arguments:?}}}}}\n\n",
            "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp-mcp-tool\",\"end_turn\":false}}}}\n\n"
        ),
        namespace = namespace,
        name = name,
        arguments = arguments,
    )
    .into_bytes()
}

pub fn tool_search_events(query: &str) -> Vec<u8> {
    format!(
        concat!(
            "data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"resp-tool-search\"}}}}\n\n",
            "data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"tool_search_call\",\"id\":\"search-item\",\"call_id\":\"search-call-1\",\"execution\":\"client\",\"arguments\":{{\"query\":{query:?}}}}}}}\n\n",
            "data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"tool_search_call\",\"call_id\":\"search-call-1\",\"execution\":\"client\",\"arguments\":{{\"query\":{query:?}}}}}}}\n\n",
            "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp-tool-search\",\"end_turn\":false}}}}\n\n"
        ),
        query = query,
    )
    .into_bytes()
}

pub fn assistant_events(text: &str) -> Vec<u8> {
    format!(
        concat!(
            "data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"resp-final\"}}}}\n\n",
            "data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"message\",\"id\":\"message\",\"phase\":\"final_answer\"}}}}\n\n",
            "data: {{\"type\":\"response.output_text.delta\",\"item_id\":\"message\",\"delta\":{text:?}}}\n\n",
            "data: {{\"type\":\"response.output_text.done\",\"item_id\":\"message\",\"text\":{text:?}}}\n\n",
            "data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"message\",\"id\":\"message\",\"phase\":\"final_answer\",\"content\":[{{\"type\":\"output_text\",\"text\":{text:?}}}]}}}}\n\n",
            "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp-final\",\"end_turn\":true}}}}\n\n"
        ),
        text = text,
    )
    .into_bytes()
}
