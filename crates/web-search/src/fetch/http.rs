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

pub(super) fn reader_client(
    timeout: Duration,
    proxy: &WebSearchProxy,
) -> Result<Client, WebFetchError> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(BROWSER_USER_AGENT));
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.8"));
    let builder = Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(5).min(timeout))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none());
    apply_proxy(builder, proxy)
        .map_err(|_| WebFetchError::InvalidConfiguration("proxy URL is invalid"))?
        .build()
        .map_err(|_| {
            WebFetchError::InvalidConfiguration("Jina Reader HTTP client could not be built")
        })
}

pub(super) fn direct_client(
    url: &Url,
    address: Option<SocketAddr>,
    timeout: Duration,
    proxy: &WebSearchProxy,
) -> Result<Client, WebFetchError> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(BROWSER_USER_AGENT));
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(
            "text/html,application/xhtml+xml,text/plain,application/json;q=0.9,*/*;q=0.1",
        ),
    );
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.8"));
    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    let builder = Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(5).min(timeout))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .no_gzip()
        .no_brotli()
        .no_deflate();
    let mut builder = apply_proxy(builder, proxy)
        .map_err(|_| WebFetchError::InvalidConfiguration("proxy URL is invalid"))?;
    // Pinned to the address checked, unless the proxy resolves the name.
    if let Some(address) = address.filter(|_| {
        url.host_str()
            .is_some_and(|host| host.parse::<IpAddr>().is_err())
    }) {
        builder = builder.resolve(url.host_str().unwrap_or_default(), address);
    }
    builder
        .build()
        .map_err(|_| WebFetchError::InvalidConfiguration("direct HTTP client could not be built"))
}

pub(super) fn resolve_redirect(current: &Url, response: &Response) -> Result<Url, WebFetchError> {
    let location = response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or(WebFetchError::RedirectRejected)?;
    if location.len() > MAX_WEB_SEARCH_URL_BYTES {
        return Err(WebFetchError::RedirectRejected);
    }
    let next = current
        .join(location)
        .map_err(|_| WebFetchError::RedirectRejected)?;
    validate_page_url(next.as_str()).map_err(|_| WebFetchError::RedirectRejected)
}

pub(super) fn supported_content_type(response: &Response) -> Result<String, WebFetchError> {
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .ok_or(WebFetchError::UnsupportedContentType)?
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if matches!(
        content_type.as_str(),
        "text/html"
            | "application/xhtml+xml"
            | "text/plain"
            | "text/markdown"
            | "application/json"
            | "application/rdap+json"
    ) {
        Ok(content_type)
    } else {
        Err(WebFetchError::UnsupportedContentType)
    }
}

pub(super) fn ensure_identity_encoding(response: &Response) -> Result<(), WebFetchError> {
    let encoding = response
        .headers()
        .get(CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("identity")
        .trim();
    if encoding.is_empty() || encoding.eq_ignore_ascii_case("identity") {
        Ok(())
    } else {
        Err(WebFetchError::UnsupportedContentEncoding)
    }
}

pub(super) async fn read_bounded(
    response: Response,
    maximum: usize,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, WebFetchError> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum as u64)
    {
        return Err(WebFetchError::ResponseTooLarge);
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    loop {
        let next = tokio::select! {
            next = stream.next() => next,
            _ = wait_cancelled(cancellation) => return Err(WebFetchError::Cancelled),
        };
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(map_request_error)?;
        if body.len().saturating_add(chunk.len()) > maximum {
            return Err(WebFetchError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(super) fn map_request_error(error: reqwest::Error) -> WebFetchError {
    if error.is_timeout() {
        WebFetchError::Timeout
    } else {
        WebFetchError::RequestFailed
    }
}

pub(super) async fn wait_cancelled(cancellation: &CancellationToken) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(CANCELLATION_POLL_INTERVAL).await;
    }
}
