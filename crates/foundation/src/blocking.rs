//! Runtime-neutral port for dispatching blocking application work.

use std::any::Any;
use std::future::Future;
use std::pin::Pin;

pub type BlockingValue = Box<dyn Any + Send>;
pub type BlockingOperation = Box<dyn FnOnce() -> BlockingValue + Send>;
pub type BlockingFuture<'a> =
    Pin<Box<dyn Future<Output = Result<BlockingValue, String>> + Send + 'a>>;

pub trait BlockingOperationPort: Send + Sync {
    fn execute<'a>(
        &'a self,
        label: &'static str,
        operation: BlockingOperation,
    ) -> BlockingFuture<'a>;
}

pub async fn run_blocking_operation<T, F>(
    port: &dyn BlockingOperationPort,
    label: &'static str,
    operation: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    port.execute(label, Box::new(move || Box::new(operation())))
        .await?
        .downcast::<T>()
        .map(|value| *value)
        .map_err(|_| format!("{label} returned an unexpected result type"))
}

#[derive(Clone, Default)]
pub struct InlineBlockingOperationPort;

impl BlockingOperationPort for InlineBlockingOperationPort {
    fn execute<'a>(
        &'a self,
        _label: &'static str,
        operation: BlockingOperation,
    ) -> BlockingFuture<'a> {
        Box::pin(async move { Ok(operation()) })
    }
}
