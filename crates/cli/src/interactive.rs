//! `hanafubuki-agent [PROMPT]` and `hanafubuki-agent resume`: the terminal
//! interface, on a terminal.

use std::io::IsTerminal;

use hanafubuki_agent::{AgentRuntimeHost, AgentSessionId};
use hanafubuki_agent_tui::{TuiEvents, TuiOptions, TuiResume};

use crate::agent::Standalone;
use crate::cli::GlobalArgs;
use crate::exit::{Failure, SUCCESS};

/// `resume [ID] [PROMPT]`, `resume --last [PROMPT]`: with neither an id nor
/// `--last`, the interface opens on the resume picker.
pub(crate) fn resume(
    global: &GlobalArgs,
    session: Option<String>,
    prompt: Option<String>,
    last: bool,
) -> Result<u8, Failure> {
    let (resume, prompt) = match (last, session, prompt) {
        (true, _, Some(_)) => {
            return Err(Failure::usage("`resume --last` takes no session id"));
        }
        (true, prompt, None) => (TuiResume::Last, prompt),
        (false, Some(id), prompt) => {
            let id = id
                .parse()
                .map(AgentSessionId::new)
                .map_err(|_| Failure::usage(format!("`{id}` is not a session id")))?;
            (TuiResume::Session(id), prompt)
        }
        (false, None, _) => (TuiResume::Picker, None),
    };
    run(global, prompt, Some(resume))
}

pub(crate) fn run(
    global: &GlobalArgs,
    prompt: Option<String>,
    resume: Option<TuiResume>,
) -> Result<u8, Failure> {
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
            resume,
        };
        hanafubuki_agent_tui::run(events, &standalone.runtime, &agent, options)
            .map_err(Failure::new)
    });
    standalone.shutdown();
    result.map(|()| SUCCESS)
}
