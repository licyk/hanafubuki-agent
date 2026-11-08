//! `skills list|install|remove`, through the running Agent's Skill catalog.

use hanafubuki_agent::{LocalSkillInstallRequest, SkillId};

use crate::agent::with_agent;
use crate::cli::{GlobalArgs, SkillsCommand};
use crate::exit::Failure;

pub(crate) fn run(global: &GlobalArgs, command: SkillsCommand) -> Result<(), Failure> {
    with_agent(global, |agent| match command {
        SkillsCommand::List => {
            let catalog = agent.list_skills().map_err(Failure::new)?;
            for entry in &catalog.entries {
                let state = if entry.enabled { "" } else { " (disabled)" };
                println!("{}{state}  {}", entry.id, entry.description);
            }
            for issue in &catalog.issues {
                eprintln!("warning: {}: {}", issue.path, issue.message);
            }
            Ok(())
        }
        SkillsCommand::Install {
            directory,
            overwrite,
        } => {
            let source_directory = std::path::absolute(&directory)
                .map_err(|error| Failure::usage(format!("{}: {error}", directory.display())))?;
            let entry = agent
                .install_local_skill(LocalSkillInstallRequest {
                    source_directory,
                    overwrite,
                })
                .map_err(Failure::new)?;
            println!("installed {}", entry.id);
            Ok(())
        }
        SkillsCommand::Remove { id } => {
            let id =
                SkillId::from_untrusted(id).map_err(|error| Failure::usage(error.to_string()))?;
            agent.delete_skill(&id).map(|_| ()).map_err(Failure::new)
        }
    })
}
