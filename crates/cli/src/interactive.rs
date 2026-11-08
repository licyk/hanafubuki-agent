//! `hanafubuki-agent [PROMPT]`: the terminal interface, on a terminal.

use std::io::IsTerminal;

use hanafubuki_agent::AgentRuntimeHost;
use hanafubuki_agent_tui::{TuiEvents, TuiOptions};

use crate::agent::Standalone;
use crate::cli::GlobalArgs;
use crate::exit::{Failure, SUCCESS};

pub(crate) fn run(global: &GlobalArgs, prompt: Option<String>) -> Result<u8, Failure> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(Failure::usage(
            "the terminal interface needs a terminal; run one turn with \
             `hanafubuki-agent exec PROMPT`",
        ));
    }
    let events = TuiEvents::new();
    let host = AgentRuntimeHost {
        status: events.status_sink(),
        ..AgentRuntimeHost::default()
    };
    let standalone = Standalone::open(global, host)?;
    let directory = match &global.cd {
        Some(directory) => directory.display().to_string(),
        None => std::env::current_dir()
            .map(|directory| directory.display().to_string())
            .unwrap_or_default(),
    };
    let result = standalone.start().and_then(|agent| {
        let options = TuiOptions {
            prompt,
            model: global.model.clone(),
            directory,
        };
        hanafubuki_agent_tui::run(events, &standalone.runtime, &agent, options)
            .map_err(Failure::new)
    });
    standalone.shutdown();
    result.map(|()| SUCCESS)
}
