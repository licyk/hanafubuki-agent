use hanafubuki_agent_config::{write_atomic, FileAccess};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::ErrorKind, path::PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Adoption {
    Followed,
    NotFollowed,
    Unknown,
}
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Validation {
    Passed,
    Failed,
    NotRun,
    Unknown,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Feedback {
    pub advice_id: String,
    pub adoption: Adoption,
    pub validation_status: Validation,
}

#[derive(Serialize, Deserialize)]
struct Record {
    hash: [u8; 32],
    adoption: Adoption,
    validation: Validation,
    timestamp: u64,
}

#[derive(Default, Serialize)]
pub struct AdvisorStats {
    pub total: usize,
    pub adoption: BTreeMap<Adoption, usize>,
    pub validation: BTreeMap<Validation, usize>,
    pub malformed: usize,
}

pub(super) struct Ledger {
    path: PathBuf,
    lock: Mutex<()>,
}
impl Ledger {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }
    fn read(&self) -> Result<Vec<u8>, String> {
        match std::fs::read(&self.path) {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e.to_string()),
        }
    }
    pub fn record(&self, feedback: Feedback) -> Result<(), String> {
        let _lock = self.lock.lock();
        let mut bytes = self.read()?;
        let entry = Record {
            hash: Sha256::digest(feedback.advice_id.as_bytes()).into(),
            adoption: feedback.adoption,
            validation: feedback.validation_status,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };
        if !bytes.is_empty() && !bytes.ends_with(b"\n") {
            bytes.push(b'\n');
        }
        serde_json::to_writer(&mut bytes, &entry).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        let excess = bytes.len().saturating_sub(1024 * 1024);
        let start = if excess == 0 {
            0
        } else {
            excess
                + bytes[excess..]
                    .iter()
                    .position(|b| *b == b'\n')
                    .unwrap_or(0)
                + 1
        };
        write_atomic(&self.path, &bytes[start..], FileAccess::Private)
    }
    pub fn stats(&self) -> Result<AdvisorStats, String> {
        let mut stats = AdvisorStats::default();
        for line in self
            .read()?
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
        {
            if let Ok(r) = serde_json::from_slice::<Record>(line) {
                stats.total += 1;
                *stats.adoption.entry(r.adoption).or_default() += 1;
                *stats.validation.entry(r.validation).or_default() += 1;
            } else {
                stats.malformed += 1;
            }
        }
        Ok(stats)
    }
}

#[cfg(test)]
mod tests;
