//! `roles list|show`: the roles the configuration and the installed plugins
//! that are on define over the built-in ones, read and resolved without
//! starting the Agent or opening the plugins' store
//! (site/content/docs/roles.en.mdx).

use hanafubuki_agent::{
    AgentPermissionMode, AgentResolvedRole, AgentRoleCatalog, AgentRuntimeHost,
};

use crate::agent::Standalone;
use crate::cli::{GlobalArgs, RolesCommand};
use crate::exit::Failure;

pub(crate) fn run(global: &GlobalArgs, command: RolesCommand) -> Result<(), Failure> {
    let standalone = Standalone::open(global, AgentRuntimeHost::default())?;
    let catalog = AgentRoleCatalog::load(
        standalone.runtime.config(),
        &standalone.runtime.storage_paths().plugins_root,
        global.safe_mode,
    );
    standalone.shutdown();
    match command {
        RolesCommand::List => {
            list(&catalog);
            Ok(())
        }
        RolesCommand::Show { name } => show(&catalog, &name),
    }
}

/// One line per role, by name: its source and the first line of its
/// description, or why it is not usable; the issues go to standard error.
fn list(catalog: &AgentRoleCatalog) {
    let mut lines: Vec<(String, String)> = catalog
        .roles()
        .map(|role| {
            let spawnable = if role.spawnable {
                ""
            } else {
                " (not spawnable)"
            };
            let summary = role.description.lines().next().unwrap_or_default();
            (
                role.name.clone(),
                format!("{}{spawnable}  {summary}", role.source),
            )
        })
        .chain(catalog.unusable().map(|role| {
            (
                role.name.clone(),
                format!("{}  unusable: {}", role.source, role.reason),
            )
        }))
        .collect();
    lines.sort();
    for (name, line) in lines {
        println!("{name}  {line}");
    }
    for issue in catalog.issues() {
        eprintln!("warning: {issue}");
    }
}

fn show(catalog: &AgentRoleCatalog, name: &str) -> Result<(), Failure> {
    if let Some(role) = catalog.role(name) {
        print_role(role);
        return Ok(());
    }
    if let Some(role) = catalog.unusable_role(name) {
        println!("{}  {}", role.name, role.source);
        println!("unusable: {}", role.reason);
        return Ok(());
    }
    let names: Vec<&str> = catalog.roles().map(|role| role.name.as_str()).collect();
    Err(Failure::new(format!(
        "there is no role `{name}`; the usable roles are {}",
        names.join(", ")
    )))
}

/// Each effective field, then the roles of the chain that decide it.
fn print_role(role: &AgentResolvedRole) {
    println!("{}  {}", role.name, role.source);
    let chain: Vec<String> = role
        .chain
        .iter()
        .map(|link| format!("{} ({})", link.name, link.source))
        .collect();
    println!("chain: {}", chain.join(" → "));
    let scope = &role.scope;
    let unset = || "(not set)".to_string();
    let lists: Vec<String> = scope
        .tools
        .allow
        .iter()
        // Nearest first, as the roles that set them are named.
        .rev()
        .map(|list| format!("[{}]", join(list)))
        .collect();
    let fields = [
        ("description", Some(role.description.clone())),
        ("instructions", role.instructions.clone()),
        ("model", role.model.clone()),
        (
            "reasoningEffort",
            role.reasoning_effort.as_ref().map(ToString::to_string),
        ),
        ("spawnable", Some(role.spawnable.to_string())),
        (
            "permissionModeCap",
            scope
                .permission_mode_cap
                .map(|mode| mode_key(mode).to_string()),
        ),
        (
            "tools.allow",
            match lists.is_empty() {
                true => Some("every tool".to_string()),
                false => Some(format!("every one of {}", lists.join(" and "))),
            },
        ),
        (
            "tools.deny",
            Some(join(&scope.tools.deny)).filter(|deny| !deny.is_empty()),
        ),
        (
            "plugins.off",
            Some(join(&scope.plugins_off)).filter(|off| !off.is_empty()),
        ),
    ];
    for (field, value) in fields {
        let value = value.unwrap_or_else(unset);
        let from = role
            .provenance
            .get(field)
            .map(|roles| format!("  [from {}]", roles.join(", ")))
            .unwrap_or_default();
        let mut lines = value.lines();
        println!("{field}: {}{from}", lines.next().unwrap_or_default());
        for line in lines {
            println!("    {line}");
        }
    }
}

/// The mode as `permissionModeCap` writes it.
fn mode_key(mode: AgentPermissionMode) -> &'static str {
    match mode {
        AgentPermissionMode::Manual => "manual",
        AgentPermissionMode::AutoReview => "auto_review",
        AgentPermissionMode::FullAccess => "full_access",
    }
}

fn join<T: ToString>(items: impl IntoIterator<Item = T>) -> String {
    items
        .into_iter()
        .map(|item| item.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
