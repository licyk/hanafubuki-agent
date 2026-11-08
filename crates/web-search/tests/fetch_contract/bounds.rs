use super::*;

#[tokio::test]
async fn response_type_raw_body_and_extracted_output_are_bounded() {
    let server = start_server(|path| match path {
        "/binary" => TestResponse {
            status: 200,
            content_type: "application/octet-stream",
            headers: Vec::new(),
            body: "not readable".to_string(),
        },
        "/oversized" => TestResponse {
            status: 200,
            content_type: "text/plain",
            headers: vec![("Content-Length".to_string(), (9 * 1024 * 1024).to_string())],
            body: String::new(),
        },
        _ => TestResponse {
            status: 200,
            content_type: "text/html",
            headers: Vec::new(),
            body: format!("<main><p>{}</p></main>", "bounded content ".repeat(100)),
        },
    })
    .await;
    let service = WebSearchService::new(fetch_config(&server.base_url)).unwrap();

    for (path, expected) in [
        ("binary", WebFetchError::UnsupportedContentType),
        ("oversized", WebFetchError::ResponseTooLarge),
    ] {
        let error = service
            .fetch_page(
                WebFetchRequest {
                    url: page_url(&format!("{}/{path}", server.base_url)),
                    maximum_content_bytes: 1024,
                    from_offset: 0,
                    content_generation: None,
                },
                CancellationToken::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(error, expected);
    }

    let response = service
        .fetch_page(
            WebFetchRequest {
                url: page_url(&format!("{}/long", server.base_url)),
                maximum_content_bytes: 64,
                from_offset: 0,
                content_generation: None,
            },
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert!(response.truncated);
    assert!(response.content_bytes <= 64);
}
