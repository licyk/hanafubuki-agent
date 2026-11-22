// Modified from OpenAI Codex cf3ff1530d82 (Apache-2.0).

//! Read-only sidebar files for one conversation, with independent document tabs.

mod document;
mod image;
pub(crate) use image::ImagePlacement;
mod links;

use std::path::{Path, PathBuf};

use hanafubuki_agent::{AgentSessionId, PathKey};
use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyEvent, MouseEvent},
    layout::Rect,
};

use crate::agent_bridge::AgentBridge;
use document::Document;
pub(crate) use document::DocumentContent;
pub(crate) use links::references as history_references;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Location {
    pub(super) line: usize,
    pub(super) column: Option<usize>,
}

#[derive(Clone, Debug)]
pub(crate) enum FileTarget {
    Directory(PathBuf),
    Document {
        path: PathBuf,
        line: Option<Location>,
    },
    Roots,
}

impl PartialEq for FileTarget {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Directory(a), Self::Directory(b)) => PathKey::of(a) == PathKey::of(b),
            (Self::Document { path: a, line: x }, Self::Document { path: b, line: y }) => {
                PathKey::of(a) == PathKey::of(b) && x == y
            }
            (Self::Roots, Self::Roots) => true,
            _ => false,
        }
    }
}
impl Eq for FileTarget {}

