//! `hanafubuki-agent`: the standalone Agent's command line (docs/cli.md).

mod agent;
mod cli;
mod config;
mod doctor;
mod exec;
mod exit;
mod interactive;
mod mcp;
mod memory;
mod providers;
mod sessions;
mod skills;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::{Cli, Command};
use crate::exit::{Failure, SUCCESS};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let global = &cli.global;
    let result = match cli.command {
        None => interactive::run(global, cli.prompt),
        Some(Command::Resume { .. }) => Err(Failure::usage(
            "resuming a session is not available yet; run one turn with \
             `hanafubuki-agent exec PROMPT`",
        )),
        Some(Command::Exec(args)) => exec::run(global, args),
        Some(Command::Config(command)) => config::run(global, command).map(|()| SUCCESS),
        Some(Command::Providers(command)) => providers::run(global, command).map(|()| SUCCESS),
        Some(Command::Mcp(command)) => mcp::run(global, command).map(|()| SUCCESS),
        Some(Command::Sessions(command)) => sessions::run(global, command).map(|()| SUCCESS),
        Some(Command::Skills(command)) => skills::run(global, command).map(|()| SUCCESS),
        Some(Command::Memory(command)) => memory::run(global, command).map(|()| SUCCESS),
        Some(Command::Doctor { online }) => doctor::run(global, online),
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(failure) => failure.report(),
    }
}
