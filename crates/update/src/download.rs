//! The archive's download into the staging directory: streamed, its
//! SHA-256 and byte count taken as it comes, bounded by the size the
//! release gives, failing when nothing arrives for a while. No total time
//! bound: 19 MB at a slow link takes what it takes.

use std::future::Future;
use std::path::Path;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::error::UpdateError;
use crate::release::ReleaseAsset;
use crate::updater::Progress;

/// Where the download goes, and the bounds it keeps.
pub(crate) struct Download<'a> {
    pub(crate) http: &'a reqwest::Client,
    pub(crate) asset: &'a ReleaseAsset,
    pub(crate) sha256: [u8; 32],
    pub(crate) destination: &'a Path,
    /// The executable's directory, for a message.
    pub(crate) directory: &'a str,
    /// How long a download may go without a byte.
    pub(crate) stall: Duration,
    /// How often, at most, the progress is reported.
    pub(crate) progress_every: Duration,
}

impl Download<'_> {
    /// Downloads the archive, then compares its digest
    /// ([`Progress::Verifying`]).
    pub(crate) async fn run(&self, progress: &mut dyn FnMut(Progress)) -> Result<(), UpdateError> {
        let total = self.asset.size;
        progress(Progress::Downloading { done: 0, total });
        let mut response = self
            .stalled(self.http.get(self.asset.url.clone()).send())
            .await?
            .map_err(|error| self.failed(request_failure(&error)))?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(self.failed(format!("the download host answered {status}")));
        }
        let mut file = tokio::fs::File::create(self.destination)
            .await
            .map_err(|error| self.write_failed(&error))?;
        let mut hasher = Sha256::new();
        let mut received: u64 = 0;
        let mut reported = Instant::now();
        loop {
            let chunk = match self.stalled(response.chunk()).await? {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                // The connection ended before its length: what came is short.
                Err(_) => break,
            };
            received += chunk.len() as u64;
            if received > total {
                return Err(UpdateError::Overlong {
                    archive: self.asset.name.clone(),
                });
            }
            hasher.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|error| self.write_failed(&error))?;
            if reported.elapsed() >= self.progress_every {
                reported = Instant::now();
                progress(Progress::Downloading {
                    done: received,
                    total,
                });
            }
        }
        if received < total {
            return Err(UpdateError::Truncated {
                archive: self.asset.name.clone(),
                received,
                expected: total,
            });
        }
        file.sync_all()
            .await
            .map_err(|error| self.write_failed(&error))?;
        progress(Progress::Downloading { done: total, total });
        progress(Progress::Verifying);
        if hasher.finalize().as_slice() != self.sha256 {
            return Err(UpdateError::DigestMismatch {
                archive: self.asset.name.clone(),
            });
        }
        Ok(())
    }

    /// `operation`, failing when it takes longer than the stall bound.
    async fn stalled<T>(&self, operation: impl Future<Output = T>) -> Result<T, UpdateError> {
        tokio::time::timeout(self.stall, operation)
            .await
            .map_err(|_| {
                self.failed(format!(
                    "nothing arrived for {} s",
                    self.stall.as_secs_f64().round()
                ))
            })
    }

    fn failed(&self, reason: String) -> UpdateError {
        UpdateError::Download {
            archive: self.asset.name.clone(),
            reason,
        }
    }

    fn write_failed(&self, error: &std::io::Error) -> UpdateError {
        UpdateError::Write {
            what: "the download",
            directory: self.directory.to_string(),
            reason: error.to_string(),
        }
    }
}

/// Why the request failed, without its URL.
fn request_failure(error: &reqwest::Error) -> String {
    if error.is_redirect() {
        "it was redirected to a URL that is not https, or too many times".to_string()
    } else if error.is_timeout() {
        "timed out".to_string()
    } else {
        "the connection failed".to_string()
    }
}

#[cfg(test)]
mod tests;
