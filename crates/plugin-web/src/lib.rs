//! Web search and page fetch for the Agent: binds `hanafubuki-agent-web-search` to the
//! tool registry as `web_search` and `fetch_web_page`, and presents their
//! calls.

mod config;
mod plugin;
mod presenter;
mod tools;

pub use config::{WebPluginConfig, WebProviderSetting, WebProxySetting};
pub use plugin::web_plugin;
