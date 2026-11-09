//! Paged MCP listings (`tools/list`, `resources/list`,
//! `resources/templates/list`), decoded entry by entry.
//!
//! Pages are requested with `Peer::send_request`, not rmcp's typed `list_*`
//! helpers. rmcp decodes a response into the untagged `ServerResult`, so a page
//! with one entry its typed structs reject arrives as
//! `ServerResult::CustomResult` holding the raw page (the fallback
//! `request.rs` relies on for tools/call), and the typed helpers turn that into
//! an error for the whole page. Such a page is decoded here entry by entry: an
//! entry that does not decode is left out and named in
//! [`McpListedPage::undecodable`]. Everything else about a page (its entry
//! array, its cursor and other fields under rmcp's rules, its bounds) still
//! fails the page as a whole.

use std::collections::HashSet;
use std::time::Duration;

use rmcp::model::{
    ClientRequest, ListResourceTemplatesRequest, ListResourceTemplatesResult, ListResourcesRequest,
    ListResourcesResult, ListToolsRequest, ListToolsResult, PaginatedRequestParams,
    RequestMetaObject, Resource, ResourceTemplate, ServerResult, Tool,
};
use rmcp::{Peer, RoleClient};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::McpError;

pub(crate) const MAXIMUM_MCP_LIST_PAGES: usize = 100;
pub(crate) const MAXIMUM_MCP_LIST_ITEMS: usize = 2_048;
pub(crate) const MAXIMUM_MCP_CURSOR_BYTES: usize = 64 * 1024;
pub(crate) const MAXIMUM_MCP_PAGE_BYTES: usize = 4 * 1024 * 1024;

/// What a whole listing decoded.
#[derive(Debug)]
pub(crate) struct McpListing<T> {
    pub(crate) items: Vec<T>,
    /// Entries that did not decode, named as in [`McpListedPage::undecodable`].
    pub(crate) undecodable: Vec<String>,
}

impl<T> McpListing<T> {
    /// Adds one page, failing once the listing holds more than `limit`
    /// entries, undecodable ones included, and returns the page's cursor.
    pub(crate) fn extend(
        &mut self,
        page: McpListedPage<T>,
        limit: usize,
    ) -> Result<Option<String>, McpError> {
        self.items.extend(page.items);
        self.undecodable.extend(page.undecodable);
        if self.len() > limit {
            return Err(McpError::LimitExceeded);
        }
        Ok(page.next_cursor)
    }

    /// Entries listed so far, undecodable ones included.
    pub(crate) fn len(&self) -> usize {
        self.items.len() + self.undecodable.len()
    }
}

/// What one page decoded.
#[derive(Debug)]
pub(crate) struct McpListedPage<T> {
    pub(crate) items: Vec<T>,
    /// Each entry that did not decode, named by its first string field of
    /// [`McpListMethod::NAME_FIELDS`] (bounded to
    /// [`crate::MAX_MCP_CATALOG_ISSUE_NAME_BYTES`]) or, without one, by its
    /// position in the listing (`#3`). Nothing else of the entry is kept.
    pub(crate) undecodable: Vec<String>,
    /// Already through [`end_of_paging`].
    pub(crate) next_cursor: Option<String>,
}

/// One paged list method: its request, its typed page and the page field that
/// holds its entries.
pub(crate) trait McpListMethod {
    type Item: DeserializeOwned + Serialize;
    type Page: DeserializeOwned;
    const ENTRIES: &'static str;
    /// The entry fields that name an undecodable entry, in preference order.
    const NAME_FIELDS: &'static [&'static str];

    fn request(params: Option<PaginatedRequestParams>) -> ClientRequest;
    /// The page, when rmcp decoded the whole result as this method's page.
    #[allow(
        clippy::result_large_err,
        reason = "the error hands the caller back the result it did not consume"
    )]
    fn typed(result: ServerResult) -> Result<Self::Page, ServerResult>;
    fn into_parts(page: Self::Page) -> (Vec<Self::Item>, Option<String>);
}

pub(crate) struct ListTools;
pub(crate) struct ListResources;
pub(crate) struct ListResourceTemplates;

