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

//! Disposable document cache. An unreadable database is left untouched.
use hanafubuki_agent_foundation::paths::ResolvedPath;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;

pub(crate) struct DocumentCache {
    connection: Mutex<Option<Connection>>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedPage {
    pub url: String,
    pub title: String,
    pub content: String,
    pub fetched_at: i64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheMatch {
    pub url: String,
    pub title: String,
    pub excerpt: String,
    pub fetched_at: String,
}

impl DocumentCache {
    pub fn new(directory: Option<&Path>) -> Self {
        let override_dir = std::env::var_os("RESEARCH_CACHE_DIR")
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from);
        let directory = override_dir.as_deref().or(directory);
        let connection = directory.and_then(|p| open(p).ok());
        if directory.is_some() && connection.is_none() {
            log::warn!("web document cache could not be opened; existing cache left untouched");
        }
        Self {
            connection: Mutex::new(connection),
        }
    }
    pub fn get(&self, url: &str, max_age_seconds: i64) -> Option<CachedPage> {
        let guard = self.connection.lock();
        let db = guard.as_ref()?;
        db.query_row(
            "SELECT url,title,content,fetched_at FROM pages WHERE url=?1 AND fetched_at>=?2",
            params![url, chrono::Utc::now().timestamp() - max_age_seconds],
            |r| {
                Ok(CachedPage {
                    url: r.get(0)?,
                    title: r.get(1)?,
                    content: r.get(2)?,
                    fetched_at: r.get(3)?,
                })
            },
        )
        .optional()
        .ok()
        .flatten()
    }
    pub fn put(&self, page: &CachedPage) {
        let guard = self.connection.lock();
        let Some(db) = guard.as_ref() else {
            return;
        };
        if db.execute("INSERT INTO pages(url,title,content,fetched_at) VALUES (?1,?2,?3,?4) ON CONFLICT(url) DO UPDATE SET title=excluded.title,content=excluded.content,fetched_at=excluded.fetched_at",params![page.url,page.title,page.content,page.fetched_at]).is_err() {
            log::warn!("web document cache write failed");
        }
    }
    pub fn search(&self, query: &str, limit: usize) -> Vec<CacheMatch> {
        let guard = self.connection.lock();
        let Some(db) = guard.as_ref() else {
            return Vec::new();
        };
        let Ok(mut statement)=db.prepare(r"SELECT url,title,content,fetched_at FROM pages WHERE (title LIKE ?1 ESCAPE '\' OR content LIKE ?1 ESCAPE '\') AND fetched_at>=?2 ORDER BY fetched_at DESC LIMIT ?3") else {return Vec::new();};
        let pattern = format!(
            "%{}%",
            query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let Ok(rows) = statement.query_map(
            params![
                pattern,
                chrono::Utc::now().timestamp() - 7 * 86400,
                limit.clamp(1, 20) as i64
            ],
            |r| {
                Ok(CachedPage {
                    url: r.get(0)?,
                    title: r.get(1)?,
                    content: r.get(2)?,
                    fetched_at: r.get(3)?,
                })
            },
        ) else {
            return Vec::new();
        };
        rows.filter_map(Result::ok)
            .map(|page| {
                let lower = page.content.to_lowercase();
                let at = lower.find(&query.to_lowercase()).unwrap_or(0);
                // Case conversion can change byte lengths; count characters before slicing the original.
                let skip = lower[..at].chars().count().saturating_sub(40);
                let excerpt = page
                    .content
                    .chars()
                    .skip(skip)
                    .take(160)
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                CacheMatch {
                    url: page.url,
                    title: page.title,
                    excerpt: if skip > 0 {
                        format!("…{excerpt}")
                    } else {
                        excerpt
                    },
                    fetched_at: chrono::DateTime::from_timestamp(page.fetched_at, 0)
                        .map(|d| d.to_rfc3339())
                        .unwrap_or_default(),
                }
            })
            .collect()
    }
}
fn open(directory: &Path) -> Result<Connection, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(hanafubuki_agent_foundation::paths::system_form(directory))?;
    let directory = ResolvedPath::resolve(directory)?;
    let file = ResolvedPath::resolve_child(&directory, std::ffi::OsStr::new("research.db"))?;
    let connection = Connection::open(file.system())?;
    connection.busy_timeout(std::time::Duration::from_secs(2))?;
    connection.execute_batch("CREATE TABLE IF NOT EXISTS pages(url TEXT PRIMARY KEY,title TEXT NOT NULL DEFAULT '',content TEXT NOT NULL,fetched_at INTEGER NOT NULL)")?;
    Ok(connection)
}
