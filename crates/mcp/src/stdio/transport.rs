use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::Transport;
use rmcp::RoleClient;
use std::future::Future;
use std::sync::mpsc;
use tokio::sync::{mpsc as tokio_mpsc, oneshot};

#[derive(Clone, Debug, thiserror::Error)]
pub enum McpExecutorTransportError {
    #[error("MCP stdio transport is closed")]
    Closed,
    #[error("MCP stdio transport queue is full")]
    QueueFull,
    #[error("MCP stdio transport failed: {0}")]
    Failed(String),
}

pub(super) enum WorkerCommand {
    Send {
        message: ClientJsonRpcMessage,
        completion: oneshot::Sender<Result<(), McpExecutorTransportError>>,
    },
    Close {
        completion: oneshot::Sender<Result<(), McpExecutorTransportError>>,
    },
}

pub struct McpExecutorTransport {
    pub(super) commands: mpsc::SyncSender<WorkerCommand>,
    pub(super) inbound: tokio_mpsc::Receiver<ServerJsonRpcMessage>,
    pub(super) closed: bool,
}

impl Transport<RoleClient> for McpExecutorTransport {
    type Error = McpExecutorTransportError;

    fn send(
        &mut self,
        item: ClientJsonRpcMessage,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let commands = self.commands.clone();
        async move {
            let (completion, result) = oneshot::channel();
            commands
                .try_send(WorkerCommand::Send {
                    message: item,
                    completion,
                })
                .map_err(|error| match error {
                    mpsc::TrySendError::Full(_) => McpExecutorTransportError::QueueFull,
                    mpsc::TrySendError::Disconnected(_) => McpExecutorTransportError::Closed,
                })?;
            result
                .await
                .map_err(|_| McpExecutorTransportError::Closed)?
        }
    }

    async fn receive(&mut self) -> Option<ServerJsonRpcMessage> {
        self.inbound.recv().await
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        if self.closed {
            return Box::pin(async { Ok(()) })
                as std::pin::Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send>>;
        }
        self.closed = true;
        let commands = self.commands.clone();
        Box::pin(async move {
            let (completion, result) = oneshot::channel();
            commands
                .try_send(WorkerCommand::Close { completion })
                .map_err(|error| match error {
                    mpsc::TrySendError::Full(_) => McpExecutorTransportError::QueueFull,
                    mpsc::TrySendError::Disconnected(_) => McpExecutorTransportError::Closed,
                })?;
            result
                .await
                .map_err(|_| McpExecutorTransportError::Closed)?
        })
    }
}
