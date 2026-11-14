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

use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub(super) struct Server {
    pub url: String,
    pub searches: Arc<AtomicUsize>,
    pub pages: Arc<AtomicUsize>,
    pub task: tokio::task::JoinHandle<()>,
}
pub(super) async fn server(auth_failure: bool) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let root = url.clone();
    let searches = Arc::new(AtomicUsize::new(0));
    let pages = Arc::new(AtomicUsize::new(0));
    let search_count = searches.clone();
    let page_count = pages.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let n = socket.read(&mut chunk).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let text = String::from_utf8_lossy(&bytes[..end]);
                    let len = text
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            let request = String::from_utf8(bytes).unwrap();
            let path = request.split_whitespace().nth(1).unwrap();
            let body: Value = request
                .split_once("\r\n\r\n")
                .and_then(|(_, s)| serde_json::from_str(s).ok())
                .unwrap_or(Value::Null);
            let (status, kind, response) = if auth_failure {
                search_count.fetch_add(1, Ordering::Relaxed);
                ("401 Unauthorized", "application/json", "{}".into())
            } else if path == "/search" {
                let n = search_count.fetch_add(1, Ordering::Relaxed);
                let side = if n == 0 { "Compare Alpha" } else { "Beta" };
                ("200 OK","application/json",json!({"results":[{"url":format!("{root}/page/{}",if n==0 {"alpha"} else {"beta"}),"title":side,"content":format!("{side} official evidence"),"published_date":"2026-11-14"}]}).to_string())
            } else if path.starts_with("/page/") {
                page_count.fetch_add(1, Ordering::Relaxed);
                let side = if path.ends_with("alpha") {
                    "Compare Alpha"
                } else {
                    "Beta"
                };
                (
                    "200 OK",
                    "text/plain",
                    format!("{side} provides version 1.0 support and verified features. ")
                        .repeat(20),
                )
            } else if path == "/rerank" {
                let n = body["documents"].as_array().unwrap().len();
                ("200 OK","application/json",json!({"results":(0..n).map(|i|json!({"index":i,"relevance_score":0.9-i as f64*0.01})).collect::<Vec<_>>()} ).to_string())
            } else if path == "/embeddings" {
                let n = body["input"].as_array().unwrap().len();
                ("200 OK","application/json",json!({"data":(0..n).map(|i|json!({"index":i,"embedding":[1.0,i as f64]})).collect::<Vec<_>>()} ).to_string())
            } else if path == "/decision" {
                ("200 OK","application/json",json!({"answers":{"qtype":{"choice":"research"},"breadth":{"score":2},"sufficient":{"noul":0.2},"pick":{"choice":"c1"}}}).to_string())
            } else {
                ("404 Not Found", "application/json", "{}".into())
            };
            let response=format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len());
            if socket.write_all(response.as_bytes()).await.is_err() {
                continue;
            }
        }
    });
    Server {
        url,
        searches,
        pages,
        task,
    }
}
