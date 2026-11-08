//! Process owner groups and the ledger of the process groups they leave.
//!
//! A [`ProcessOwnerGroup`] is a capability its creator holds for one
//! lifetime of a subsystem (one Agent instance): every Executor session
//! started through an [`ExecutorManager`](crate::executor::ExecutorManager)
//! scoped to it (`ExecutorManager::owned_by`) belongs to it, and the holder
//! can terminate, confirm and dispose all of them at once
//! (`terminate_owner_group`) when that lifetime ends — the sweep after its
//! own owners stopped. Ownership is the group value itself, never an id,
//! prefix or name.
//!
//! On Unix every Executor child is started in its own session (`setsid`), so
//! a process that crashes leaves its children running, orphaned. A group can
//! carry a [`ProcessGroupRecord`]: a small file listing the process group of
//! each of its sessions with an identity of the group's leader (Linux: the
//! boot id and the leader's start time), written before the session is
//! published and removed when its process ends. The next lifetime calls
//! [`ProcessGroupRecord::reap_leftovers`] first: a recorded group whose leader
//! still runs with the recorded identity is terminated (`SIGTERM`, a bounded
//! grace, `SIGKILL`); one whose identity cannot be confirmed is left alone.
//! Elsewhere nothing is recorded: Windows kills an Executor tree with its job
//! object, and macOS has no identity this ledger reads yet.

mod identity;

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

static NEXT_GROUP: AtomicU64 = AtomicU64::new(1);

/// The capability over every Executor session started in one lifetime of
/// its holder. Clones are the same group.
#[derive(Clone)]
pub struct ProcessOwnerGroup {
    inner: Arc<OwnerGroupInner>,
}

struct OwnerGroupInner {
    id: u64,
    record: Option<ProcessGroupRecord>,
}

impl std::fmt::Debug for ProcessOwnerGroup {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProcessOwnerGroup(..)")
    }
}

impl Default for ProcessOwnerGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessOwnerGroup {
    /// A group whose process groups are not recorded.
    pub fn new() -> Self {
        Self::with_record(None)
    }

    /// A group whose sessions' process groups are recorded in `record`.
    pub fn recorded_in(record: ProcessGroupRecord) -> Self {
        Self::with_record(Some(record))
    }

    fn with_record(record: Option<ProcessGroupRecord>) -> Self {
        Self {
            inner: Arc::new(OwnerGroupInner {
                id: NEXT_GROUP.fetch_add(1, Ordering::Relaxed),
                record,
            }),
        }
    }

    pub(crate) fn same(&self, other: &Self) -> bool {
        self.inner.id == other.inner.id
    }

