use rmcp::model::{ServerResult, Tool};
use serde_json::{json, Value};

use super::*;

/// A page as rmcp's transport decodes it.
fn server_result(page: Value) -> ServerResult {
    serde_json::from_value(page).unwrap()
}

fn tool(name: &str) -> Value {
    json!({"name": name, "inputSchema": {"type": "object"}})
}

#[test]
fn a_page_with_one_undecodable_tool_keeps_the_others_and_names_the_bad_one() {
    let result = server_result(json!({
        "tools": [
            tool("first"),
            {"name": "bad-schema", "inputSchema": "not an object"},
            {"inputSchema": {"type": "object"}},
            tool("last"),
        ],
        "nextCursor": "page-2",
        "vendorExtension": true,
    }));
    assert!(matches!(result, ServerResult::CustomResult(_)));
    let page = decode_page::<ListTools>(result, 10).unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|tool: &Tool| tool.name.as_ref())
            .collect::<Vec<_>>(),
        ["first", "last"]
    );
    // An entry without a name is named by its position in the listing.
    assert_eq!(page.undecodable, ["bad-schema", "#12"]);
    assert_eq!(page.next_cursor.as_deref(), Some("page-2"));
}

#[test]
fn resource_and_template_pages_skip_only_their_undecodable_entries() {
    let page = decode_page::<ListResources>(
        server_result(json!({
            "resources": [
                {"uri": "mcp://local/doc", "name": "doc"},
                {"name": "no-uri"},
                {"uri": "mcp://local/unnamed", "size": "large"},
            ],
            "nextCursor": "",
        })),
        0,
    )
    .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].uri, "mcp://local/doc");
    assert_eq!(page.undecodable, ["no-uri", "mcp://local/unnamed"]);
    assert_eq!(page.next_cursor, None);

    let page = decode_page::<ListResourceTemplates>(
        server_result(json!({
            "resourceTemplates": [
                {"uriTemplate": "mcp://local/{id}", "name": "by-id"},
                {"name": "no-template"},
            ],
        })),
        0,
    )
    .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.undecodable, ["no-template"]);
}

#[test]
fn a_decodable_page_is_used_as_rmcp_decoded_it() {
    let result = server_result(json!({"tools": [tool("only")], "nextCursor": "next"}));
    assert!(matches!(result, ServerResult::ListToolsResult(_)));
    let page = decode_page::<ListTools>(result, 0).unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(page.undecodable.is_empty());
    assert_eq!(page.next_cursor.as_deref(), Some("next"));
}

/// Only entries are decoded one by one: a page whose entries are not a list,
/// whose other fields break rmcp's rules or that is another method's result
/// still fails as a whole.
#[test]
fn a_page_that_is_not_a_page_still_fails_as_a_whole() {
    let unexpected = McpError::Protocol("MCP server returned an unexpected response type".into());
    for page in [
        json!({"tools": {"name": "not-a-list"}, "extra": 1}),
        json!({"tools": [{"name": "bad", "inputSchema": 1}], "nextCursor": 7}),
        json!({"resources": [{"name": "bad"}]}),
    ] {
        assert_eq!(
            decode_page::<ListTools>(server_result(page), 0).unwrap_err(),
            unexpected
        );
    }
}

/// rmcp's `ServerResult` is untagged and tries `CallToolResult` before its
/// custom fallback; `CallToolResult` accepts any object with `_meta`, so a
/// page carrying `_meta` and an undecodable entry reaches the client as a
/// tool result and its entries are already gone. Such a page still fails as
/// a whole; this pins the rmcp behavior the per-entry decoding cannot reach.
#[test]
fn rmcp_decodes_a_page_with_meta_and_a_bad_entry_as_a_tool_result() {
    let result = server_result(json!({
        "tools": [tool("good"), {"name": "bad", "inputSchema": 1}],
        "_meta": {"vendor": true},
    }));
    assert!(matches!(result, ServerResult::CallToolResult(_)));
    assert!(matches!(
        decode_page::<ListTools>(result, 0),
        Err(McpError::Protocol(_))
    ));
}

#[test]
fn page_bounds_count_undecodable_entries_and_stay_strict() {
    let mut entries = vec![json!({"name": "bad", "inputSchema": 1})];
    entries.extend((0..MAXIMUM_MCP_LIST_ITEMS).map(|index| tool(&format!("tool-{index}"))));
    assert_eq!(
        decode_page::<ListTools>(server_result(json!({ "tools": entries })), 0).unwrap_err(),
        McpError::LimitExceeded
    );
    assert_eq!(
        decode_page::<ListTools>(
            server_result(json!({
                "tools": [
                    {"name": "bad", "inputSchema": 1},
                    {"name": "big", "inputSchema": {"type": "object"},
                     "description": "x".repeat(MAXIMUM_MCP_PAGE_BYTES)},
                ],
            })),
            0
        )
        .unwrap_err(),
        McpError::LimitExceeded
    );
    assert_eq!(
        decode_page::<ListTools>(
            server_result(json!({
                "tools": [{"name": "bad", "inputSchema": 1}],
                "nextCursor": "c".repeat(MAXIMUM_MCP_CURSOR_BYTES + 1),
            })),
            0
        )
        .unwrap_err(),
        McpError::LimitExceeded
    );
}

#[test]
fn validate_page_rejects_unbounded_pages_and_cursors() {
    assert!(validate_page(&[json!({"name": "bounded"})], Some("next")).is_ok());
    assert_eq!(
        validate_page(
            &[json!({"name": "bounded"})],
            Some(&"x".repeat(MAXIMUM_MCP_CURSOR_BYTES + 1))
        )
        .unwrap_err(),
        McpError::LimitExceeded
    );
    assert_eq!(
        validate_page(&vec![json!(null); MAXIMUM_MCP_LIST_ITEMS + 1], None).unwrap_err(),
        McpError::LimitExceeded
    );
    assert_eq!(
        validate_page(
            &[json!({"description": "x".repeat(MAXIMUM_MCP_PAGE_BYTES + 1)})],
            None
        )
        .unwrap_err(),
        McpError::LimitExceeded
    );
}

/// The tool count is bounded once, while the listing is paged: a page that
/// takes it past the bound fails the listing, and an undecodable entry
/// counts like any other.
#[test]
fn a_listing_past_its_bound_fails_while_it_is_paged() {
    let limit = crate::MAX_MCP_TOOLS_PER_SERVER;
    let mut listing = McpListing {
        items: vec![(); limit - 1],
        undecodable: Vec::new(),
    };
    let page = |undecodable: &str, next_cursor: Option<&str>| McpListedPage {
        items: Vec::new(),
        undecodable: vec![undecodable.to_string()],
        next_cursor: next_cursor.map(str::to_string),
    };
    assert_eq!(
        listing.extend(page("#1", Some("next")), limit).unwrap(),
        Some("next".to_string())
    );
    assert_eq!(listing.len(), limit);
    assert_eq!(
        listing.extend(page("#2", None), limit).unwrap_err(),
        McpError::LimitExceeded
    );
}
