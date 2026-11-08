//! The browser User-Agent the project sends wherever it identifies itself as a
//! browser: the launcher's release lookup and package download, the main
//! application's updater, the managed Python runtime's downloads, and the
//! Agent's web search and page fetching.
//!
//! One string for every such request, defined once here so the two
//! applications and their crates cannot drift apart. Requests that identify
//! the project by name (the launcher's self-update, its VC++ runtime download)
//! keep their own descriptive agents.

/// A current desktop Chrome on Windows.
pub const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