impl FileTarget {
    pub(crate) fn document(path: PathBuf) -> Self {
        Self::Document { path, line: None }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileEntry {
    pub(crate) label: String,
    pub(crate) target: FileTarget,
}

impl FileEntry {
    fn document_key(&self) -> Option<PathKey> {
        match &self.target {
            FileTarget::Document { path, .. } => Some(PathKey::of(path)),
            _ => None,
        }
    }
}

pub(crate) enum FileUpdate {
    Inventory {
        request: u64,
        roots: Vec<PathBuf>,
        error: Option<String>,
    },
    References {
        request: u64,
        insert_at: usize,
        entries: Result<Vec<FileEntry>, String>,
    },
    Directory {
        request: u64,
        entries: Result<Vec<FileEntry>, String>,
    },
    Document {
        request: u64,
        result: Result<(PathBuf, DocumentContent), String>,
    },
}

#[derive(Default)]
pub(crate) struct SidebarFiles {
    session: Option<AgentSessionId>,
    roots: Vec<PathBuf>,
    inventory: Vec<FileEntry>,
    folder: Option<(PathBuf, Vec<FileEntry>)>,
    notice: Option<String>,
    documents: Vec<Document>,
    inventory_request: u64,
    history: Option<tokio::sync::watch::Sender<()>>,
    directory_request: u64,
}

impl SidebarFiles {
    pub(crate) fn refresh(&mut self, bridge: &AgentBridge) {
        if self.session.is_some()
            && self.session == bridge.session_id()
            && self
                .history
                .as_ref()
                .is_some_and(|watch| !watch.is_closed())
        {
            self.note_commit();
            if let Some((path, _)) = &self.folder {
                self.directory_request = next_request();
                bridge.sidebar_directory(path.clone(), self.directory_request);
            }
            return;
        }
        if self.session != bridge.session_id() {
            self.documents.clear();
            self.inventory.clear();
            self.folder = None;
            self.roots.clear();
            self.session = bridge.session_id();
        }
        self.inventory_request = next_request();
        self.notice = Some("Loading files…".into());
        self.history = Some(bridge.sidebar_inventory(self.inventory_request));
    }

    pub(crate) fn note_commit(&self) {
        if let Some(history) = &self.history {
            history.send_replace(());
        }
    }

    pub(crate) fn entries(&self) -> &[FileEntry] {
        self.folder
            .as_ref()
            .map_or(&self.inventory, |(_, entries)| entries)
    }

    pub(crate) fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub(crate) fn apply(&mut self, update: FileUpdate) {
        match update {
            FileUpdate::Inventory {
                request,
                roots,
                error,
            } if request == self.inventory_request => {
                self.roots = roots;
                self.notice = error;
                let root_entries = self.roots.iter().map(|root| FileEntry {
                    label: format!("Browse {}/", root.display()),
                    target: FileTarget::Directory(root.clone()),
                });
                self.inventory = root_entries.collect();
            }
            FileUpdate::References {
                request,
                insert_at,
                entries,
            } if request == self.inventory_request => match entries {
                Ok(entries) => {
                    let insert_at = self.roots.len() + insert_at;
                    if insert_at == self.inventory.len() {
                        self.inventory.extend(entries)
                    } else {
                        let new = entries
                            .iter()
                            .filter_map(FileEntry::document_key)
                            .collect::<std::collections::HashSet<_>>();
                        let mut older = self.inventory.split_off(insert_at);
                        older.retain(|entry| {
                            entry.document_key().is_none_or(|key| !new.contains(&key))
                        });
                        self.inventory.extend(entries);
                        self.inventory.extend(older);
                    }
                }
                Err(error) => self.notice = Some(error),
            },
            FileUpdate::Directory { request, entries } if request == self.directory_request => {
                if let Some((_, list)) = &mut self.folder {
                    list.truncate(1);
                    match entries {
                        Ok(entries) => {
                            list.extend(entries);
                            self.notice = None;
                        }
                        Err(error) => self.notice = Some(error),
                    }
                }
            }
            FileUpdate::Document { request, result } => {
                if let Some(doc) = self.documents.iter_mut().find(|doc| doc.request == request) {
                    doc.loaded(result)
                }
            }
            _ => {}
        }
    }

    pub(crate) fn open_paths(&self) -> Vec<PathBuf> {
        self.documents.iter().map(|doc| doc.path.clone()).collect()
    }

    pub(crate) fn open(&mut self, target: FileTarget, bridge: &AgentBridge) -> Option<PathBuf> {
        if self.history.is_none() || self.session != bridge.session_id() {
            self.refresh(bridge);
        }
        match target {
            FileTarget::Roots => {
                self.folder = None;
                self.notice = None;
                None
            }
            FileTarget::Directory(path) => {
                self.directory_request = next_request();
                let back = match path.parent() {
                    Some(parent)
                        if self
                            .roots
                            .iter()
                            .any(|root| PathKey::of(parent).starts_with(&PathKey::of(root))) =>
                    {
                        FileTarget::Directory(parent.to_owned())
                    }
                    _ => FileTarget::Roots,
                };
                self.folder = Some((
                    path.clone(),
                    vec![FileEntry {
                        label: "../".into(),
                        target: back,
                    }],
                ));
                self.notice = Some("Loading directory…".into());
                bridge.sidebar_directory(path, self.directory_request);
                None
            }
            FileTarget::Document { path, line } => {
                let path = if let Some(root) = self.roots.first() {
                    hanafubuki_agent::normalize_path(root.join(&path)).unwrap_or(path)
                } else {
                    path
                };
                if let Some(doc) = self.doc_mut(&path) {
                    if let Some(line) = line {
                        doc.go_to(line);
                    }
                    return Some(doc.path.clone());
                }
                let request = next_request();
                let watch = bridge.sidebar_document(path.clone(), request);
                self.documents.push(Document::new(
                    path.clone(),
                    line,
                    request,
                    watch,
                    bridge.sidebar_sender(),
                ));
                Some(path)
            }
        }
    }

    fn doc_mut(&mut self, path: &Path) -> Option<&mut Document> {
        let key = PathKey::of(path);
        self.documents
            .iter_mut()
            .find(|doc| PathKey::of(&doc.path) == key || doc.canonical_key() == key)
    }
    pub(crate) fn close(&mut self, path: &Path) {
        self.documents
            .retain(|doc| PathKey::of(&doc.path) != PathKey::of(path));
    }
    pub(crate) fn paint(
        &mut self,
        path: &Path,
        area: Rect,
        buf: &mut Buffer,
    ) -> Option<ImagePlacement> {
        self.doc_mut(path)?.paint(area, buf)
    }
    pub(crate) fn key(
        &mut self,
        path: &Path,
        key: KeyEvent,
        bridge: &AgentBridge,
    ) -> Option<PathBuf> {
        let target = self.doc_mut(path)?.key(key)?;
        self.open(target, bridge)
    }
    pub(crate) fn mouse(
        &mut self,
        path: &Path,
        mouse: MouseEvent,
        bridge: &AgentBridge,
    ) -> Option<PathBuf> {
        let target = self.doc_mut(path)?.mouse(mouse)?;
        self.open(target, bridge)
    }
    pub(crate) fn editing_search(&self, path: &Path) -> bool {
        self.documents
            .iter()
            .any(|doc| PathKey::of(&doc.path) == PathKey::of(path) && doc.editing_search)
    }
    pub(crate) fn paste(&mut self, path: &Path, text: &str) {
        if let Some(doc) = self.doc_mut(path).filter(|doc| doc.editing_search) {
            doc.paste(text)
        }
    }
}

fn next_request() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests;
