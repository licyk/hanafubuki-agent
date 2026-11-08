//! `memory status|reset`, through the running Agent's memory.

use crate::agent::with_agent;
use crate::cli::{GlobalArgs, MemoryCommand};
use crate::exit::Failure;

pub(crate) fn run(global: &GlobalArgs, command: MemoryCommand) -> Result<(), Failure> {
    with_agent(global, |_, agent| {
        let status = agent.memory_status().map_err(Failure::new)?;
        match command {
            MemoryCommand::Status => {
                println!("enabled: {}", status.enabled);
                println!("sessions read: {}", status.source_count);
                println!("extracted memories: {}", status.phase_one_count);
                println!("running jobs: {}", status.running_jobs);
                if let Some(error) = &status.last_error {
                    println!("last error: {error}");
                }
                Ok(())
            }
            MemoryCommand::Reset => agent
                .reset_memory(status.generation)
                .map(|_| ())
                .map_err(Failure::new),
        }
    })
}