impl McpListMethod for ListTools {
    type Item = Tool;
    type Page = ListToolsResult;
    const ENTRIES: &'static str = "tools";
    const NAME_FIELDS: &'static [&'static str] = &["name"];

    fn request(params: Option<PaginatedRequestParams>) -> ClientRequest {
        ClientRequest::ListToolsRequest(ListToolsRequest {
            method: Default::default(),
            params,
            extensions: Default::default(),
        })
    }

    fn typed(result: ServerResult) -> Result<Self::Page, ServerResult> {
        match result {
            ServerResult::ListToolsResult(page) => Ok(page),
            other => Err(other),
        }
    }

    fn into_parts(page: Self::Page) -> (Vec<Self::Item>, Option<String>) {
        (page.tools, page.next_cursor)
    }
}

impl McpListMethod for ListResources {
    type Item = Resource;
    type Page = ListResourcesResult;
    const ENTRIES: &'static str = "resources";
    const NAME_FIELDS: &'static [&'static str] = &["name", "uri"];

    fn request(params: Option<PaginatedRequestParams>) -> ClientRequest {
        ClientRequest::ListResourcesRequest(ListResourcesRequest {
            method: Default::default(),
            params,
            extensions: Default::default(),
        })
    }

    fn typed(result: ServerResult) -> Result<Self::Page, ServerResult> {
        match result {
            ServerResult::ListResourcesResult(page) => Ok(page),
            other => Err(other),
        }
    }

    fn into_parts(page: Self::Page) -> (Vec<Self::Item>, Option<String>) {
        (page.resources, page.next_cursor)
    }
}

impl McpListMethod for ListResourceTemplates {
    type Item = ResourceTemplate;
    type Page = ListResourceTemplatesResult;
    const ENTRIES: &'static str = "resourceTemplates";
    const NAME_FIELDS: &'static [&'static str] = &["name", "uriTemplate"];

    fn request(params: Option<PaginatedRequestParams>) -> ClientRequest {
        ClientRequest::ListResourceTemplatesRequest(ListResourceTemplatesRequest {
            method: Default::default(),
            params,
            extensions: Default::default(),
        })
    }

    fn typed(result: ServerResult) -> Result<Self::Page, ServerResult> {
        match result {
            ServerResult::ListResourceTemplatesResult(page) => Ok(page),
            other => Err(other),
        }
    }

    fn into_parts(page: Self::Page) -> (Vec<Self::Item>, Option<String>) {
        (page.resource_templates, page.next_cursor)
    }
}

/// Lists every page within `timeout`. The listing holds at most `limit`
/// entries ([`McpListing::extend`]); this is where a server's tool count is
/// bounded, and the catalog built from the listing does not check it again.
pub(crate) async fn list_all<M: McpListMethod>(
    peer: &Peer<RoleClient>,
    timeout: Duration,
    request_meta: Option<&RequestMetaObject>,
    limit: usize,
) -> Result<McpListing<M::Item>, McpError> {
    tokio::time::timeout(timeout, async {
        let mut listing = McpListing {
            items: Vec::new(),
            undecodable: Vec::new(),
        };
        let mut cursor = None;
        let mut seen = HashSet::new();
        for _ in 0..MAXIMUM_MCP_LIST_PAGES {
            let page = request_page::<M>(peer, cursor.take(), request_meta, listing.len()).await?;
            let Some(next) = listing.extend(page, limit)? else {
                return Ok(listing);
            };
            if !seen.insert(next.clone()) {
                return Err(McpError::Protocol("MCP pagination cursor repeated".into()));
            }
            cursor = Some(next);
        }
        Err(McpError::LimitExceeded)
    })
    .await
    .map_err(|_| McpError::Timeout)?
}

/// Lists the one page `cursor` names within `timeout`.
pub(crate) async fn list_page<M: McpListMethod>(
    peer: &Peer<RoleClient>,
    cursor: Option<String>,
    timeout: Duration,
    request_meta: Option<&RequestMetaObject>,
) -> Result<McpListedPage<M::Item>, McpError> {
    tokio::time::timeout(timeout, request_page::<M>(peer, cursor, request_meta, 0))
        .await
        .map_err(|_| McpError::Timeout)?
}

