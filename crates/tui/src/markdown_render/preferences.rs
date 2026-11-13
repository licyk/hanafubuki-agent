// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! `tui.rendering`, read by every Markdown render: one process-wide value,
//! set at launch and when the settings change. What was rendered before a
//! change keeps its look. Tests read a value of their own thread, so
//! parallel tests do not change each other's rendering.

use hanafubuki_agent::AgentTuiRendering;

#[cfg(not(test))]
static RENDERING: parking_lot::RwLock<AgentTuiRendering> =
    parking_lot::RwLock::new(AgentTuiRendering {
        tables: true,
        lists: true,
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
