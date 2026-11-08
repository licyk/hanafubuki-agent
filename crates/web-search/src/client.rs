use reqwest::ClientBuilder;

use crate::model::WebSearchProxy;

pub(crate) fn apply_proxy(
    builder: ClientBuilder,
    proxy: &WebSearchProxy,
) -> Result<ClientBuilder, ()> {
    match proxy {
        WebSearchProxy::System => Ok(builder),
        WebSearchProxy::Disabled => Ok(builder.no_proxy()),
        WebSearchProxy::Explicit(address) => reqwest::Proxy::all(address)
            .map(|proxy| builder.proxy(proxy))
            .map_err(|_| ()),
    }
}
