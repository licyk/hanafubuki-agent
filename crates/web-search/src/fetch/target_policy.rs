use crate::model::{WebFetchError, MAX_WEB_SEARCH_URL_BYTES};
use hanafubuki_agent_foundation::url_policy::{UrlPolicy, UrlRejection};
use reqwest::Url;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub(super) fn validate_fixed_endpoint(value: &str) -> Result<Url, WebFetchError> {
    let mut url = validate_page_url(value)?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(WebFetchError::InvalidConfiguration(
            "fixed reader endpoint must not contain a query or fragment",
        ));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

pub(super) fn validate_page_url(value: &str) -> Result<Url, WebFetchError> {
    let mut url = UrlPolicy::PAGE
        .within(MAX_WEB_SEARCH_URL_BYTES)
        .parse(value)
        .map_err(|rejection| {
            WebFetchError::InvalidRequest(match rejection {
                UrlRejection::TooLong { .. } => "URL exceeds 2048 bytes",
                UrlRejection::Invalid(_) => "URL is invalid",
                _ => "URL must be HTTP(S), include a host, and contain no credentials",
            })
        })?;
    url.set_fragment(None);
    Ok(url)
}

pub(super) fn origin(url: &Url) -> String {
    format!(
        "{}://{}:{}",
        url.scheme(),
        url.host_str().unwrap_or_default().to_ascii_lowercase(),
        url.port_or_known_default().unwrap_or_default()
    )
}

pub(super) fn is_blocked_hostname(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".home.arpa")
}

pub(super) fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 198 && matches!(b, 18 | 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    // Only ordinary global unicast (2000::/3). Exclude documentation,
    // benchmarking/ORCHID and 6to4, whose embedded IPv4 target can otherwise
    // reintroduce private-address routing after the IPv6 check.
    (segments[0] & 0xe000) == 0x2000
        && !(segments[0] == 0x2001
            && (segments[1] == 0x0002
                || (0x0010..=0x002f).contains(&segments[1])
                || segments[1] == 0x0db8))
        && segments[0] != 0x2002
}
