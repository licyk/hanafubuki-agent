//! Repository tasks: `cargo xtask <command>`.

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

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
Usage: cargo xtask <command>

Commands:
  check                                Run every check below
  check-dependencies                   Check the workspace's dependency direction and layers
  check-source-layout                  Check the Rust module and test layout
  check-module-size                    Check file and function sizes against their limits
  check-strict-decoding                Check that deny_unknown_fields is only on listed types
  check-model-api-context              Check that hanafubuki-agent-model-api names no context section kind
  check-path-boundaries                Check that paths resolve only through the foundation's authority
  check-locks                          Check that production code uses no std Mutex or RwLock
  check-plugin-panics                  Check that plugin production code does not unwrap or expect
  check-notices                        Check the third-party attribution of derived files and NOTICE
  check-config-reference               Check that every setting has its row in the configuration reference
  module-size-report [--top N]         Print the largest files and functions
  version <patch|minor|major>          Bump the workspace version everywhere it is named
          [--dry-run]
  tag [--version X.Y.Z | --name NAME]  Create an annotated tag on HEAD (default: v<workspace version>)
      [--message TEXT] [--push] [--remote NAME] [--dry-run]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&root(), &args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let (command, rest) = args.split_first().ok_or_else(|| USAGE.to_string())?;
    match command.as_str() {
        "check" => {
            no_arguments(rest)?;
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
        "check-dependencies" => {
            no_arguments(rest)?;
            dependencies::run(root)
        }
        "check-source-layout" => {
            no_arguments(rest)?;
            source_layout::run(root)
        }
        "check-module-size" => {
            no_arguments(rest)?;
            module_size::run(root)
        }
        "check-strict-decoding" => {
            no_arguments(rest)?;
            strict_decoding::run(root)
        }
        "check-model-api-context" => {
            no_arguments(rest)?;
            model_api_context::run(root)
        }
        "check-path-boundaries" => {
            no_arguments(rest)?;
            path_boundaries::run(root)
        }
        "check-locks" => {
            no_arguments(rest)?;
            locks::run(root)
        }
        "check-plugin-panics" => {
            no_arguments(rest)?;
            plugin_panics::run(root)
        }
        "check-notices" => {
            no_arguments(rest)?;
            notices::run(root)
        }
        "check-config-reference" => {
            no_arguments(rest)?;
            config_reference::run(root)
        }
        "module-size-report" => {
            print!("{}", module_size::report(root, top_option(rest)?));
            Ok(())
        }
        "version" => version::run(root, &version::Options::parse(rest)?),
        "tag" => tag::run(root, &tag::Options::parse(rest)?),
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

fn top_option(rest: &[String]) -> Result<usize, String> {
    match rest {
        [] => Ok(20),
        [option, value] if option == "--top" => value
            .parse()
            .ok()
            .filter(|top| *top > 0)
            .ok_or_else(|| "--top requires a positive number".to_string()),
        _ => Err("usage: module-size-report [--top N]".to_string()),
    }
}

fn no_arguments(rest: &[String]) -> Result<(), String> {
    match rest.first() {
        Some(argument) => Err(format!("unexpected argument `{argument}`")),
        None => Ok(()),
    }
}

/// The workspace root: the parent of this crate.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace")
        .to_path_buf()
}
