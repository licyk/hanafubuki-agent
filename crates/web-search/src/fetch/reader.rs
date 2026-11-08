use super::*;

impl WebPageFetcher {
    pub(super) async fn fetch_jina(
        &self,
        target: Url,
        cancellation: &CancellationToken,
    ) -> Result<FetchedDocument, WebFetchError> {
        // Validate the target independently before disclosing it to the fixed
        // reader service. Jina is never used as an SSRF bypass.
        self.resolve_public_target(&target, cancellation).await?;
        self.wait_for_rate_limit(cancellation).await?;
        let jina_url = Url::parse(&format!("{}{target}", self.jina_reader_base))
            .map_err(|_| WebFetchError::InvalidConfiguration("Jina Reader URL is invalid"))?;
        let response = tokio::select! {
            result = self.jina_client.get(jina_url).header(ACCEPT, "text/markdown").send() => result,
            _ = wait_cancelled(cancellation) => return Err(WebFetchError::Cancelled),
        }
        .map_err(map_request_error)?;
        if !response.status().is_success() {
            return Err(WebFetchError::HttpStatus(response.status().as_u16()));
        }
        let body = read_bounded(response, MAX_WEB_FETCH_READER_BODY_BYTES, cancellation).await?;
        let markdown = String::from_utf8_lossy(&body);
        if markdown.trim().is_empty() {
            return Err(WebFetchError::EmptyContent);
        }
        let title = markdown
            .lines()
            .find_map(|line| line.strip_prefix("Title:"))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| truncate_utf8(value, MAX_WEB_SEARCH_TITLE_BYTES).0);
        let (content, document_truncated) =
            truncate_utf8(markdown.trim(), MAX_WEB_FETCH_DOCUMENT_BYTES);
        Ok(FetchedDocument {
            final_url: target.to_string(),
            title,
            content,
            content_type: "text/markdown".to_string(),
            backend: WebFetchBackend::JinaReader,
            fetched_bytes: body.len(),
            generation: String::new(),
            document_truncated,
            redirect_count: 0,
        })
    }
}
