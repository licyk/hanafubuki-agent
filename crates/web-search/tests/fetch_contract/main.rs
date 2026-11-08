#![cfg(feature = "test-support")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_web_search::{
    WebFetchBackend, WebFetchError, WebFetchRequest, WebSearchConfig, WebSearchProxy,
    WebSearchService,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

mod bounds;
mod direct;
mod pagination;
mod reader;
mod security;
mod support;

use support::*;
