//! Whether a connection still has its transport. rmcp's service loop ends
//! when the transport's input ends (a stdio server exited, an HTTP worker
//! stopped) or when the client closes it, and then drops the transport; the
//! transport holds the sending half of a channel, so the connection's
//! watcher learns the loop ended, however it ended.

use std::future::Future;

use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use rmcp::RoleClient;
use tokio::sync::watch;

pub(crate) struct McpClosingTransport<T> {
    inner: T,
    closed: Option<watch::Sender<()>>,
}

/// `inner`, and the receiver that sees it close.
pub(crate) fn closing_transport<T>(inner: T) -> (McpClosingTransport<T>, watch::Receiver<()>) {
    let (sender, receiver) = watch::channel(());
    (
        McpClosingTransport {
            inner,
            closed: Some(sender),
        },
        receiver,
    )
}

/// Returns once the transport behind `closed` ended.
pub(crate) async fn transport_closed(mut closed: watch::Receiver<()>) {
    while closed.changed().await.is_ok() {}
}

impl<T: Transport<RoleClient>> Transport<RoleClient> for McpClosingTransport<T> {
    type Error = T::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(item)
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        let item = self.inner.receive().await;
        if item.is_none() {
            // The input ended: say so now, not after rmcp drains and drops us.
            self.closed = None;
        }
        item
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.inner.close()
    }
}
