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

//! The host owns local helper processes; HTTP-only embedders need no runner.
use crate::WebSearchError;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use std::{future::Future, pin::Pin};

#[derive(Clone, Debug)]
pub enum LocalWebRequest {
    Agy {
        query: String,
        maximum: usize,
        model: String,
    },
    Grok {
        query: String,
        maximum: usize,
    },
    Browser {
        url: String,
        proxy: String,
    },
}
pub type LocalWebFuture<'a> =
    Pin<Box<dyn Future<Output = Result<String, WebSearchError>> + Send + 'a>>;
/// Implementations must reserve an Agent-owned process before spawning and
/// confirm disposal on success, failure, cancellation and future drop.
pub trait LocalWebRunner: Send + Sync {
    fn available(&self, kind: LocalWebKind) -> bool;
    fn run(&self, request: LocalWebRequest, cancellation: CancellationToken) -> LocalWebFuture<'_>;
}
#[derive(Clone, Copy)]
pub enum LocalWebKind {
    Agy,
    Grok,
    Browser,
}
