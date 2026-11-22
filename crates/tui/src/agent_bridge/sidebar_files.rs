//! Filesystem and paginated transcript reads happen off the UI thread.
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use hanafubuki_agent::{
    AgentHandle, AgentHistorySortDirection, AgentSessionId, AgentTranscriptPageRequest, PathKey,
};
use tokio::sync::watch;

use super::AgentBridge;
use crate::app::WorkspaceEvent;
use crate::app_event::{AppEvent, AppEventSender};
use crate::sidebar_files::{
    history_references, DocumentContent, FileEntry, FileTarget, FileUpdate,
};

fn send(tx: &AppEventSender, update: FileUpdate) {
    tx.send(AppEvent::Workspace(WorkspaceEvent::Files(Box::new(update))));
}

impl AgentBridge {
    pub(crate) fn sidebar_sender(&self) -> AppEventSender {
        self.app_event_tx.untagged()
    }
    pub(crate) fn sidebar_inventory(&self, request: u64) -> watch::Sender<()> {
        let session = self.session_id();
        let tx = self.app_event_tx.untagged();
        let agent = self.agent.clone();
        let (changed, mut changes) = watch::channel(());
        self.agent.runtime().handle().spawn(async move {
            let reader = agent.clone();
            let roots = tokio::task::spawn_blocking(move || match session {
                Some(id) => reader.session_workspace_roots(id),
                None => reader.workspace_roots(),
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|v| v);
            let (roots, error) = match roots {
                Ok(roots) => (roots.into_vec(), None),
                Err(error) => (vec![], Some(error)),
            };
            let root = roots.first().cloned();
            send(
                &tx,
                FileUpdate::Inventory {
                    request,
                    roots,
                    error,
                },
            );
            let (Some(id), Some(root)) = (session, root) else {
                return;
            };
            let mut newest = None;
            loop {
                let mut seen = HashSet::<PathKey>::new();
                let mut insert_at = 0;
                let mut cursor = None;
                let mut through_revision = None;
                let mut head = None;
                loop {
                    if changes.has_changed().is_err() {
                        return;
                    }
                    let reader = agent.clone();
                    let query = AgentTranscriptPageRequest {
                        session_id: id,
                        page_size: super::commands::REPLAYED_ITEMS,
                        cursor,
                        direction: AgentHistorySortDirection::Descending,
                        through_revision,
                    };
                    let result =
                        tokio::task::spawn_blocking(move || reader.read_transcript_page(query))
                            .await;
                    let page = match result
                        .map_err(|e| e.to_string())
                        .and_then(|r| r.map_err(|e| e.to_string()))
                    {
                        Ok(page) => page,
                        Err(error) => {
                            send(
                                &tx,
                                FileUpdate::References {
                                    request,
                                    insert_at,
                                    entries: Err(error),
                                },
                            );
                            break;
                        }
                    };
                    let mut reached_head = false;
                    let mut entries = Vec::new();
                    for item in page.items {
                        let key = (item.source_session_id, item.item_id, item.item_revision);
                        head.get_or_insert(key);
                        if Some(key) == newest {
                            reached_head = true;
                            break;
                        }
                        history_references(item.content, &root, &mut entries, &mut seen, |patch| {
                            agent.preview_patch(patch)
                        });
                    }
                    if !entries.is_empty() {
                        let count = entries.len();
                        send(
                            &tx,
                            FileUpdate::References {
                                request,
                                insert_at,
                                entries: Ok(entries),
                            },
                        );
                        insert_at += count;
                    }
                    cursor = page.older_cursor;
                    if reached_head || cursor.is_none() {
                        newest = head;
                        break;
                    }
                    through_revision = Some(page.through_revision);
                }
                if changes.changed().await.is_err() {
                    return;
                }
            }
        });
        changed
    }

    pub(crate) fn sidebar_directory(&self, path: PathBuf, request: u64) {
        let tx = self.app_event_tx.untagged();
        let agent = self.agent.clone();
        let session = self.session_id();
        self.agent.runtime().handle().spawn_blocking(move || {
            let entries = agent
                .browse_workspace_directory(session, &path)
                .map(|entries| {
                    entries
                        .into_iter()
                        .map(|entry| {
                            let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
                            if entry.directory {
                                FileEntry {
                                    label: format!("{name}/"),
                                    target: FileTarget::Directory(entry.path),
                                }
                            } else {
                                FileEntry {
                                    label: name.into_owned(),
                                    target: FileTarget::document(entry.path),
                                }
                            }
                        })
                        .collect()
                });
            send(&tx, FileUpdate::Directory { request, entries });
        });
    }

    pub(crate) fn sidebar_document(&self, path: PathBuf, request: u64) -> tokio::task::AbortHandle {
        let tx = self.app_event_tx.untagged();
        let session = self.session_id();
        let agent = self.agent.clone();
        self.agent
            .runtime()
            .handle()
            .spawn(async move {
                let (changed, mut changes) = watch::channel(());
                let watch_agent = agent.clone();
                let watch_path = path.clone();
                let watch = tokio::task::spawn_blocking(move || {
                    watch_agent.watch_workspace_document(session, &watch_path, move || {
                        changed.send_replace(());
                    })
                })
                .await
                .ok()
                .and_then(Result::ok);
                let mut previous = None;
                loop {
                    let reader = agent.clone();
                    let path = path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        read_document(&reader, session, path, previous)
                    })
                    .await;
                    let result = match result {
                        Ok(Ok((fingerprint, Some(content)))) => {
                            previous = Some(fingerprint);
                            Ok(content)
                        }
                        Ok(Ok((_, None))) => {
                            wait_for_change(&mut changes, watch.is_some()).await;
                            continue;
                        }
                        Ok(Err(e)) => {
                            previous = None;
                            Err(e)
                        }
                        Err(e) => Err(e.to_string()),
                    };
                    send(&tx, FileUpdate::Document { request, result });
                    wait_for_change(&mut changes, watch.is_some()).await;
                }
            })
            .abort_handle()
    }
}

async fn wait_for_change(changes: &mut watch::Receiver<()>, watching: bool) {
    if watching {
        let _ = changes.changed().await;
    } else {
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

type DocumentRead = (u64, Option<(PathBuf, DocumentContent)>);

fn read_document(
    agent: &AgentHandle,
    session: Option<AgentSessionId>,
    path: PathBuf,
    previous: Option<u64>,
) -> Result<DocumentRead, String> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let file = agent.read_workspace_document(
        session,
        &path,
        hanafubuki_agent::MAXIMUM_SOURCE_IMAGE_BYTES,
    )?;
    let mut hash = DefaultHasher::new();
    file.bytes.hash(&mut hash);
    let fingerprint = hash.finish();
    if previous == Some(fingerprint) {
        return Ok((fingerprint, None));
    }
    let content = DocumentContent::decode(file.bytes, &file.path)?;
    Ok((fingerprint, Some((file.path, content))))
}
