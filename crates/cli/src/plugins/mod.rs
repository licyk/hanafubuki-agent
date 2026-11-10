// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/cli/src/plugin_cmd.rs` (`plugin add|list|remove`: the
// `PLUGIN@MARKETPLACE` or `--marketplace` selection, the list's table,
// what an install and a removal print) and
// `codex-rs/cli/src/marketplace_cmd.rs` (`plugin marketplace
// add|list|upgrade|remove` and what each prints) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: `plugins` (not `plugin`) over the running
// Agent's facade calls; `add` also takes a plugin's directory; `enable`,
// `disable`, `update`, `info` and `retry` are added, and `remove` can
// purge the plugin's data; the list shows the installed plugins unless
// `--available`; remote marketplaces, apps and their auth are not ported;
// `grant`, `settings`, `new`, `validate` and `dev` (the code of plugins in
// this project's format) are this project's own.

mod code;
mod develop;
mod scaffold;

use std::path::Path;

use hanafubuki_agent::{
    AgentHandle, AgentMarketplace, AgentPackageInstall, AgentPackageSource, AgentPackageStatus,
    MarketplaceAddRequest, MarketplaceSourceType, PackageSource,
};
use serde::Serialize;

use crate::agent::{block_on, with_agent};
use crate::cli::{GlobalArgs, MarketplaceCommand, PluginsCommand};
use crate::exit::Failure;

pub(crate) fn run(global: &GlobalArgs, command: PluginsCommand) -> Result<(), Failure> {
    // The author's commands run without the Agent, or start their own.
    let command = match command {
        PluginsCommand::New {
            name,
            lang,
            target,
            dir,
        } => return scaffold::run(&name, lang, target, dir),
        PluginsCommand::Validate { dir, run, json } => {
            return develop::validate(global, &dir, run, json)
        }
        PluginsCommand::Dev { dir, grant } => return develop::dev(global, &dir, grant),
        command => command,
    };
    // A selection that cannot be used fails before the Agent starts.
    if let PluginsCommand::Add {
        plugin,
        marketplace,
        ..
    } = &command
    {
        install_source(plugin, marketplace.clone())?;
    }
    with_agent(global, |agent| match command {
        PluginsCommand::List { available, json } => list(agent, available, json),
        PluginsCommand::Add {
            plugin,
            marketplace,
            json,
        } => {
            let source = install_source(&plugin, marketplace)?;
            let install = block_on(
                agent,
                |agent| async move { agent.install_package(source).await },
            )?;
            if json {
                return print_json(&install);
            }
            let id = &install.package.id;
            println!("Added plugin `{id}` (version {}).", install.package.version);
            println!("Installed plugin root: {}", install.root.display());
            print_install_notes(&install);
            Ok(())
        }
        PluginsCommand::Remove { id, purge_data } => {
            let removed = block_on(agent, move |agent| async move {
                agent.uninstall_package(&id, purge_data).await
            })?;
            println!("Removed plugin `{}`.", removed.id);
            match removed.purged_data {
                Some(data) => println!("Deleted its data: {}", data.display()),
                None if purge_data => println!("It had no data."),
                None => println!("Its data is kept; --purge-data deletes it."),
            }
            Ok(())
        }
        PluginsCommand::Enable { id } => set_enabled(agent, id, true),
        PluginsCommand::Disable { id } => set_enabled(agent, id, false),
        PluginsCommand::Update { id } => {
            let install = block_on(agent, move |agent| async move {
                agent.update_package(&id).await
            })?;
            println!(
                "Updated plugin `{}` (version {}).",
                install.package.id, install.package.version
            );
            print_install_notes(&install);
            Ok(())
        }
        PluginsCommand::Info { id, json } => {
            let package = block_on(agent, move |agent| async move {
                let package = agent.package(&id).await?;
                package.ok_or_else(|| format!("plugin `{id}` is not installed"))
            })?;
            if json {
                return print_json(&package);
            }
            print_package(&package);
            Ok(())
        }
        PluginsCommand::Retry { id } => {
            let package = block_on(
                agent,
                move |agent| async move { agent.retry_package(&id).await },
            )?;
            print_package(&package);
            Ok(())
        }
        PluginsCommand::Grant { id } => code::grant(agent, id),
        PluginsCommand::Settings { id, set, get: _ } => code::settings(agent, id, set),
        PluginsCommand::Marketplace(command) => marketplace(agent, command),
        PluginsCommand::New { .. }
        | PluginsCommand::Validate { .. }
        | PluginsCommand::Dev { .. } => {
            unreachable!("handled before the Agent starts")
        }
    })
}

