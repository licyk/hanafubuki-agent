use super::*;
use rmcp::model::{ErrorData, ServerCapabilities, ServerConfig};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler, ServiceExt};

struct ListingServer {
    repeat_cursor: bool,
}

impl ListingServer {
    fn page(&self, field: &str, params: Option<PaginatedRequestParams>) -> Value {
        let page = params
            .and_then(|params| params.cursor)
            .map_or(0, |cursor| cursor.parse::<usize>().unwrap());
        let items: Vec<_> = (page * 21..(page + 1) * 21)
            .map(|index| match field {
                "tools" => tool(&format!("tool-{index}")),
                "resources" => json!({"name": format!("resource-{index}"), "uri": format!("mcp://fixture/{index}")}),
                "resourceTemplates" => json!({"name": format!("template-{index}"), "uriTemplate": format!("mcp://fixture/{index}/{{id}}")}),
                _ => unreachable!(),
            })
            .collect();
        let mut result = json!({(field): items});
        if page < 100 {
            result["nextCursor"] = json!(if self.repeat_cursor { 1 } else { page + 1 }.to_string());
        }
        result
    }
}

impl ServerHandler for ListingServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(serde_json::from_value(self.page("tools", request)).unwrap())
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(serde_json::from_value(self.page("resources", request)).unwrap())
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(serde_json::from_value(self.page("resourceTemplates", request)).unwrap())
    }
}

#[tokio::test]
async fn listings_read_all_pages_past_the_former_item_and_page_limits() {
    let (server_io, client_io) = tokio::io::duplex(4096);
    let server = tokio::spawn(async {
        ListingServer {
            repeat_cursor: false,
        }
        .serve(server_io)
        .await
        .unwrap()
    });
    let client = ().serve(client_io).await.unwrap();
    let server = server.await.unwrap();
    let timeout = Duration::from_secs(30);
    let tools = list_all::<ListTools>(client.peer(), timeout, None)
        .await
        .unwrap();
    let resources = list_all::<ListResources>(client.peer(), timeout, None)
        .await
        .unwrap();
    let templates = list_all::<ListResourceTemplates>(client.peer(), timeout, None)
        .await
        .unwrap();
    assert_eq!(tools.items.len(), 2_121);
    assert_eq!(tools.items.last().unwrap().name, "tool-2120");
    assert!(tools.undecodable.is_empty());
    assert_eq!(resources.items.len(), 2_121);
    assert_eq!(resources.items.last().unwrap().uri, "mcp://fixture/2120");
    assert_eq!(templates.items.len(), 2_121);
    assert_eq!(
        templates.items.last().unwrap().uri_template,
        "mcp://fixture/2120/{id}"
    );
    client.cancel().await.unwrap();
    server.cancel().await.unwrap();
}

#[tokio::test]
async fn listings_still_reject_a_repeated_cursor() {
    let (server_io, client_io) = tokio::io::duplex(4096);
    let server = tokio::spawn(async {
        ListingServer {
            repeat_cursor: true,
        }
        .serve(server_io)
        .await
        .unwrap()
    });
    let client = ().serve(client_io).await.unwrap();
    let server = server.await.unwrap();
    let error = list_all::<ListTools>(client.peer(), Duration::from_secs(30), None)
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::Protocol(message) if message.contains("cursor repeated")));
    client.cancel().await.unwrap();
    server.cancel().await.unwrap();
}
