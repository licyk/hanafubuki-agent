//! A scripted model provider in the test process: a loopback HTTP server
//! that answers each request with the next event stream it was given
//! (Responses, or Anthropic Messages for a provider of that protocol), and
//! keeps every request it received. An Anthropic Messages request the API
//! would refuse is answered with its error instead, taking no answer.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone)]
pub struct CapturedRequest {
    pub path: String,
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
            while !responses.is_empty() {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_request(&mut stream).await;
                let refusal = anthropic_refusal(&request);
                captured.lock().unwrap().push(request);
                if let Some(error) = refusal {
                    let response = format!(
                        "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{error}",
                        error.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                    continue;
                }
                let (body, delay) = responses.pop_front().unwrap();
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
    let head = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let path = head.split(' ').nth(1).unwrap_or_default().to_string();
    let headers = head
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
        path,
        headers,
        body: bytes[header_end..header_end + length].to_vec(),
    }
}

/// The Anthropic Messages API's error for a request it refuses: one whose
/// history holds `tool_use` or `tool_result` blocks but that declares no
/// tools.
fn anthropic_refusal(request: &CapturedRequest) -> Option<String> {
    if !request.path.ends_with("/messages") {
        return None;
    }
    let body: serde_json::Value = serde_json::from_slice(&request.body).ok()?;
    let declares_tools = body["tools"]
        .as_array()
        .is_some_and(|tools| !tools.is_empty());
    let has_tool_blocks = body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|message| message["content"].as_array())
        .flatten()
        .any(|block| matches!(block["type"].as_str(), Some("tool_use" | "tool_result")));
    (has_tool_blocks && !declares_tools).then(|| {
        serde_json::json!({
            "type": "error",
            "error": {
                "type": "invalid_request_error",
                "message": "Requests which include `tool_use` or `tool_result` blocks must define tools."
            }
        })
        .to_string()
    })
}

/// An Anthropic Messages stream: one text answer that ends the turn.
pub fn anthropic_assistant_events(text: &str) -> Vec<u8> {
    let text = serde_json::to_string(text).unwrap();
    format!(
        concat!(
            "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg-text\",\"usage\":{{\"input_tokens\":10,\"output_tokens\":1}}}}}}\n\n",
            "event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n",
            "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":{text}}}}}\n\n",
            "event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n",
            "event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"output_tokens\":5}}}}\n\n",
            "event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
        ),
        text = text,
    )
    .into_bytes()
}

/// An Anthropic Messages stream: one call of `name` with `input` (a JSON
/// object).
pub fn anthropic_tool_events(name: &str, input: &str) -> Vec<u8> {
    let input = serde_json::to_string(input).unwrap();
    format!(
        concat!(
            "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg-tool\",\"usage\":{{\"input_tokens\":10,\"output_tokens\":1}}}}}}\n\n",
            "event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"tool_use\",\"id\":\"toolu-1\",\"name\":\"{name}\",\"input\":{{}}}}}}\n\n",
            "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":{input}}}}}\n\n",
            "event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n",
            "event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"tool_use\"}},\"usage\":{{\"output_tokens\":5}}}}\n\n",
            "event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
        ),
        name = name,
        input = input,
    )
    .into_bytes()
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

/// A reasoning item whose summary streams, then a tool call.
pub fn reasoning_tool_events(summary: &str, name: &str, arguments: &str) -> Vec<u8> {
    format!(
        concat!(
            "data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"resp-reasoning-tool\"}}}}\n\n",
            "data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"reasoning\",\"id\":\"reason-tool\",\"summary\":[]}}}}\n\n",
            "data: {{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"reason-tool\",\"summary_index\":0,\"delta\":{summary:?}}}\n\n",
            "data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"reasoning\",\"id\":\"reason-tool\",\"summary\":[{{\"type\":\"summary_text\",\"text\":{summary:?}}}]}}}}\n\n",
            "data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"function_call\",\"id\":\"tool-item\"}}}}\n\n",
            "data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"function_call\",\"id\":\"tool-item\",\"call_id\":\"call-tool\",\"name\":\"{name}\",\"arguments\":{arguments:?}}}}}\n\n",
            "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp-reasoning-tool\",\"end_turn\":false}}}}\n\n"
        ),
        summary = summary,
        name = name,
        arguments = arguments,
    )
    .into_bytes()
}

/// A reasoning item whose summary streams, then the final answer.
pub fn reasoning_answer_events(summary: &str, text: &str) -> Vec<u8> {
    format!(
        concat!(
            "data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"resp-reasoning-final\"}}}}\n\n",
            "data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"reasoning\",\"id\":\"reason-final\",\"summary\":[]}}}}\n\n",
            "data: {{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"reason-final\",\"summary_index\":0,\"delta\":{summary:?}}}\n\n",
            "data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"reasoning\",\"id\":\"reason-final\",\"summary\":[{{\"type\":\"summary_text\",\"text\":{summary:?}}}]}}}}\n\n",
            "data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"message\",\"id\":\"message\",\"phase\":\"final_answer\"}}}}\n\n",
            "data: {{\"type\":\"response.output_text.delta\",\"item_id\":\"message\",\"delta\":{text:?}}}\n\n",
            "data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"message\",\"id\":\"message\",\"phase\":\"final_answer\",\"content\":[{{\"type\":\"output_text\",\"text\":{text:?}}}]}}}}\n\n",
            "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp-reasoning-final\",\"end_turn\":true}}}}\n\n"
        ),
        summary = summary,
        text = text,
    )
    .into_bytes()
}

/// One response calling each `(call id, tool, arguments)` in order.
pub fn tool_calls_events(calls: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut events = String::new();
    let mut event = |value: serde_json::Value| events.push_str(&format!("data: {value}\n\n"));
    event(serde_json::json!({ "type": "response.created", "response": { "id": "resp-calls" } }));
    for (index, (call_id, name, arguments)) in calls.iter().enumerate() {
        let id = format!("call-item-{index}");
        event(serde_json::json!({
            "type": "response.output_item.added",
            "item": { "type": "function_call", "id": id }
        }));
        event(serde_json::json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call", "id": id, "call_id": call_id,
                "name": name, "arguments": arguments
            }
        }));
    }
    event(serde_json::json!({
        "type": "response.completed",
        "response": { "id": "resp-calls", "end_turn": false }
    }));
    events.into_bytes()
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

/// `events` (an answer's) with the provider's count of the request's input tokens.
pub fn with_input_tokens(events: Vec<u8>, input_tokens: u64) -> Vec<u8> {
    String::from_utf8(events)
        .unwrap()
        .replace(
            "\"end_turn\":true}",
            &format!("\"end_turn\":true,\"usage\":{{\"input_tokens\":{input_tokens},\"output_tokens\":1}}}}"),
        )
        .into_bytes()
}
