//! `hanafubuki-agent`: the standalone Agent's command line
//! (site/content/docs/cli-reference.en.mdx).

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
use crate::exit::SUCCESS;

fn main() -> ExitCode {
    // First, before any thread: a sandboxed command re-executes this
    // binary as the sandbox helper, which never returns from here.
    hanafubuki_agent::sandbox::helper_dispatch();
    let cli = Cli::parse();
    let global = &cli.global;
    let result = match cli.command {
        None => interactive::run(global, cli.prompt, None),
        Some(Command::Resume {
            session,
            prompt,
            last,
        }) => interactive::resume(global, session, prompt, last),
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
