//! The command line, as `clap` parses it. The shape follows Codex's
//! (`codex-rs/cli/src/main.rs`): global flags before or after the command,
//! `exec` for one headless turn, and the management commands.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "hanafubuki-agent",
    version,
    about = "An AI coding and operations agent",
    override_usage = "hanafubuki-agent [OPTIONS] [PROMPT]\n       hanafubuki-agent [OPTIONS] <COMMAND> [ARGS]",
    subcommand_negates_reqs = true
)]
pub(crate) struct Cli {
    #[command(flatten)]
    pub(crate) global: GlobalArgs,
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
    /// The first message of an interactive session in the terminal
    /// interface.
    pub(crate) prompt: Option<String>,
}

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct GlobalArgs {
    /// Overrides a key of config.toml for this run (`-c memory.enabled=true`);
    /// the value is TOML, or a string when it is not.
    #[arg(short = 'c', long = "config", value_name = "KEY=VALUE", global = true)]
    pub(crate) overrides: Vec<String>,
    /// Reads the profile [profiles.<PROFILE>] of config.toml over it.
    #[arg(short = 'p', long, global = true)]
    pub(crate) profile: Option<String>,
    /// The model a new session runs, in place of the configured one.
    #[arg(short = 'm', long, global = true)]
    pub(crate) model: Option<String>,
    /// Works in DIR instead of the current directory.
    #[arg(short = 'C', long = "cd", value_name = "DIR", global = true)]
    pub(crate) cd: Option<PathBuf>,
    /// Adds DIR to the workspace.
    #[arg(long = "add-dir", value_name = "DIR", global = true)]
    pub(crate) add_dirs: Vec<PathBuf>,
    /// The permission mode for this run.
    #[arg(long, value_enum, global = true)]
    pub(crate) permission_mode: Option<PermissionMode>,
    /// The log file's level for this run.
    #[arg(long, value_enum, global = true)]
    pub(crate) log_level: Option<LogLevel>,
    /// Keeps the history, memory, Skills and logs in DIR.
    #[arg(long, value_name = "DIR", global = true)]
    pub(crate) data_dir: Option<PathBuf>,
    /// Keeps the terminal interface's conversation in the terminal's
    /// scrollback, off the alternate screen.
    #[arg(long, global = true)]
    pub(crate) no_alt_screen: bool,
    /// Starts the Agent in safe mode: only its core runs, with no plugin
    /// row (`disabledPlugins` is unchanged) and no installed plugin's
    /// content.
    #[arg(long, global = true)]
    pub(crate) safe_mode: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum PermissionMode {
    Manual,
    AutoReview,
    FullAccess,
}

impl PermissionMode {
    /// The value of `permissionMode` in config.toml.
    pub(crate) fn key_value(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::AutoReview => "auto_review",
            Self::FullAccess => "full_access",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Runs one turn without the terminal interface.
    Exec(ExecArgs),
    /// Reopens a session in the terminal interface: the one named, the
    /// latest with --last, or the one picked from the sessions.
    Resume {
        /// The session to reopen (with --last: the first message).
        session: Option<String>,
        /// Submitted as the first message.
        prompt: Option<String>,
        /// Reopens the latest session a turn ran in.
        #[arg(long)]
        last: bool,
    },
    /// Shows and changes config.toml.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Lists and changes the model Providers.
    #[command(subcommand)]
    Providers(ProvidersCommand),
    /// Lists, changes and connects the MCP servers.
    #[command(subcommand)]
    Mcp(McpCommand),
    /// Lists, shows, archives and deletes sessions.
    #[command(subcommand)]
    Sessions(SessionsCommand),
    /// Lists, installs and removes Skills.
    #[command(subcommand)]
    Skills(SkillsCommand),
    /// Installs, lists, turns on and off, updates and removes plugins, and
    /// manages the marketplaces they come from.
    #[command(subcommand)]
    Plugins(PluginsCommand),
    /// Shows and resets the memory.
    #[command(subcommand)]
    Memory(MemoryCommand),
    /// Checks the configuration, the Providers, the data directory, the
    /// keyring and the MCP servers.
    Doctor {
        /// Also reaches the default Provider and connects the MCP servers.
        #[arg(long)]
        online: bool,
    },
}

#[derive(Debug, Args)]
pub(crate) struct ExecArgs {
    /// The message; `-`, or none with standard input not a terminal, reads
    /// it from standard input.
    pub(crate) prompt: Option<String>,
    /// Prints every event as one JSON object per line.
    #[arg(long)]
    pub(crate) json: bool,
    /// How approvals are answered: `deny`, `ask` (on a terminal; denied
    /// otherwise) or `allow:<tool>[,<tool>…]` (those tools allowed, others
    /// denied).
    #[arg(long, value_name = "POLICY", default_value = "deny")]
    pub(crate) approve: String,
    /// Writes the turn's final message to FILE.
    #[arg(short = 'o', long, value_name = "FILE")]
    pub(crate) output_last_message: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigCommand {
    /// Prints where the configuration files, the data and the log are.
    Path,
    /// Prints a key's value with every layer applied, or all of them.
    Get {
        /// A dotted key (`memory.enabled`, `profiles.ci.logLevel`,
        /// `projects."/path".trustLevel`).
        key: Option<String>,
    },
    /// Sets a key in the user's config.toml.
    Set {
        key: String,
        /// TOML, or a string when it is not.
        value: String,
    },
    /// Opens the user's config.toml in $VISUAL or $EDITOR.
    Edit,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ProvidersCommand {
    /// Lists the Providers and the model policy; tokens are never printed.
    List,
    /// Adds a Provider.
    Add {
        /// Its display name.
        name: String,
        #[arg(long)]
        base_url: String,
        #[arg(long, value_enum, default_value = "responses")]
        protocol: Protocol,
        /// Reads its API token from this environment variable.
        #[arg(long, value_name = "VAR")]
        api_key_env: Option<String>,
        /// A header sent with each of its requests.
        #[arg(long = "header", value_name = "NAME=VALUE")]
        headers: Vec<String>,
        /// Adds it turned off.
        #[arg(long)]
        disabled: bool,
    },
    /// Removes a Provider.
    Remove { id: String },
    /// Makes a Provider the default, optionally with the model it runs.
    Default {
        id: String,
        #[arg(long)]
        model: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Protocol {
    Responses,
    ChatCompletions,
    AnthropicMessages,
}

#[derive(Debug, Subcommand)]
pub(crate) enum McpCommand {
    /// Lists the MCP servers; credentials are never printed.
    List,
    /// Adds a server: `--url URL` for Streamable HTTP, or a program after
    /// `--` for stdio.
    Add {
        id: String,
        #[arg(long, conflicts_with = "command")]
        url: Option<String>,
        /// Whether the Streamable HTTP server takes an OAuth login (`mcp
        /// login`); `disabled` when not given.
        #[arg(long, value_enum)]
        oauth: Option<OAuth>,
        /// An environment variable of the stdio program.
        #[arg(long = "env", value_name = "KEY=VALUE")]
        environment: Vec<String>,
        /// Adds it turned off.
        #[arg(long)]
        disabled: bool,
        /// The program and its arguments.
        #[arg(last = true, value_name = "COMMAND")]
        command: Vec<String>,
    },
    /// Removes a server.
    Remove { id: String },
    /// Connects a server and reports how it went.
    Connect { id: String },
    /// Logs in to a Streamable HTTP server with OAuth: prints the URL to open
    /// in a browser and waits for the authorization.
    Login { id: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum OAuth {
    Disabled,
    Optional,
    Required,
}

#[derive(Debug, Subcommand)]
pub(crate) enum SessionsCommand {
    /// Lists the sessions, the latest first.
    List {
        /// The archived sessions instead.
        #[arg(long)]
        archived: bool,
        #[arg(long)]
        json: bool,
    },
    /// Prints a session's transcript.
    Show {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Archives a session.
    Archive { id: String },
    /// Deletes a session and its history.
    Delete { id: String },
}

#[derive(Debug, Subcommand)]
pub(crate) enum SkillsCommand {
    /// Lists the Skills.
    List,
    /// Installs the Skill in a directory.
    Install {
        directory: PathBuf,
        /// Replaces an installed Skill of the same name.
        #[arg(long)]
        overwrite: bool,
    },
    /// Removes an installed Skill.
    Remove { id: String },
}

#[derive(Debug, Subcommand)]
pub(crate) enum MemoryCommand {
    /// Prints the memory's state.
    Status,
    /// Forgets everything the memory holds.
    Reset,
}

#[derive(Debug, Subcommand)]
pub(crate) enum PluginsCommand {
    /// Lists the installed plugins; with --available, every plugin the
    /// added marketplaces offer too.
    List {
        #[arg(long)]
        available: bool,
        #[arg(long)]
        json: bool,
    },
    /// Installs a plugin from an added marketplace (PLUGIN@MARKETPLACE, or
    /// PLUGIN with --marketplace), or the plugin in a directory (filed
    /// under --marketplace, `local` when not given), and turns it on.
    #[command(
        after_help = "Examples:\n  hanafubuki-agent plugins add demo@my-marketplace\n  hanafubuki-agent plugins add demo --marketplace my-marketplace\n  hanafubuki-agent plugins add ./path/to/plugin"
    )]
    Add {
        /// PLUGIN[@MARKETPLACE], or a directory holding a plugin.
        #[arg(value_name = "PLUGIN[@MARKETPLACE] | DIR")]
        plugin: String,
        /// (No `-m`: it is the global `--model`.)
        #[arg(long, value_name = "MARKETPLACE")]
        marketplace: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Uninstalls a plugin; its data directory is kept unless --purge-data.
    Remove {
        /// PLUGIN@MARKETPLACE.
        id: String,
        #[arg(long)]
        purge_data: bool,
    },
    /// Turns an installed plugin on.
    Enable { id: String },
    /// Turns an installed plugin off.
    Disable { id: String },
    /// Installs a plugin again from its marketplace (as it is now) or the
    /// directory it came from.
    Update { id: String },
    /// Shows an installed plugin: its source, content, hooks and issues.
    Info {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Loads the installed plugins again and shows the plugin as it is then.
    Retry { id: String },
    /// Grants an installed plugin's code what its manifest asks for (the
    /// network, the workspace, environment variables), so its guest starts.
    Grant { id: String },
    /// Shows or replaces the settings of an installed plugin's code
    /// (`[plugins."ID"].settings`), which must fit its configSchema.
    #[command(
        after_help = "Examples:\n  hanafubuki-agent plugins settings demo@local\n  hanafubuki-agent plugins settings demo@local --set '{\"greeting\": \"Hi\"}'\n  hanafubuki-agent plugins settings demo@local --set null"
    )]
    Settings {
        id: String,
        /// Replaces them with a JSON object; `null` removes them.
        #[arg(long, value_name = "JSON")]
        set: Option<String>,
        /// Prints them and their configSchema (what it does without --set).
        #[arg(long, conflicts_with = "set")]
        get: bool,
    },
    /// Creates a plugin with code: a package whose manifest declares a
    /// tool and more, and the guest's source in Rust or Python, run as a
    /// process or (Rust) compiled to a WebAssembly module.
    #[command(
        after_help = "Examples:\n  hanafubuki-agent plugins new my-plugin\n  hanafubuki-agent plugins new my-plugin --target wasm\n  hanafubuki-agent plugins new my-plugin --lang python --dir plugins/my-plugin"
    )]
    New {
        /// The plugin's name: lowercase letters, digits and `-`, starting
        /// with a letter.
        name: String,
        #[arg(long, value_enum, default_value = "rust")]
        lang: PluginLanguage,
        /// What runs it: a program (`process`), or a WebAssembly module
        /// (`wasm`, Rust only).
        #[arg(long, value_enum, default_value = "process")]
        target: PluginTarget,
        /// The directory to create (default: ./NAME).
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
    },
    /// Checks the plugin in a directory as an install would read it, and
    /// its code as the Agent would run it; with --run, also starts it.
    Validate {
        dir: PathBuf,
        /// Also installs it into a throwaway home, grants its permissions
        /// (a program's; a WebAssembly module asks for none) and starts its
        /// guest, then reports how it ran.
        #[arg(long)]
        run: bool,
        #[arg(long)]
        json: bool,
    },
    /// Installs the plugin in a directory as NAME@local, turns it on, and
    /// installs it again each time its files change, until Ctrl+C.
    Dev {
        dir: PathBuf,
        /// Grants what its code asks for without asking.
        #[arg(long)]
        grant: bool,
    },
    /// Adds, lists, upgrades and removes the marketplaces plugins are
    /// installed from.
    #[command(subcommand)]
    Marketplace(MarketplaceCommand),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum PluginLanguage {
    Rust,
    Python,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum PluginTarget {
    Process,
    Wasm,
}

#[derive(Debug, Subcommand)]
pub(crate) enum MarketplaceCommand {
    /// Adds a marketplace: a local directory, a git URL or owner/repo[@ref].
    #[command(
        after_help = "Examples:\n  hanafubuki-agent plugins marketplace add ./path/to/marketplace\n  hanafubuki-agent plugins marketplace add owner/repo --ref main\n  hanafubuki-agent plugins marketplace add https://github.com/owner/repo --sparse plugins/foo"
    )]
    Add {
        source: String,
        /// The git ref to check out.
        #[arg(long = "ref", value_name = "REF")]
        ref_name: Option<String>,
        /// A path to check out alone (a git source; repeatable).
        #[arg(long = "sparse", value_name = "PATH")]
        sparse_paths: Vec<String>,
    },
    /// Lists the added marketplaces and their roots.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Brings a git marketplace, or every added one, to its remote's
    /// revision.
    Upgrade { name: Option<String> },
    /// Removes an added marketplace; the plugins installed from it stay.
    Remove { name: String },
}
