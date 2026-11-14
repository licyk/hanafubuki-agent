// Ported from dsh-free-search 6ab5168 (MIT); adapted to hanafubuki-agent.
//
// The MIT License (MIT)
//
// Copyright (c) 2026 dsh-free-search contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

//! Persist only expiring cooldowns and hashed key identities. A corrupt store
//! is left untouched and falls back to this process's in-memory state.
use super::{FailureClass, WebSearchProvider as E};
use hanafubuki_agent_foundation::paths::{system_form, ResolvedPath};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};
pub(super) struct CooldownStore(Mutex<Option<Connection>>, String);
impl CooldownStore {
    pub(super) fn new(directory: Option<&Path>, proxy: &crate::WebSearchProxy) -> Self {
        let fingerprint = format!(
            "{proxy:?}:{}",
            [
                "HTTPS_PROXY",
                "https_proxy",
                "HTTP_PROXY",
                "http_proxy",
                "ALL_PROXY",
                "all_proxy",
                "NO_PROXY",
                "no_proxy"
            ]
            .iter()
            .map(|name| std::env::var(name).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("|")
        );
        let connection = directory.and_then(|p| open(p).ok());
        if directory.is_some() && connection.is_none() {
            log::warn!("web cooldown store unavailable; existing file left untouched");
        }
        Self(
            Mutex::new(connection),
            hex::encode(Sha256::digest(fingerprint.as_bytes())),
        )
    }
    fn identity(&self, engine: E, key: &str) -> String {
        let key = if engine == E::Firecrawl && key.is_empty() {
            &self.1
        } else {
            key
        };
        identity(engine, key)
    }
    pub(super) fn get(&self, engine: E, key: &str) -> Option<(Duration, FailureClass)> {
        let guard = self.0.lock();
        let db = guard.as_ref()?;
        let entry: Option<(i64, String)> = db
            .query_row(
                "SELECT until, class FROM cooldowns WHERE id=?1",
                [self.identity(engine, key)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()?;
        let (until, class) = entry?;
        let left = until - chrono::Utc::now().timestamp();
        if left <= 0 {
            return None;
        }
        let class = serde_json::from_str(&class).ok()?;
        Some((Duration::from_secs(left.min(7 * 86400) as u64), class))
    }
    pub(super) fn put(&self, engine: E, key: &str, delay: Duration, class: FailureClass) {
        let guard = self.0.lock();
        let Some(db) = guard.as_ref() else {
            return;
        };
        let Ok(class) = serde_json::to_string(&class) else {
            return;
        };
        let until = chrono::Utc::now().timestamp() + delay.as_secs().min(7 * 86400) as i64;
        if db.execute("INSERT INTO cooldowns(id,until,class) VALUES (?1,?2,?3) ON CONFLICT(id) DO UPDATE SET until=excluded.until,class=excluded.class",params![self.identity(engine,key),until,class]).is_err() {log::warn!("web cooldown persistence failed");}
    }
}
fn identity(engine: E, key: &str) -> String {
    format!(
        "{}:{}",
        engine.id(),
        hex::encode(Sha256::digest(key.as_bytes()))
    )
}
fn open(directory: &Path) -> Result<Connection, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(system_form(directory))?;
    let directory = ResolvedPath::resolve(directory)?;
    let file = ResolvedPath::resolve_child(&directory, std::ffi::OsStr::new("cooldowns.db"))?;
    let db = Connection::open(file.system())?;
    db.busy_timeout(Duration::from_secs(2))?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS cooldowns(id TEXT PRIMARY KEY,until INTEGER NOT NULL,class TEXT NOT NULL)")?;
    Ok(db)
}
