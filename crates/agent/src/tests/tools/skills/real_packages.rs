//! The whole path, against Skill packages as they are actually published.
//!
//! Point `HANAFUBUKI_SKILL_SMOKE_ROOT` at a directory of real packages — a
//! Codex plugin cache is the obvious one — and this installs every package it
//! finds into a temporary managed root through the application service, reads
//! them back, and admits a turn carrying the whole catalogue. Without the
//! variable it does nothing, so the suite stays hermetic.

use super::*;

/// Install every package under the smoke root, then admit a turn that names
/// one. A package that Codex accepts must survive install, discovery, read,
/// selection, and turn admission here too.
#[cfg(unix)]
#[tokio::test]
async fn codex_cache_packages_install_and_admit_a_turn() {
    let Ok(cache) = std::env::var("HANAFUBUKI_SKILL_SMOKE_ROOT") else {
        return;
    };
    let cache = std::path::PathBuf::from(cache);

    let server = FakeServer::start(vec![(assistant_events("done"), Duration::ZERO)]).await;
    let (service, paths) = configured_service("skills-smoke", &server.base_url);

    let mut sources = Vec::new();
    collect(&cache, &mut sources);
    assert!(!sources.is_empty(), "no SKILL.md under {}", cache.display());
    println!("found {} packages under {}", sources.len(), cache.display());

    let mut installed = Vec::new();
    for directory in &sources {
        match service.install_local_skill(LocalSkillInstallRequest {
            source_directory: directory.clone(),
            overwrite: true,
        }) {
            Ok(entry) => {
                println!(
                    "  installed {} name={:?} resources={}",
                    entry.id,
                    entry.name,
                    entry.resources.len()
                );
                installed.push(entry);
            }
            Err(error) => panic!("install {} failed: {error}", directory.display()),
        }
    }

    let catalog = service.list_skills().unwrap();
    println!(
        "catalog entries={} issues={:?}",
        catalog.entries.len(),
        catalog.issues
    );
    assert!(catalog.issues.is_empty(), "{:?}", catalog.issues);
    assert_eq!(catalog.entries.len(), installed.len());

    // Every package's SKILL.md and every listed resource reads back.
    for entry in &catalog.entries {
        let read = service.read_skill(&entry.id).unwrap();
        assert!(read.instructions.starts_with("---"));
        for resource in &entry.resources {
            if resource.kind == hanafubuki_agent_skills::SkillResourceKind::Script {
                continue;
            }
            // Binary and oversized assets are refused as text; that is the read
            // boundary doing its job, not a load failure.
            let _ = service.read_skill_resource(&entry.id, &resource.path);
        }
    }

    // Turning one off and on again round-trips through the state file.
    let target = catalog.entries[0].id.clone();
    service.disable_skill(&target).unwrap();
    assert!(
        !service
            .list_skills()
            .unwrap()
            .entry(&target)
            .unwrap()
            .enabled
    );
    service.enable_skill(&target).unwrap();
    assert!(
        service
            .list_skills()
            .unwrap()
            .entry(&target)
            .unwrap()
            .enabled
    );

    // The whole catalog is captured for one turn, and admission accepts it.
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let named = catalog
        .entries
        .iter()
        .find(|entry| entry.allow_implicit_invocation)
        .unwrap_or(&catalog.entries[0]);
    service
        .start_turn(session.id, format!("${} help", named.name).into())
        .await
        .expect("turn admission must accept a catalog of real packages");

    let request: serde_json::Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    let input = request["input"].as_array().unwrap();
    let rendered = input
        .iter()
        .find(|item| {
            item["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("<skills_instructions>"))
        })
        .expect("the Skill catalog reaches the model");
    println!(
        "catalog fragment bytes={}",
        rendered["content"][0]["text"].as_str().unwrap().len()
    );

    let state = service.skill_session_state(session.id).unwrap();
    println!(
        "session available={} adopted={}",
        state.available.len(),
        state.current_turn_active.len()
    );

    assert!(service
        .shutdown(Instant::now() + Duration::from_secs(1))
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

fn collect(directory: &std::path::Path, output: &mut Vec<std::path::PathBuf>) {
    if directory.join("SKILL.md").is_file() {
        output.push(directory.to_path_buf());
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut children = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        })
        .collect::<Vec<_>>();
    children.sort();
    for child in children {
        collect(&child, output);
    }
}
