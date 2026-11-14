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

//! Owned, bounded local helpers. The tool checks an exact escalation approval
//! before enabling this runner for an invocation.
mod commands;
use hanafubuki_agent_foundation::{
    cancellation::CancellationToken,
    paths::{system_form, ResolvedPath},
};
use hanafubuki_agent_process::{
    executor::{
        manager::{
            dispose_reserved_executor, read_reserved_executor_output, release_executor_reservation,
            reserve_executor_id, start_reserved_executor, ReservedExecutorReceipt,
        },
        EnvironmentInheritance, ExecutorIoMode, ExecutorManager, ExecutorStartRequest,
        OutputStream,
    },
    ownership::ProcessOwnerId,
};
use hanafubuki_agent_web_search::{
    LocalWebFuture, LocalWebKind, LocalWebRequest, LocalWebRunner, WebSearchError,
};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
static NEXT: AtomicU64 = AtomicU64::new(1);

pub(crate) struct OwnedWebRunner {
    pub(crate) executor: ExecutorManager,
}
impl LocalWebRunner for OwnedWebRunner {
    fn available(&self, kind: LocalWebKind) -> bool {
        commands::binary(kind).is_some()
    }
    fn run(&self, request: LocalWebRequest, cancellation: CancellationToken) -> LocalWebFuture<'_> {
        Box::pin(async move {
            if cancellation.is_cancelled() {
                return Err(WebSearchError::Cancelled);
            }
            let name = format!(
                "web-helper-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let directory = std::env::temp_dir().join(&name);
            std::fs::create_dir_all(system_form(&directory))
                .map_err(|_| WebSearchError::RequestFailed)?;
            let directory =
                ResolvedPath::resolve(directory).map_err(|_| WebSearchError::RequestFailed)?;
            let mut guard = RunGuard {
                executor: self.executor.clone(),
                receipt: None,
                directory,
            };
            let (command, timeout_ms) = commands::command(request, &guard.directory)?;
            let reservation =
                reserve_executor_id(&self.executor, &name, &ProcessOwnerId::generate())
                    .map_err(|_| WebSearchError::RequestFailed)?;
            let started = start_reserved_executor(
                &self.executor,
                ExecutorStartRequest {
                    executor_id: Some(name),
                    command,
                    cwd: Some(guard.directory.system().to_path_buf()),
                    environment_inheritance: EnvironmentInheritance::Inherit,
                    env: Default::default(),
                    io_mode: ExecutorIoMode::Pipe,
                    stream_stdin: false,
                    stream_output: true,
                    cols: None,
                    rows: None,
                    timeout_ms: Some(timeout_ms),
                    output_bytes_cap: Some(16 * 1024 * 1024),
                },
                &reservation,
            );
            guard.receipt = Some(match started {
                Ok((_, receipt)) => receipt,
                Err(_) => {
                    release_executor_reservation(&self.executor, &reservation);
                    return Err(WebSearchError::RequestFailed);
                }
            });
            loop {
                if cancellation.is_cancelled() {
                    return Err(WebSearchError::Cancelled);
                }
                let receipt = guard
                    .receipt
                    .as_ref()
                    .ok_or(WebSearchError::RequestFailed)?;
                let output = read_reserved_executor_output(&self.executor, receipt, Some(0), None)
                    .map_err(|_| WebSearchError::RequestFailed)?;
                if output.status.is_terminal() {
                    if output.exit_code != Some(0) {
                        return Err(WebSearchError::RequestFailed);
                    }
                    let bytes: Vec<_> = output
                        .chunks
                        .into_iter()
                        .filter(|c| matches!(c.stream, OutputStream::Stdout))
                        .flat_map(|c| c.data)
                        .collect();
                    let text =
                        String::from_utf8(bytes).map_err(|_| WebSearchError::InvalidResponse)?;
                    guard.finish()?;
                    return Ok(text);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
    }
}
struct RunGuard {
    executor: ExecutorManager,
    receipt: Option<ReservedExecutorReceipt>,
    directory: ResolvedPath,
}
impl RunGuard {
    fn finish(&mut self) -> Result<(), WebSearchError> {
        if let Some(receipt) = &self.receipt {
            dispose_reserved_executor(&self.executor, receipt)
                .map_err(|_| WebSearchError::RequestFailed)?;
            self.receipt = None;
        }
        let _ = std::fs::remove_dir_all(self.directory.system());
        Ok(())
    }
}
impl Drop for RunGuard {
    fn drop(&mut self) {
        if self.finish().is_err() {
            log::warn!(
                "web helper termination not confirmed; ownership retained for Agent shutdown"
            );
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
