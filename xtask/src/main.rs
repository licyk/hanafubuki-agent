//! Repository tasks: `cargo xtask <command>`.

mod cli;
mod config_reference;
mod dependencies;
mod git;
mod locks;
mod model_api_context;
mod module_size;
mod notices;
mod path_boundaries;
mod plugin_panics;
mod source_layout;
mod strict_decoding;
mod tag;
mod version;
mod welcome;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Task};

fn main() -> ExitCode {
    match run(&root(), Cli::parse().task) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(root: &Path, task: Task) -> Result<(), String> {
    match task {
        Task::Check => {
            dependencies::run(root)?;
            source_layout::run(root)?;
            module_size::run(root)?;
            strict_decoding::run(root)?;
            model_api_context::run(root)?;
            path_boundaries::run(root)?;
            locks::run(root)?;
            plugin_panics::run(root)?;
            notices::run(root)?;
            config_reference::run(root)
        }
        Task::CheckDependencies => dependencies::run(root),
        Task::CheckSourceLayout => source_layout::run(root),
        Task::CheckModuleSize => module_size::run(root),
        Task::CheckStrictDecoding => strict_decoding::run(root),
        Task::CheckModelApiContext => model_api_context::run(root),
        Task::CheckPathBoundaries => path_boundaries::run(root),
        Task::CheckLocks => locks::run(root),
        Task::CheckPluginPanics => plugin_panics::run(root),
        Task::CheckNotices => notices::run(root),
        Task::CheckConfigReference => config_reference::run(root),
        Task::ModuleSizeReport { top } => {
            print!("{}", module_size::report(root, top.get()));
            Ok(())
        }
        Task::Version(options) => version::run(root, &options),
        Task::WelcomeAssets { source, output } => welcome::generate(
            &source,
            &output.unwrap_or_else(|| root.join("crates/tui/assets/welcome")),
        ),
        Task::WelcomePreview { input, output } => welcome::preview(&input, &output),
        Task::Tag(options) => tag::run(root, &options),
    }
}

/// The workspace root: the parent of this crate.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace")
        .to_path_buf()
}
