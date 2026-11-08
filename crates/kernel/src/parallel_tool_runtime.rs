use crate::{AgentPlannedTool, AgentToolExecutionClass, AgentToolPlan};
use hanafubuki_agent_model_api::ModelToolCall;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use tokio::sync::{
    Mutex, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, OwnedSemaphorePermit, RwLock, Semaphore,
};

#[derive(Clone)]
pub struct AgentToolExecutionGate {
    permits: Arc<Semaphore>,
    access: Arc<RwLock<()>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentToolExecutionGateError {
    InvalidMaximumInFlight,
    Closed,
}

impl fmt::Display for AgentToolExecutionGateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMaximumInFlight => {
                formatter.write_str("maximum in-flight tool count must be greater than zero")
            }
            Self::Closed => formatter.write_str("tool execution gate was closed"),
        }
    }
}

impl std::error::Error for AgentToolExecutionGateError {}

impl AgentToolExecutionGate {
    pub fn new(maximum_in_flight: usize) -> Result<Self, AgentToolExecutionGateError> {
        if maximum_in_flight == 0 {
            return Err(AgentToolExecutionGateError::InvalidMaximumInFlight);
        }
        Ok(Self {
            permits: Arc::new(Semaphore::new(maximum_in_flight)),
            access: Arc::new(RwLock::new(())),
        })
    }

    /// Admits one call: an in-flight permit, then read access for a
    /// `ConcurrentRead` call or exclusive access otherwise, both held until
    /// the permit is dropped. Both queues are first come, first served, so
    /// calls are admitted in the order they ask.
    pub async fn enter(
        &self,
        class: AgentToolExecutionClass,
    ) -> Result<AgentToolExecutionPermit, AgentToolExecutionGateError> {
        let permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AgentToolExecutionGateError::Closed)?;
        let (read, exclusive) = match class {
            AgentToolExecutionClass::ConcurrentRead => {
                (Some(self.access.clone().read_owned().await), None)
            }
            AgentToolExecutionClass::Exclusive => {
                (None, Some(self.access.clone().write_owned().await))
            }
        };
        Ok(AgentToolExecutionPermit {
            _read: read,
            _exclusive: exclusive,
            _permit: permit,
        })
    }

    pub async fn run<F, T>(
        &self,
        class: AgentToolExecutionClass,
        work: F,
    ) -> Result<T, AgentToolExecutionGateError>
    where
        F: Future<Output = T>,
    {
        let _permit = self.enter(class).await?;
        Ok(work.await)
    }
}

/// A call's admission through the step's gate, released on drop.
pub struct AgentToolExecutionPermit {
    _read: Option<OwnedRwLockReadGuard<()>>,
    _exclusive: Option<OwnedRwLockWriteGuard<()>>,
    _permit: OwnedSemaphorePermit,
}

/// Per-sampling-step runtime for typed response-item tool calls.
///
/// It mirrors Codex's `ToolCallRuntime` boundary while retaining the desktop
/// tool catalog and execution-class gate.
#[derive(Clone)]
pub struct ToolCallRuntime {
    catalog: AgentToolPlan,
    gate: AgentToolExecutionGate,
    approval_prompts: Arc<Mutex<()>>,
}

impl ToolCallRuntime {
    pub fn new(
        catalog: AgentToolPlan,
        maximum_in_flight: usize,
    ) -> Result<Self, AgentToolExecutionGateError> {
        Ok(Self {
            catalog,
            gate: AgentToolExecutionGate::new(maximum_in_flight)?,
            approval_prompts: Arc::new(Mutex::new(())),
        })
    }

    pub fn registration(
        &self,
        call: &ModelToolCall,
    ) -> Result<AgentPlannedTool, crate::AgentToolCatalogError> {
        self.catalog.resolve_call(call).cloned()
    }

    pub async fn dispatch<F, T>(
        &self,
        class: AgentToolExecutionClass,
        work: F,
    ) -> Result<T, AgentToolExecutionGateError>
    where
        F: Future<Output = T>,
    {
        self.gate.run(class, work).await
    }

    pub async fn enter(
        &self,
        class: AgentToolExecutionClass,
    ) -> Result<AgentToolExecutionPermit, AgentToolExecutionGateError> {
        self.gate.enter(class).await
    }

    /// Held while the user is asked about one of the step's calls: the
    /// interface shows one approval prompt at a time.
    pub fn approval_prompts(&self) -> &Mutex<()> {
        &self.approval_prompts
    }

    pub fn catalog_revision(&self) -> crate::AgentToolCatalogRevision {
        self.catalog.revision
    }

    pub fn catalog(&self) -> &AgentToolPlan {
        &self.catalog
    }
}

/// Futures polled together, whose outputs are taken in the order they
/// complete, each with the index it was given at.
pub struct CompletionSet<F: Future> {
    pending: Vec<Option<Pin<Box<F>>>>,
}

impl<F: Future> CompletionSet<F> {
    pub fn new(futures: Vec<F>) -> Self {
        Self {
            pending: futures
                .into_iter()
                .map(|future| Some(Box::pin(future)))
                .collect(),
        }
    }

    /// The next future to complete, with its index; `None` once every one
    /// has.
    pub async fn next(&mut self) -> Option<(usize, F::Output)> {
        std::future::poll_fn(|context| {
            let mut remaining = false;
            for (index, slot) in self.pending.iter_mut().enumerate() {
                let Some(future) = slot.as_mut() else {
                    continue;
                };
                if let Poll::Ready(output) = future.as_mut().poll(context) {
                    *slot = None;
                    return Poll::Ready(Some((index, output)));
                }
                remaining = true;
            }
            if remaining {
                Poll::Pending
            } else {
                Poll::Ready(None)
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests;
