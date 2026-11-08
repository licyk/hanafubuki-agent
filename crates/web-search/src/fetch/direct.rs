use super::*;

impl WebPageFetcher {
    pub(super) async fn fetch_direct(
        &self,
        mut current: Url,
        cancellation: &CancellationToken,
    ) -> Result<RawPage, WebFetchError> {
        for redirect_count in 0..=MAX_WEB_FETCH_REDIRECTS {
            self.wait_for_rate_limit(cancellation).await?;
            let address = self.resolve_public_target(&current, cancellation).await?;
            let client = direct_client(&current, address, self.direct_timeout, &self.proxy)?;
            let response = tokio::select! {
                result = client.get(current.clone()).send() => result,
                _ = wait_cancelled(cancellation) => return Err(WebFetchError::Cancelled),
            }
            .map_err(map_request_error)?;

            if response.status().is_redirection() {
                if redirect_count == MAX_WEB_FETCH_REDIRECTS {
                    return Err(WebFetchError::TooManyRedirects);
                }
                current = resolve_redirect(&current, &response)?;
                continue;
            }
            if !response.status().is_success() {
                return Err(WebFetchError::HttpStatus(response.status().as_u16()));
            }
            let content_type = supported_content_type(&response)?;
            ensure_identity_encoding(&response)?;
            let body =
                read_bounded(response, MAX_WEB_FETCH_DIRECT_BODY_BYTES, cancellation).await?;
            return Ok(RawPage {
                final_url: current,
                content_type,
                body,
                redirect_count,
            });
        }
        Err(WebFetchError::TooManyRedirects)
    }

    pub(super) async fn resolve_public_target(
        &self,
        url: &Url,
        cancellation: &CancellationToken,
    ) -> Result<SocketAddr, WebFetchError> {
        let host = url.host_str().ok_or(WebFetchError::BlockedTarget)?;
        let port = url
            .port_or_known_default()
            .ok_or(WebFetchError::BlockedTarget)?;
        let test_allowed = self
            .test_allowed_private_origins
            .iter()
            .any(|allowed| allowed == &origin(url));
        if !test_allowed && !matches!(port, 80 | 443) {
            return Err(WebFetchError::BlockedTarget);
        }
        if !test_allowed && is_blocked_hostname(host) {
            return Err(WebFetchError::BlockedTarget);
        }
        if let Ok(ip) = host.parse::<IpAddr>() {
            if !test_allowed && !is_public_ip(ip) {
                return Err(WebFetchError::BlockedTarget);
            }
            return Ok(SocketAddr::new(ip, port));
        }
        let addresses = tokio::select! {
            result = lookup_host((host, port)) => result,
            _ = wait_cancelled(cancellation) => return Err(WebFetchError::Cancelled),
        }
        .map_err(|_| WebFetchError::DnsResolutionFailed)?
        .collect::<Vec<_>>();
        if addresses.is_empty() {
            return Err(WebFetchError::DnsResolutionFailed);
        }
        if !test_allowed && addresses.iter().any(|address| !is_public_ip(address.ip())) {
            return Err(WebFetchError::BlockedTarget);
        }
        Ok(addresses[0])
    }

    pub(super) async fn wait_for_rate_limit(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<(), WebFetchError> {
        let delay = {
            let mut next = self.next_request_at.lock().await;
            let now = Instant::now();
            let scheduled = (*next).max(now);
            *next = scheduled + self.request_interval;
            scheduled.saturating_duration_since(now)
        };
        if delay.is_zero() {
            return Ok(());
        }
        tokio::select! {
            _ = tokio::time::sleep(delay) => Ok(()),
            _ = wait_cancelled(cancellation) => Err(WebFetchError::Cancelled),
        }
    }
}
