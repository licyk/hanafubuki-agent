use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use parking_lot::Mutex;

pub type SecretFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, SecretStoreError>> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretStoreError;

impl std::fmt::Display for SecretStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("platform secure credential store operation failed")
    }
}

impl std::error::Error for SecretStoreError {}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SecretKey(Arc<str>);

impl SecretKey {
    pub fn new(value: impl Into<String>) -> Result<Self, SecretStoreError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 512
            || value
                .bytes()
                .any(|byte| byte == 0 || byte == b'\r' || byte == b'\n')
        {
            return Err(SecretStoreError);
        }
        Ok(Self(Arc::from(value)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Neutral platform secure-store mechanism. Business domains derive these
/// keys only inside their typed credential authorities; IPC never accepts a
/// `SecretKey` or returns a secret value.
pub trait SecretStore: Send + Sync {
    fn get<'a>(&'a self, key: &'a SecretKey) -> SecretFuture<'a, Option<Vec<u8>>>;
    fn set<'a>(&'a self, key: &'a SecretKey, value: Vec<u8>) -> SecretFuture<'a, ()>;
    fn delete<'a>(&'a self, key: &'a SecretKey) -> SecretFuture<'a, ()>;
}

#[derive(Clone, Default)]
pub struct MemorySecretStore {
    values: Arc<Mutex<BTreeMap<SecretKey, Vec<u8>>>>,
}

impl SecretStore for MemorySecretStore {
    fn get<'a>(&'a self, key: &'a SecretKey) -> SecretFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move { Ok(self.values.lock().get(key).cloned()) })
    }

    fn set<'a>(&'a self, key: &'a SecretKey, value: Vec<u8>) -> SecretFuture<'a, ()> {
        Box::pin(async move {
            self.values.lock().insert(key.clone(), value);
            Ok(())
        })
    }

    fn delete<'a>(&'a self, key: &'a SecretKey) -> SecretFuture<'a, ()> {
        Box::pin(async move {
            self.values.lock().remove(key);
            Ok(())
        })
    }
}