/// What `add PLUGIN` installs: a directory when PLUGIN is one, else
/// `PLUGIN@MARKETPLACE` or PLUGIN from `--marketplace`.
fn install_source(
    plugin: &str,
    marketplace: Option<String>,
) -> Result<AgentPackageSource, Failure> {
    if Path::new(plugin).is_dir() {
        let path = std::path::absolute(plugin)
            .map_err(|error| Failure::usage(format!("{plugin}: {error}")))?;
        return Ok(AgentPackageSource::Directory { path, marketplace });
    }
    let (plugin, marketplace) = match (plugin.split_once('@'), marketplace) {
        (Some((_, from_id)), Some(flag)) if from_id != flag => {
            return Err(Failure::usage(format!(
                "plugin id `{plugin}` belongs to marketplace `{from_id}`, but --marketplace specified `{flag}`"
            )));
        }
        (Some((name, from_id)), _) => (name.to_string(), from_id.to_string()),
        (None, Some(marketplace)) => (plugin.to_string(), marketplace),
        (None, None) => {
            return Err(Failure::usage(
                "plugin requires --marketplace unless passed as <plugin>@<marketplace> or a directory",
            ));
        }
    };
    Ok(AgentPackageSource::Marketplace {
        marketplace,
        plugin,
    })
}

fn set_enabled(agent: &AgentHandle, id: String, enabled: bool) -> Result<(), Failure> {
    let package = block_on(agent, move |agent| async move {
        agent.set_package_enabled(&id, enabled).await
    })?;
    let state = if package.enabled { "on" } else { "off" };
    println!("Plugin `{}` is {state}.", package.id);
    Ok(())
}

/// The installed plugins; with `available`, each added marketplace's
/// plugins under it, then the installed plugins no marketplace offers.
fn list(agent: &AgentHandle, available: bool, json: bool) -> Result<(), Failure> {
    let (packages, marketplaces) = block_on(agent, move |agent| async move {
        let packages = agent.packages().await?;
        let marketplaces = if available {
            agent.marketplaces().await?
        } else {
            Vec::new()
        };
        Ok::<_, String>((packages, marketplaces))
    })?;
    if json {
        #[derive(Serialize)]
        struct Listing<'a> {
            installed: &'a [AgentPackageStatus],
            #[serde(skip_serializing_if = "Option::is_none")]
            marketplaces: Option<&'a [AgentMarketplace]>,
        }
        return print_json(&Listing {
            installed: &packages,
            marketplaces: available.then_some(marketplaces.as_slice()),
        });
    }
    if !available {
        if packages.is_empty() {
            println!("No plugins are installed.");
        }
        let rows = packages
            .iter()
            .map(|package| {
                [
                    package.id.clone(),
                    package_state(package).to_string(),
                    package.version.clone(),
                    source_text(package.source.as_ref()),
                ]
            })
            .collect::<Vec<_>>();
        print_table(["PLUGIN", "STATUS", "VERSION", "SOURCE"], &rows);
        return Ok(());
    }
    if marketplaces.is_empty() {
        println!("No marketplaces are added.");
    }
    for (index, marketplace) in marketplaces.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("Marketplace `{}`", marketplace.name);
        println!("{}", marketplace.root.display());
        if let Some(error) = &marketplace.error {
            println!("  cannot be read: {error}");
            continue;
        }
        let rows = marketplace
            .plugins
            .iter()
            .map(|plugin| {
                let state = match (plugin.installed, plugin.enabled) {
                    (true, true) => "installed, enabled",
                    (true, false) => "installed, disabled",
                    _ if !plugin.installable => "not installable",
                    _ => "not installed",
                };
                [
                    plugin.id.clone(),
                    state.to_string(),
                    plugin.version.clone().unwrap_or_default(),
                    plugin.description.clone().unwrap_or_default(),
                ]
            })
            .collect::<Vec<_>>();
        print_table(["PLUGIN", "STATUS", "VERSION", "DESCRIPTION"], &rows);
    }
    let offered = |id: &str| {
        marketplaces
            .iter()
            .any(|marketplace| marketplace.plugins.iter().any(|plugin| plugin.id == id))
    };
    let others = packages
        .iter()
        .filter(|package| !offered(&package.id))
        .map(|package| {
            [
                package.id.clone(),
                package_state(package).to_string(),
                package.version.clone(),
                source_text(package.source.as_ref()),
            ]
        })
        .collect::<Vec<_>>();
    if !others.is_empty() {
        println!();
        println!("Installed from elsewhere");
        print_table(["PLUGIN", "STATUS", "VERSION", "SOURCE"], &others);
    }
    Ok(())
}

