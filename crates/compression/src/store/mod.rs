// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/content-store.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: the store is changed in place; there is no
// ledger restamp, so `restampContentStore` is not ported; lengths are UTF-16
// code units as the kernel's.

mod arrival;
mod placeholder;
mod retrieve;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::text::utf16_len;

pub use arrival::{store_covered_originals, store_large_results, StoreLargeResults};
pub use placeholder::{
    build_stored_placeholder, classify_kind, extract_command, is_stored_placeholder_text,
    normalize_head, parse_stored_placeholder, restore_stored_placeholder_text, strip_leading_tag,
    ParsedStoredPlaceholder, StoredPlaceholder, STORED_PLACEHOLDER_MARKER,
};
pub use retrieve::{
    apply_retrieve, build_retrieval_pointer, frame_retrieved_original, note_retrieval, Retrieval,
};

/// The originals the content store replaced, by ref: plain data the host
/// persists apart from the [`State`](crate::State), so megabytes of
/// originals are not rewritten with every state save. Identical texts are
/// kept once; a ref's first entry is never replaced or removed, and refs are
/// never reissued, so a ref always retrieves the same original.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContentStore {
    pub version: u32,
    /// The texts by the SHA-256 (lowercase hex) of their UTF-8 bytes.
    pub by_hash: BTreeMap<String, String>,
    pub by_ref: BTreeMap<String, StoredEntry>,
}

impl Default for ContentStore {
    fn default() -> Self {
        Self {
            version: 1,
            by_hash: BTreeMap::new(),
            by_ref: BTreeMap::new(),
        }
    }
}

/// What a ref's original was when it was stored; the placeholder is built
/// from these fields alone, so it never changes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredEntry {
    /// The key of its text in `by_hash`.
    pub hash: String,
    /// The message id it was stored from; it stays valid whatever the ref
    /// map becomes.
    pub raw_id: String,
    /// The placeholder's label (`shell output`, `file read`, ...).
    pub kind: String,
    pub tool_name: Option<String>,
    pub tokens: u64,
    /// In UTF-16 code units.
    pub chars: usize,
    pub head: String,
    /// The call's subject (command, path, query), when it had one.
    pub command: Option<String>,
}

/// An original to store.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoreSpec {
    pub r#ref: String,
    pub raw_id: String,
    pub text: String,
    pub kind: String,
    pub tool_name: Option<String>,
    pub tokens: u64,
    pub head: String,
    pub command: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentStoreStats {
    pub entries: usize,
    pub unique_contents: usize,
    /// In UTF-16 code units.
    pub total_chars: usize,
}

impl ContentStore {
    /// Stores `spec` under its ref unless the ref already has an entry
    /// (first write wins); true when it was stored.
    pub fn store_original(&mut self, spec: StoreSpec) -> bool {
        if spec.r#ref.is_empty() || self.by_ref.contains_key(&spec.r#ref) {
            return false;
        }
        let hash = hash_content(&spec.text);
        let entry = StoredEntry {
            hash: hash.clone(),
            raw_id: spec.raw_id,
            kind: spec.kind,
            tool_name: spec.tool_name,
            tokens: spec.tokens,
            chars: utf16_len(&spec.text),
            head: spec.head,
            command: spec.command,
        };
        self.by_hash.entry(hash).or_insert(spec.text);
        self.by_ref.insert(spec.r#ref, entry);
        true
    }

    /// A ref's entry and original; `None` for a ref never stored, which is
    /// what a ref the model made up gets.
    pub fn retrieve(&self, r#ref: &str) -> Option<(&StoredEntry, &str)> {
        let entry = self.by_ref.get(r#ref)?;
        let text = self.by_hash.get(&entry.hash)?;
        Some((entry, text))
    }

    pub fn has_ref(&self, r#ref: &str) -> bool {
        self.by_ref.contains_key(r#ref)
    }

    pub fn stats(&self) -> ContentStoreStats {
        ContentStoreStats {
            entries: self.by_ref.len(),
            unique_contents: self.by_hash.len(),
            total_chars: self.by_hash.values().map(|text| utf16_len(text)).sum(),
        }
    }
}

/// The SHA-256 of `text`'s UTF-8 bytes, in lowercase hex.
pub fn hash_content(text: &str) -> String {
    let digest: [u8; 32] = Sha256::digest(text.as_bytes()).into();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
