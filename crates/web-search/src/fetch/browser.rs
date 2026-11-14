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

//! Chromium's HTTP and CONNECT traffic uses the same public-target policy as
//! direct fetching, including redirects and subresources. Connections pin the
//! address that was checked; the browser cannot resolve it a second time.
use super::*;
use crate::{LocalWebKind, LocalWebRequest, LocalWebRunner};
use futures_util::stream::FuturesUnordered;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

impl WebPageFetcher {
    pub(super) async fn fetch_browser(
        &self,
        target: Url,
        runner: &dyn LocalWebRunner,
        cancel: &CancellationToken,
    ) -> Result<FetchedDocument, WebFetchError> {
        self.resolve_public_target(&target, cancel).await?;
        // This checked forwarding proxy connects to pinned addresses directly.
        // Never bypass a host-selected upstream proxy; remote readers remain available.
        if !matches!(self.proxy, WebSearchProxy::Disabled)
            || !self.options.browser_enabled
            || !runner.available(LocalWebKind::Browser)
        {
            return Err(WebFetchError::RequestFailed);
        }
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| WebFetchError::RequestFailed)?;
        let proxy = format!(
            "http://{}",
            listener
                .local_addr()
                .map_err(|_| WebFetchError::RequestFailed)?
        );
        let run = runner.run(
            LocalWebRequest::Browser {
                url: target.to_string(),
                proxy,
            },
            cancel.clone(),
        );
        let serving = self.serve_browser(listener, cancel);
        let html = tokio::select! {
            result=run=>result.map_err(|error|if matches!(error,crate::WebSearchError::Cancelled) {WebFetchError::Cancelled} else {WebFetchError::RequestFailed})?,
            _=serving=>return Err(WebFetchError::RequestFailed),
            _=wait_cancelled(cancel)=>return Err(WebFetchError::Cancelled),
        };
        let mut document = project_direct_document(RawPage {
            fetched_bytes: html.len(),
            final_url: target,
            content_type: "text/html".into(),
            body: html.into_bytes(),
            redirect_count: 0,
        })?;
        if document.content.chars().count() < 300 {
            return Err(WebFetchError::EmptyContent);
        }
        document.backend = WebFetchBackend::Browser;
        Ok(document)
    }
    async fn serve_browser(&self, listener: TcpListener, cancel: &CancellationToken) {
        let mut connections = FuturesUnordered::new();
        loop {
            tokio::select! {
                accepted=listener.accept(),if connections.len()<32=>match accepted {
                    Ok((socket,_))=>connections.push(tokio::time::timeout(Duration::from_secs(25),self.browser_connection(socket,cancel))),
                    Err(_)=>return,
                },
                _=connections.next(),if !connections.is_empty()=>{},
                _=wait_cancelled(cancel)=>return,
            }
        }
    }
    async fn browser_connection(
        &self,
        mut downstream: TcpStream,
        cancel: &CancellationToken,
    ) -> Result<(), WebFetchError> {
        let mut head = Vec::new();
        let mut byte = [0];
        while !head.ends_with(b"\r\n\r\n") {
            if head.len() >= 16384 {
                return Err(WebFetchError::ResponseTooLarge);
            }
            downstream
                .read_exact(&mut byte)
                .await
                .map_err(|_| WebFetchError::RequestFailed)?;
            head.push(byte[0]);
        }
        let text = std::str::from_utf8(&head).map_err(|_| WebFetchError::BlockedTarget)?;
        let mut lines = text.split("\r\n");
        let first = lines.next().ok_or(WebFetchError::BlockedTarget)?;
        let parts: Vec<_> = first.split_whitespace().collect();
        let [method, address, _] = parts.as_slice() else {
            return Err(WebFetchError::BlockedTarget);
        };
        let connect = *method == "CONNECT";
        if !connect && *method != "GET" && *method != "HEAD" {
            return Err(WebFetchError::BlockedTarget);
        }
        let url = validate_page_url(&if connect {
            format!("https://{address}/")
        } else {
            address.to_string()
        })?;
        let address = self
            .resolve_public_target(&url, cancel)
            .await?
            .ok_or(WebFetchError::DnsResolutionFailed)?;
        let mut upstream = TcpStream::connect(address)
            .await
            .map_err(|_| WebFetchError::RequestFailed)?;
        if connect {
            downstream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .map_err(|_| WebFetchError::RequestFailed)?;
        } else {
            let mut header = format!(
                "{method} {} HTTP/1.1\r\n",
                &url[url::Position::BeforePath..url::Position::AfterQuery]
            );
            for line in lines.filter(|line| !line.is_empty()) {
                let lower = line.to_ascii_lowercase();
                if !lower.starts_with("proxy-") && !lower.starts_with("connection:") {
                    header.push_str(line);
                    header.push_str("\r\n");
                }
            }
            header.push_str("Connection: close\r\n\r\n");
            upstream
                .write_all(header.as_bytes())
                .await
                .map_err(|_| WebFetchError::RequestFailed)?;
        }
        if connect {
            tokio::io::copy_bidirectional(&mut downstream, &mut upstream)
                .await
                .map_err(|_| WebFetchError::RequestFailed)?;
        } else {
            tokio::io::copy(&mut upstream, &mut downstream)
                .await
                .map_err(|_| WebFetchError::RequestFailed)?;
            downstream
                .shutdown()
                .await
                .map_err(|_| WebFetchError::RequestFailed)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
