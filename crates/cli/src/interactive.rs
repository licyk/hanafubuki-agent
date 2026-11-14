//! `hanafubuki-agent [PROMPT]` and `hanafubuki-agent resume`: the terminal
//! interface, on a terminal.

use std::io::IsTerminal;

use hanafubuki_agent::{AgentRuntimeHost, AgentSessionId};
use hanafubuki_agent_tui::{TuiEvents, TuiOptions, TuiResume};

use crate::agent::Standalone;
use crate::cli::GlobalArgs;
use crate::exit::{Failure, SUCCESS};
use crate::update_check::{self, AtLaunch};

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
    // Before the start: the update prompt may end the process, and the
    // Agent has then neither run nor taken the data directory's lock.
    let update_notice =
        match update_check::at_launch(&standalone, events.update_notifier(), prompt.is_some()) {
            AtLaunch::Interface(notice) => notice,
            AtLaunch::Exit(result) => {
                standalone.shutdown();
                return result;
            }
        };
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
            role: global.role.clone(),
            directory,
            resume,
            no_alt_screen: global.no_alt_screen,
            update_notice,
        };
        hanafubuki_agent_tui::run(events, &standalone.runtime, &agent, options)
            .map_err(Failure::new)
    });
    standalone.shutdown();
    result.map(|()| SUCCESS)
}
