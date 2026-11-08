//! The standalone Agent's secret store: the operating system's keyring
//! (Keychain, Credential Manager, the Secret Service). A host hands the
//! Agent its own store instead.

use std::sync::Arc;

use hanafubuki_agent_foundation::blocking::{run_blocking_operation, BlockingOperationPort};
use hanafubuki_agent_foundation::secrets::{
    SecretFuture, SecretKey, SecretStore, SecretStoreError,
};

/// The keyring service the standalone Agent's entries are under. Hanafubuki
/// keeps its own under `org.licyk.hanafubuki.secrets`, so the two never
/// share an entry.
pub const KEYRING_SERVICE: &str = "org.licyk.hanafubuki-agent.secrets";

/// Each call runs on the Agent runtime's blocking pool: the platform stores
/// block, and a panic there stays contained.
pub(crate) struct KeyringSecretStore {
    blocking: Arc<dyn BlockingOperationPort>,
}

impl KeyringSecretStore {
    pub(crate) fn shared(blocking: Arc<dyn BlockingOperationPort>) -> Arc<dyn SecretStore> {
        Arc::new(Self { blocking })
    }

    fn run<'a, T: Send + 'static>(
        &'a self,
        label: &'static str,
        key: &SecretKey,
        operation: impl FnOnce(keyring::Entry) -> keyring::Result<T> + Send + 'static,
    ) -> SecretFuture<'a, T> {
        let username = key.as_str().to_string();
        Box::pin(async move {
            run_blocking_operation(self.blocking.as_ref(), label, move || {
                keyring::Entry::new(KEYRING_SERVICE, &username)
                    .and_then(operation)
                    .map_err(|_| SecretStoreError)
            })
            .await
            .map_err(|_| SecretStoreError)?
        })
    }
}

impl SecretStore for KeyringSecretStore {
    fn get<'a>(&'a self, key: &'a SecretKey) -> SecretFuture<'a, Option<Vec<u8>>> {
        self.run("secure credential read worker", key, |entry| {
            match entry.get_secret() {
                Ok(value) => Ok(Some(value)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(error) => Err(error),
            }
        })
    }

    fn set<'a>(&'a self, key: &'a SecretKey, value: Vec<u8>) -> SecretFuture<'a, ()> {
        self.run("secure credential write worker", key, move |entry| {
            entry.set_secret(&value)
        })
    }

    fn delete<'a>(&'a self, key: &'a SecretKey) -> SecretFuture<'a, ()> {
        self.run(
            "secure credential delete worker",
            key,
            |entry| match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(error),
            },
        )
    }
}