    pub(crate) fn record(&self) -> Option<&ProcessGroupRecord> {
        self.inner.record.as_ref()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RecordDocument {
    groups: Vec<RecordedGroup>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RecordedGroup {
    pgid: u32,
    /// What the leader must still be to be this group's (`identity`).
    leader_identity: String,
}

/// What [`ProcessGroupRecord::reap_leftovers`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessGroupReap {
    /// The recorded groups found running and terminated.
    pub terminated: Vec<u32>,
    /// Recorded groups whose leader is gone or is now another process: left
    /// alone and forgotten.
    pub unconfirmed: usize,
    /// Groups that did not stop, with why; they stay recorded.
    pub failed: Vec<String>,
}

/// The ledger file of the process groups an owner's sessions run in.
/// Clones share one file and one lock; every change rewrites the file whole
/// (a temporary file renamed over it).
#[derive(Clone)]
pub struct ProcessGroupRecord {
    inner: Arc<RecordInner>,
}

struct RecordInner {
    file: PathBuf,
    /// Serializes the ledger's read-modify-write of its own file.
    gate: Mutex<()>,
}

impl ProcessGroupRecord {
    pub fn new(file: PathBuf) -> Self {
        Self {
            inner: Arc::new(RecordInner {
                file,
                gate: Mutex::new(()),
            }),
        }
    }

    /// Records the group `pgid` leads, when its leader's identity can be
    /// read: `false` when it cannot (it is then never reaped) or the file
    /// could not be written.
    pub(crate) fn add(&self, pgid: u32) -> bool {
        let Some(leader_identity) = identity::leader_identity(pgid) else {
            return false;
        };
        self.change(|document| {
            document.groups.retain(|group| group.pgid != pgid);
            document.groups.push(RecordedGroup {
                pgid,
                leader_identity,
            });
        })
        .map_err(|error| log::warn!("process group {pgid} was not recorded: {error}"))
        .is_ok()
    }

    /// Forgets `pgid`: its process ended.
    pub(crate) fn remove(&self, pgid: u32) {
        if let Err(error) =
            self.change(|document| document.groups.retain(|group| group.pgid != pgid))
        {
            log::warn!("process group {pgid} stays recorded: {error}");
        }
    }

    /// Terminates every recorded group whose leader still runs with the
    /// recorded identity: a process an earlier lifetime started and did not
    /// see end. A group whose identity cannot be confirmed is never
    /// signalled. Blocks for each group's bounded grace.
    pub fn reap_leftovers(&self) -> ProcessGroupReap {
        let _gate = self.inner.gate.lock();
        let mut reap = ProcessGroupReap::default();
        let document = match self.read() {
            Ok(document) => document,
            Err(error) => {
                reap.failed.push(error);
                return reap;
            }
        };
        let mut kept = Vec::new();
        for group in document.groups {
            // A damaged or hand-edited ledger must never make the reap
            // signal init, the kernel's group or this process's own group.
            if is_protected_group(group.pgid) {
                log::warn!(
                    "recorded process group {} is protected and is not signalled",
                    group.pgid
                );
                reap.unconfirmed += 1;
                continue;
            }
            if identity::leader_identity(group.pgid).as_deref()
                != Some(group.leader_identity.as_str())
            {
                reap.unconfirmed += 1;
                continue;
            }
            match terminate(group.pgid) {
                Ok(()) => {
                    log::warn!(
                        "terminated leftover process group {} of an earlier Agent",
                        group.pgid
                    );
                    reap.terminated.push(group.pgid);
                }
                Err(error) => {
                    reap.failed
                        .push(format!("process group {}: {error}", group.pgid));
                    kept.push(group);
                }
            }
        }
        if let Err(error) = self.write(&RecordDocument { groups: kept }) {
            reap.failed.push(error);
        }
        reap
    }

    fn change(&self, edit: impl FnOnce(&mut RecordDocument)) -> Result<(), String> {
        let _gate = self.inner.gate.lock();
        let mut document = self.read()?;
        edit(&mut document);
        self.write(&document)
    }

    /// The recorded groups; a file that does not read records none, since
    /// nothing unconfirmed is ever signalled.
    fn read(&self) -> Result<RecordDocument, String> {
        match fs::read(&self.inner.file) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                log::warn!(
                    "the process group record {} does not read and is replaced: {error}",
                    self.inner.file.display()
                );
                RecordDocument::default()
            })),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(RecordDocument::default())
            }
            Err(error) => Err(format!(
                "read the process group record {}: {error}",
                self.inner.file.display()
            )),
        }
    }

    fn write(&self, document: &RecordDocument) -> Result<(), String> {
        let file = &self.inner.file;
        if document.groups.is_empty() {
            return match fs::remove_file(file) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!("remove {}: {error}", file.display())),
            };
        }
        let bytes = serde_json::to_vec(document).map_err(|error| error.to_string())?;
        let mut temporary = file.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        (|| -> std::io::Result<()> {
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut handle = fs::File::create(&temporary)?;
            handle.write_all(&bytes)?;
            handle.sync_all()?;
            fs::rename(&temporary, file)
        })()
        .map_err(|error| format!("write {}: {error}", file.display()))
    }
}

/// Groups the reap never signals whatever the ledger says: the kernel's
/// (0), init's (1), and this process's own group and id.
#[cfg(unix)]
fn is_protected_group(pgid: u32) -> bool {
    // SAFETY: getpgrp and getpid take no arguments and cannot fail.
    let (own_group, own_pid) = unsafe { (libc::getpgrp(), libc::getpid()) };
    pgid <= 1 || pgid as libc::pid_t == own_group || pgid as libc::pid_t == own_pid
}

#[cfg(not(unix))]
fn is_protected_group(_pgid: u32) -> bool {
    false
}

#[cfg(unix)]
fn terminate(pgid: u32) -> Result<(), String> {
    crate::tree::terminate_group_confirmed(pgid)
}

#[cfg(not(unix))]
fn terminate(_pgid: u32) -> Result<(), String> {
    Err("process groups are not recorded on this platform".to_string())
}

#[cfg(all(test, unix))]
mod tests;