fn marketplace(agent: &AgentHandle, command: MarketplaceCommand) -> Result<(), Failure> {
    match command {
        MarketplaceCommand::Add {
            source,
            ref_name,
            sparse_paths,
        } => {
            let shown = source.clone();
            let added = block_on(agent, move |agent| async move {
                agent
                    .add_marketplace(MarketplaceAddRequest {
                        source,
                        ref_name,
                        sparse_paths,
                    })
                    .await
            })?;
            if added.already_added {
                println!(
                    "Marketplace `{}` is already added from {shown}.",
                    added.name
                );
            } else {
                println!("Added marketplace `{}` from {shown}.", added.name);
            }
            println!("Installed marketplace root: {}", added.root.display());
            Ok(())
        }
        MarketplaceCommand::List { json } => {
            let marketplaces = block_on(agent, |agent| async move { agent.marketplaces().await })?;
            if json {
                return print_json(&marketplaces);
            }
            if marketplaces.is_empty() {
                println!("No marketplaces are added.");
                return Ok(());
            }
            let rows = marketplaces
                .iter()
                .map(|marketplace| {
                    let kind = match marketplace.source_type {
                        MarketplaceSourceType::Git => "git",
                        MarketplaceSourceType::Local => "local",
                    };
                    let state = match (&marketplace.error, marketplace.enabled) {
                        (Some(_), _) => "unreadable",
                        (None, false) => "off",
                        (None, true) => "on",
                    };
                    [
                        marketplace.name.clone(),
                        format!("{kind}, {state}"),
                        marketplace.root.display().to_string(),
                    ]
                })
                .collect::<Vec<_>>();
            print_table(["MARKETPLACE", "SOURCE", "ROOT"], &rows);
            Ok(())
        }
        MarketplaceCommand::Upgrade { name } => {
            let named = name.clone();
            let upgrades = block_on(agent, move |agent| async move {
                agent.upgrade_marketplace(name).await
            })?;
            let mut failures = 0;
            for upgrade in &upgrades {
                if let Some(error) = &upgrade.error {
                    failures += 1;
                    eprintln!("Failed to upgrade marketplace `{}`: {error}", upgrade.name);
                }
            }
            if failures > 0 {
                return Err(Failure::new(format!(
                    "{failures} upgrade failure(s) occurred."
                )));
            }
            let upgraded = upgrades.iter().filter(|upgrade| upgrade.upgraded).count();
            match (named, upgrades.is_empty(), upgraded) {
                (None, true, _) => println!("No added git marketplaces to upgrade."),
                (Some(name), _, 0) => println!("Marketplace `{name}` is already up to date."),
                (None, _, 0) => println!("All added git marketplaces are already up to date."),
                (Some(name), _, _) => println!("Upgraded marketplace `{name}`."),
                (None, _, count) => println!("Upgraded {count} marketplace(s)."),
            }
            Ok(())
        }
        MarketplaceCommand::Remove { name } => {
            let removed = name.clone();
            block_on(agent, move |agent| async move {
                agent.remove_marketplace(&name).await
            })?;
            println!("Removed marketplace `{removed}`.");
            Ok(())
        }
    }
}

