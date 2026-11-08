use std::fmt;

/// A kardia error carried across a `std::error::Error` boundary.
///
/// `kardia_core::Error` is a cloneable handle that does not implement
/// `std::error::Error` itself; plugin and host code wraps it here when it
/// reports a plugin failure through project error types.
#[derive(Clone)]
pub struct PluginError(kardia_core::Error);

impl PluginError {
    pub fn kardia(&self) -> &kardia_core::Error {
        &self.0
    }
}

impl From<kardia_core::Error> for PluginError {
    fn from(error: kardia_core::Error) -> Self {
        Self(error)
    }
}

impl fmt::Display for PluginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.0.as_std(), formatter)
    }
}

impl fmt::Debug for PluginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.0.as_std(), formatter)
    }
}

impl std::error::Error for PluginError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.as_std().source()
    }
}
