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

use super::*;

pub(super) fn project_direct_document(page: RawPage) -> Result<FetchedDocument, WebFetchError> {
    let fetched_bytes = page.fetched_bytes;
    let is_html = matches!(
        page.content_type.as_str(),
        "text/html" | "application/xhtml+xml"
    );
    let (title, content, document_truncated) = if is_html {
        extract_html(&page.body, &page.final_url, MAX_WEB_FETCH_DOCUMENT_BYTES)?
    } else {
        let text = String::from_utf8_lossy(&page.body);
        let (content, truncated) = truncate_utf8(text.trim(), MAX_WEB_FETCH_DOCUMENT_BYTES);
        if content.is_empty() {
            return Err(WebFetchError::EmptyContent);
        }
        (None, content, truncated)
    };
    Ok(FetchedDocument {
        updated_at: if is_html {
            super::metadata::updated(&page.body)
        } else {
            None
        },
        final_url: page.final_url.to_string(),
        title,
        content,
        content_type: page.content_type,
        backend: if is_html {
            WebFetchBackend::DirectHtml
        } else {
            WebFetchBackend::DirectText
        },
        fetched_bytes,
        generation: String::new(),
        document_truncated,
        redirect_count: page.redirect_count,
    })
}

pub(super) fn finalize_document(
    mut document: FetchedDocument,
    requested_url: &str,
) -> FetchedDocument {
    let mut hasher = Sha256::new();
    hasher.update(requested_url.as_bytes());
    hasher.update([0]);
    hasher.update(document.final_url.as_bytes());
    hasher.update([0]);
    hasher.update(document.content_type.as_bytes());
    hasher.update([0]);
    hasher.update(match document.backend {
        WebFetchBackend::DirectHtml => b"direct_html".as_slice(),
        WebFetchBackend::DirectText => b"direct_text".as_slice(),
        WebFetchBackend::JinaReader => b"jina_reader".as_slice(),
        WebFetchBackend::Exa => b"exa".as_slice(),
        WebFetchBackend::Firecrawl => b"firecrawl".as_slice(),
        WebFetchBackend::Crawl4ai => b"crawl4ai".as_slice(),
        WebFetchBackend::Browser => b"browser".as_slice(),
    });
    hasher.update([0]);
    if let Some(title) = document.title.as_deref() {
        hasher.update(title.as_bytes());
    }
    hasher.update([0, u8::from(document.document_truncated)]);
    if let Some(date) = &document.updated_at {
        hasher.update(date.as_bytes());
    }
    hasher.update(document.content.as_bytes());
    document.generation = hex::encode(hasher.finalize());
    document
}

pub(super) fn page_document(
    document: &FetchedDocument,
    request: &WebFetchRequest,
) -> Result<WebFetchResponse, WebFetchError> {
    if request.from_offset >= document.content.len()
        || !document.content.is_char_boundary(request.from_offset)
    {
        return Err(WebFetchError::InvalidContentOffset);
    }
    let mut end = request
        .from_offset
        .saturating_add(request.maximum_content_bytes)
        .min(document.content.len());
    while end > request.from_offset && !document.content.is_char_boundary(end) {
        end -= 1;
    }
    if end == request.from_offset {
        return Err(WebFetchError::InvalidContentOffset);
    }
    let content = document.content[request.from_offset..end].to_string();
    let next_offset = (end < document.content.len()).then_some(end);
    Ok(WebFetchResponse {
        updated_at: document.updated_at.clone(),
        final_url: document.final_url.clone(),
        title: document.title.clone(),
        content_bytes: content.len(),
        content,
        content_type: document.content_type.clone(),
        backend: document.backend,
        fetched_bytes: document.fetched_bytes,
        content_offset: request.from_offset,
        next_offset,
        total_content_bytes: document.content.len(),
        content_generation: document.generation.clone(),
        document_truncated: document.document_truncated,
        truncated: next_offset.is_some() || document.document_truncated,
        redirect_count: document.redirect_count,
        untrusted_external_content: true,
    })
}
