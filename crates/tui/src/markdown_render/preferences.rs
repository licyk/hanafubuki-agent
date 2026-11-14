// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `tui.rendering`, read by every Markdown render: one process-wide value,
//! set at launch and when the settings change. What was rendered before a
//! change keeps its look until it is laid out for another width (an answer
//! or a plan after a resize), as in Codex. Tests read a value of their own
//! thread, so parallel tests do not change each other's rendering.
//!
//! How web links show ([`WebLinkDisplay`]) is no setting: it is the
//! terminal's, detected once at launch. Tests keep the fallback, which
//! shows the destination, unless they set it.

use hanafubuki_agent::AgentTuiRendering;

use super::WebLinkDisplay;

#[cfg(not(test))]
static RENDERING: parking_lot::RwLock<AgentTuiRendering> =
    parking_lot::RwLock::new(AgentTuiRendering {
        tables: true,
        lists: true,
        math: true,
        mermaid: true,
    });

#[cfg(test)]
thread_local! {
    static RENDERING: std::cell::Cell<AgentTuiRendering> =
        std::cell::Cell::new(AgentTuiRendering::default());
}

pub(crate) fn set(rendering: AgentTuiRendering) {
    #[cfg(not(test))]
    {
        *RENDERING.write() = rendering;
    }
    #[cfg(test)]
    RENDERING.set(rendering);
}

pub(crate) fn current() -> AgentTuiRendering {
    #[cfg(not(test))]
    {
        *RENDERING.read()
    }
    #[cfg(test)]
    RENDERING.get()
}

#[cfg(not(test))]
static WEB_LINK_DISPLAY: parking_lot::RwLock<WebLinkDisplay> =
    parking_lot::RwLock::new(WebLinkDisplay::WithDestination);

#[cfg(test)]
thread_local! {
    static WEB_LINK_DISPLAY: std::cell::Cell<WebLinkDisplay> =
        const { std::cell::Cell::new(WebLinkDisplay::WithDestination) };
}

pub(crate) fn set_web_link_display(display: WebLinkDisplay) {
    #[cfg(not(test))]
    {
        *WEB_LINK_DISPLAY.write() = display;
    }
    #[cfg(test)]
    WEB_LINK_DISPLAY.set(display);
}

pub(crate) fn web_link_display() -> WebLinkDisplay {
    #[cfg(not(test))]
    {
        *WEB_LINK_DISPLAY.read()
    }
    #[cfg(test)]
    WEB_LINK_DISPLAY.get()
}