async fn request_page<M: McpListMethod>(
    peer: &Peer<RoleClient>,
    cursor: Option<String>,
    request_meta: Option<&RequestMetaObject>,
    first_position: usize,
) -> Result<McpListedPage<M::Item>, McpError> {
    let result = peer
        .send_request(M::request(page_params(cursor, request_meta)))
        .await
        .map_err(|error| {
            McpError::Protocol(crate::sanitize_mcp_diagnostic(&error.to_string(), 4 * 1024))
        })?;
    decode_page::<M>(result, first_position)
}

/// Decodes one page; `first_position` is the listing position of its first
/// entry, which names an undecodable entry that has no name.
pub(crate) fn decode_page<M: McpListMethod>(
    result: ServerResult,
    first_position: usize,
) -> Result<McpListedPage<M::Item>, McpError> {
    let mut raw = match M::typed(result) {
        Ok(page) => {
            let (items, next_cursor) = M::into_parts(page);
            let next_cursor = end_of_paging(next_cursor);
            validate_page(&items, next_cursor.as_deref())?;
            return Ok(McpListedPage {
                items,
                undecodable: Vec::new(),
                next_cursor,
            });
        }
        Err(ServerResult::CustomResult(raw)) => raw.0,
        Err(_) => return Err(unexpected_response()),
    };
    let entries = match raw.get_mut(M::ENTRIES) {
        Some(Value::Array(entries)) => std::mem::take(entries),
        _ => return Err(unexpected_response()),
    };
    // Without its entries the page must still be a page by rmcp's rules.
    let page = M::Page::deserialize(&raw).map_err(|_| unexpected_response())?;
    let next_cursor = end_of_paging(M::into_parts(page).1);
    validate_page(&entries, next_cursor.as_deref())?;
    let mut items = Vec::with_capacity(entries.len());
    let mut undecodable = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        match M::Item::deserialize(entry) {
            Ok(item) => items.push(item),
            Err(_) => undecodable.push(entry_name(entry, M::NAME_FIELDS, first_position + index)),
        }
    }
    Ok(McpListedPage {
        items,
        undecodable,
        next_cursor,
    })
}

fn entry_name(entry: &Value, fields: &[&str], position: usize) -> String {
    fields
        .iter()
        .find_map(|field| {
            entry
                .get(*field)
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
        })
        .map_or_else(|| format!("#{position}"), crate::catalog::bounded_name)
}

fn unexpected_response() -> McpError {
    McpError::Protocol("MCP server returned an unexpected response type".into())
}

pub(crate) fn page_params(
    cursor: Option<String>,
    request_meta: Option<&RequestMetaObject>,
) -> Option<PaginatedRequestParams> {
    if cursor.is_none() && request_meta.is_none() {
        return None;
    }
    let mut params = PaginatedRequestParams::default().with_cursor(cursor);
    if let Some(request_meta) = request_meta {
        params.meta = Some(request_meta.clone());
    }
    Some(params)
}

/// Servers end paging with an absent cursor or, in practice, an empty one;
/// both mean there is no next page.
pub(crate) fn end_of_paging(next_cursor: Option<String>) -> Option<String> {
    next_cursor.filter(|cursor| !cursor.is_empty())
}

/// Bounds one page by its entries as listed (undecodable ones included);
/// `next_cursor` has already been through [`end_of_paging`].
pub(crate) fn validate_page<T: Serialize>(
    items: &[T],
    next_cursor: Option<&str>,
) -> Result<(), McpError> {
    if items.len() > MAXIMUM_MCP_LIST_ITEMS
        || next_cursor.is_some_and(|cursor| cursor.len() > MAXIMUM_MCP_CURSOR_BYTES)
        || serde_json::to_vec(items)
            .map_err(|_| McpError::Protocol("MCP page is not serializable".into()))?
            .len()
            > MAXIMUM_MCP_PAGE_BYTES
    {
        return Err(McpError::LimitExceeded);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