fn print_install_notes(install: &AgentPackageInstall) {
    for issue in &install.issues {
        eprintln!("warning: {issue}");
    }
    for skill in &install.migrated_commands {
        println!("Its command became the Skill {skill}.");
    }
    if install.permissions.is_some() {
        println!(
            "Its code waits for what it asks for to be granted: `plugins grant {}` shows and grants it.",
            install.package.id
        );
    }
    match install.hooks_to_trust.len() {
        0 => {}
        count => println!(
            "{count} of its hooks wait to be trusted: review them with /hooks or /plugins in the terminal interface."
        ),
    }
}

fn print_package(package: &AgentPackageStatus) {
    println!("{} ({})", package.display_name, package.id);
    if let Some(description) = &package.description {
        println!("  {description}");
    }
    println!("  status:      {}", package_state(package));
    println!("  version:     {}", package.version);
    if let Some(format) = package.format {
        println!("  format:      {format:?}");
    }
    println!("  source:      {}", source_text(package.source.as_ref()));
    let list = |items: &[String]| match items {
        [] => "none".to_string(),
        items => items.join(", "),
    };
    println!("  Skills:      {}", list(&package.content.skills));
    println!("  MCP servers: {}", list(&package.content.mcp_servers));
    println!("  hooks:       {}", list(&package.content.hooks));
    if !package.hooks_to_trust.is_empty() {
        println!("  to trust:    {}", list(&package.hooks_to_trust));
    }
    if let Some(guest) = &package.guest {
        let status = &guest.status;
        println!(
            "  code:        {}{}",
            code::state_text(status.state),
            code::runtime_text(guest.runtime)
        );
        if let Some(error) = &status.error {
            println!("  code error:  {error}");
        }
        for issue in &status.issues {
            println!("  code issue:  {issue}");
        }
    }
    for issue in &package.issues {
        println!("  issue:       {issue}");
    }
}

fn package_state(package: &AgentPackageStatus) -> &'static str {
    if package.enabled {
        "enabled"
    } else {
        "disabled"
    }
}

fn source_text(source: Option<&PackageSource>) -> String {
    match source {
        None => "unknown".to_string(),
        Some(PackageSource::Local { path }) => path.display().to_string(),
        Some(PackageSource::Git {
            url,
            path,
            ref_name,
            sha,
        }) => {
            let mut parts = vec![url.clone()];
            if let Some(path) = path {
                parts.push(format!("path `{path}`"));
            }
            if let Some(ref_name) = ref_name {
                parts.push(format!("ref `{ref_name}`"));
            }
            if let Some(sha) = sha {
                parts.push(format!("sha `{sha}`"));
            }
            parts.join(", ")
        }
    }
}

/// `rows` under `headers`, each column as wide as its widest cell; the
/// last column is not padded.
fn print_table<const N: usize>(headers: [&str; N], rows: &[[String; N]]) {
    if rows.is_empty() {
        return;
    }
    let mut widths = headers.map(str::len);
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: Vec<&str>| {
        let mut text = String::new();
        for (index, cell) in cells.iter().enumerate() {
            if index + 1 == N {
                text.push_str(cell);
            } else {
                text.push_str(&format!("{cell:<width$}  ", width = widths[index]));
            }
        }
        println!("{}", text.trim_end());
    };
    line(headers.to_vec());
    for row in rows {
        line(row.iter().map(String::as_str).collect());
    }
}

fn print_json(value: &impl Serialize) -> Result<(), Failure> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| Failure::new(format!("the output could not be written: {error}")))?;
    println!("{text}");
    Ok(())
}
