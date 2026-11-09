//! MCP elicitation: the interaction hub that tracks active tool calls and their
//! pending elicitation requests, and the rmcp client handler that hands server
//! elicitation requests to it.

mod client_handler;
mod validation;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rmcp::model::{ElicitRequestParams, ElicitResult, ElicitationAction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{oneshot, watch};

use crate::{
    McpConnectionGeneration, McpError, McpEventKind, McpEventNotice, McpEventSink, McpServerId,
    NoopMcpEventSink,
};
use validation::{build_request, validate_decision};

pub(crate) use client_handler::McpClientHandler;

pub const MAXIMUM_MCP_ELICITATION_WAIT: Duration = Duration::from_secs(15 * 60);

#[derive(Clone)]
pub(crate) struct ElicitationPauseState {
    active_count: Arc<AtomicUsize>,
    paused: watch::Sender<bool>,
}

impl ElicitationPauseState {
    pub(crate) fn new() -> Self {
        let (paused, _receiver) = watch::channel(false);
        Self {
            active_count: Arc::new(AtomicUsize::new(0)),
            paused,
        }
    }

    pub(crate) fn enter(&self) -> ElicitationPauseGuard {
        if self.active_count.fetch_add(1, Ordering::AcqRel) == 0 {
            self.paused.send_replace(true);
        }
        ElicitationPauseGuard {
            state: self.clone(),
        }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<bool> {
        self.paused.subscribe()
    }
}

pub(crate) struct ElicitationPauseGuard {
    state: ElicitationPauseState,
}

impl Drop for ElicitationPauseGuard {
    fn drop(&mut self) {
        if self.state.active_count.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.state.paused.send_replace(false);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct McpElicitationRequestId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum McpElicitationKind {
    Form {
        schema: Value,
        schema_digest: [u8; 32],
    },
    Url {
        url: String,
        remote_elicitation_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpElicitationRequest {
    pub id: McpElicitationRequestId,
    pub server_id: McpServerId,
    pub connection_generation: McpConnectionGeneration,
    pub session_id: String,
    pub turn_id: u64,
    pub call_id: String,
    pub tool_generation: String,
    pub message: String,
    pub kind: McpElicitationKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum McpElicitationDecision {
    Accept { content: Value },
    Decline,
    Cancel,
}

struct PendingElicitation {
    request: McpElicitationRequest,
    completion: oneshot::Sender<McpElicitationDecision>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpCallContext {
    pub session_id: String,
    pub turn_id: u64,
    pub call_id: String,
    pub tool_generation: String,
}

struct ActiveCall {
    token: u64,
    context: McpCallContext,
    pause_state: ElicitationPauseState,
}

pub struct McpActiveCallGuard {
    hub: McpInteractionHub,
    server_id: McpServerId,
    connection_generation: McpConnectionGeneration,
    token: u64,
    pause_state: ElicitationPauseState,
}

impl Drop for McpActiveCallGuard {
    fn drop(&mut self) {
        self.hub
            .finish_call(&self.server_id, self.connection_generation, self.token);
    }
}

impl McpActiveCallGuard {
    pub(crate) fn pause_state(&self) -> ElicitationPauseState {
        self.pause_state.clone()
    }
}

type ActiveCalls = HashMap<(McpServerId, McpConnectionGeneration), Vec<ActiveCall>>;

#[derive(Clone)]
pub struct McpInteractionHub {
    next_id: Arc<AtomicU64>,
    pending: Arc<Mutex<HashMap<McpElicitationRequestId, PendingElicitation>>>,
    active_calls: Arc<Mutex<ActiveCalls>>,
    events: Arc<dyn McpEventSink>,
}

impl Default for McpInteractionHub {
    fn default() -> Self {
        Self::new(Arc::new(NoopMcpEventSink))
    }
}

impl McpInteractionHub {
    pub fn new(events: Arc<dyn McpEventSink>) -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(0)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            active_calls: Arc::new(Mutex::new(HashMap::new())),
            events,
        }
    }

    pub fn pending(&self) -> Vec<McpElicitationRequest> {
        let mut requests = self
            .pending
            .lock()
            .values()
            .map(|pending| pending.request.clone())
            .collect::<Vec<_>>();
        requests.sort_by_key(|request| request.id.0);
        requests
    }

    pub fn enter_call(
        &self,
        server_id: McpServerId,
        connection_generation: McpConnectionGeneration,
        context: McpCallContext,
    ) -> McpActiveCallGuard {
        let token = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let pause_state = ElicitationPauseState::new();
        self.active_calls
            .lock()
            .entry((server_id.clone(), connection_generation))
            .or_default()
            .push(ActiveCall {
                token,
                context,
                pause_state: pause_state.clone(),
            });
        McpActiveCallGuard {
            hub: self.clone(),
            server_id,
            connection_generation,
            token,
            pause_state,
        }
    }

    fn finish_call(
        &self,
        server_id: &McpServerId,
        connection_generation: McpConnectionGeneration,
        token: u64,
    ) {
        let key = (server_id.clone(), connection_generation);
        let mut active = self.active_calls.lock();
        if let Some(calls) = active.get_mut(&key) {
            calls.retain(|call| call.token != token);
            if calls.is_empty() {
                active.remove(&key);
            }
        }
    }

    pub fn resolve(
        &self,
        id: McpElicitationRequestId,
        decision: McpElicitationDecision,
    ) -> Result<(), McpError> {
        let pending = {
            let mut registry = self.pending.lock();
            let request = registry
                .get(&id)
                .ok_or(McpError::StaleGeneration)?
                .request
                .clone();
            validate_decision(&request, &decision)?;
            registry.remove(&id).ok_or(McpError::StaleGeneration)?
        };
        let server_id = pending.request.server_id.clone();
        let completion = pending
            .completion
            .send(decision)
            .map_err(|_| McpError::Unavailable("MCP elicitation is no longer active".into()));
        self.emit_elicitation(server_id);
        completion
    }

    pub fn cancel_connection(&self, server_id: &McpServerId, generation: McpConnectionGeneration) {
        let mut registry = self.pending.lock();
        let ids = registry
            .iter()
            .filter_map(|(id, pending)| {
                (pending.request.server_id == *server_id
                    && pending.request.connection_generation == generation)
                    .then_some(*id)
            })
            .collect::<Vec<_>>();
        let changed = !ids.is_empty();
        for id in ids {
            if let Some(pending) = registry.remove(&id) {
                let _ = pending.completion.send(McpElicitationDecision::Cancel);
            }
        }
        drop(registry);
        if changed {
            self.emit_elicitation(server_id.clone());
        }
    }

    pub fn cancel_call(
        &self,
        server_id: &McpServerId,
        generation: McpConnectionGeneration,
        context: &McpCallContext,
    ) {
        let mut registry = self.pending.lock();
        let ids = registry
            .iter()
            .filter_map(|(id, pending)| {
                (pending.request.server_id == *server_id
                    && pending.request.connection_generation == generation
                    && pending.request.session_id == context.session_id
                    && pending.request.turn_id == context.turn_id
                    && pending.request.call_id == context.call_id
                    && pending.request.tool_generation == context.tool_generation)
                    .then_some(*id)
            })
            .collect::<Vec<_>>();
        let changed = !ids.is_empty();
        for id in ids {
            if let Some(pending) = registry.remove(&id) {
                let _ = pending.completion.send(McpElicitationDecision::Cancel);
            }
        }
        drop(registry);
        if changed {
            self.emit_elicitation(server_id.clone());
        }
    }

    pub fn cancel_all(&self) {
        let pending = self
            .pending
            .lock()
            .drain()
            .map(|(_, pending)| pending)
            .collect::<Vec<_>>();
        let servers = pending
            .iter()
            .map(|pending| pending.request.server_id.clone())
            .collect::<std::collections::HashSet<_>>();
        for pending in pending {
            let _ = pending.completion.send(McpElicitationDecision::Decline);
        }
        for server_id in servers {
            self.emit_elicitation(server_id);
        }
    }

    async fn request(
        &self,
        server_id: McpServerId,
        connection_generation: McpConnectionGeneration,
        params: ElicitRequestParams,
        timeout: Duration,
    ) -> ElicitResult {
        let (call_context, pause_state) = {
            let active = self.active_calls.lock();
            let Some(calls) = active.get(&(server_id.clone(), connection_generation)) else {
                return ElicitResult::new(ElicitationAction::Decline);
            };
            if calls.len() != 1 {
                return ElicitResult::new(ElicitationAction::Decline);
            }
            (calls[0].context.clone(), calls[0].pause_state.clone())
        };
        let Ok(request) = build_request(
            McpElicitationRequestId(self.next_id.fetch_add(1, Ordering::Relaxed) + 1),
            server_id,
            connection_generation,
            call_context,
            params,
        ) else {
            return ElicitResult::new(ElicitationAction::Decline);
        };
        let (completion, receiver) = oneshot::channel();
        let _pause = pause_state.enter();
        self.pending.lock().insert(
            request.id,
            PendingElicitation {
                request: request.clone(),
                completion,
            },
        );
        self.emit_elicitation(request.server_id.clone());
        let decision = match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(decision)) => decision,
            Ok(Err(_)) => McpElicitationDecision::Cancel,
            Err(_) => {
                self.pending.lock().remove(&request.id);
                self.emit_elicitation(request.server_id.clone());
                McpElicitationDecision::Decline
            }
        };
        match decision {
            McpElicitationDecision::Accept { content } if content.is_null() => {
                ElicitResult::new(ElicitationAction::Accept)
            }
            McpElicitationDecision::Accept { content } => {
                ElicitResult::new(ElicitationAction::Accept).with_content(content)
            }
            McpElicitationDecision::Decline => ElicitResult::new(ElicitationAction::Decline),
            McpElicitationDecision::Cancel => ElicitResult::new(ElicitationAction::Cancel),
        }
    }

    fn emit_elicitation(&self, server_id: McpServerId) {
        self.events.emit(McpEventNotice {
            server_id,
            kind: McpEventKind::Elicitation,
        });
    }
}

#[cfg(test)]
mod tests;
