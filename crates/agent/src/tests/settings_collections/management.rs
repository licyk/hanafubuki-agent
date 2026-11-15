use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn skills_install_inspect_enable_remove_and_refuse_unrequested_overwrite() {
    let (service, paths, _) = service_with_config("settings-skill-management", |_, _| {});
    let source = paths.app_data_dir.join("example-skill");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("SKILL.md"),
        "---\nname: example-skill\ndescription: Test Skill\n---\nInstructions.\n",
    )
    .unwrap();
    let installed = service
        .run_settings_action(action("skills", source.to_str(), "add", None, json!({})))
        .await
        .unwrap()
        .record
        .unwrap();
    let catalog = service.settings_catalog().await.unwrap();
    let skills = collection(&catalog, "skills");
    assert!(skills
        .records
        .iter()
        .any(|record| record.id == installed && record.summary.contains("Test Skill")));
    assert!(service
        .run_settings_action(action("skills", source.to_str(), "add", None, json!({})))
        .await
        .is_err());
    service
        .run_settings_action(action(
            "skills",
            source.to_str(),
            "add",
            None,
            json!({"overwrite": true}),
        ))
        .await
        .unwrap();
    service
        .run_settings_action(action(
            "skills",
            Some(&installed),
            "save",
            None,
            json!({"enabled": false}),
        ))
        .await
        .unwrap();
    let id = SkillId::from_untrusted(&installed).unwrap();
    assert!(!service.list_skills().unwrap().entry(&id).unwrap().enabled);
    let content = service
        .run_settings_action(action(
            "skills",
            Some(&installed),
            "inspect",
            None,
            json!({}),
        ))
        .await
        .unwrap()
        .message
        .unwrap();
    assert!(content.contains("Instructions."));
    service
        .run_settings_action(action(
            "skills",
            Some(&installed),
            "delete",
            None,
            json!({}),
        ))
        .await
        .unwrap();
    assert!(service.list_skills().unwrap().entry(&id).is_none());
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn hook_definition_edits_invalidate_trust_and_keep_enablement() {
    let (service, paths, _) = service_with_config("settings-hook-management", |_, _| {});
    service
        .run_settings_action(action(
            "hooks",
            None,
            "add",
            None,
            json!({"event": "PreToolUse", "command": "check-one", "timeout": 7, "enabled": false}),
        ))
        .await
        .unwrap();
    let hooks = service.hooks_list().await.unwrap();
    let hook = &hooks.hooks[0];
    assert!(!hook.enabled);
    service.trust_hooks(vec![hook.key.clone()]).await.unwrap();
    service
        .run_settings_action(action(
            "hooks",
            Some(&hook.key),
            "save",
            None,
            json!({"command": "check-two"}),
        ))
        .await
        .unwrap();
    let hooks = service.hooks_list().await.unwrap();
    assert_eq!(hooks.hooks.len(), 1);
    assert!(!hooks.hooks[0].enabled);
    assert_ne!(
        hooks.hooks[0].trust,
        hanafubuki_agent_hooks::HookTrust::Trusted
    );
    service
        .run_settings_action(action(
            "hooks",
            Some(&hooks.hooks[0].key),
            "delete",
            None,
            json!({}),
        ))
        .await
        .unwrap();
    assert!(service.hooks_list().await.unwrap().hooks.is_empty());
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn user_role_files_can_be_created_edited_linked_and_copied_without_writing_builtins() {
    let (service, paths, config) = service_with_config("settings-role-file", |_, _| {});
    service
        .run_settings_action(action(
            "roleFiles",
            Some("file_role"),
            "add",
            None,
            json!({"description": "From file", "instructions": "First instructions"}),
        ))
        .await
        .unwrap();
    service
        .run_settings_action(action(
            "roleFiles",
            Some("file_role"),
            "save",
            None,
            json!({"instructions": "Second instructions"}),
        ))
        .await
        .unwrap();
    assert_eq!(
        service
            .roles()
            .role("file_role")
            .unwrap()
            .instructions
            .as_deref(),
        Some("Second instructions")
    );
    service
        .run_settings_action(action(
            "roles",
            Some("linked"),
            "add",
            None,
            json!({"configFile": "agents/file_role.toml"}),
        ))
        .await
        .unwrap();
    assert_eq!(
        service
            .roles()
            .role("linked")
            .unwrap()
            .instructions
            .as_deref(),
        Some("Second instructions")
    );
    let copied = service
        .run_settings_action(action("roles", Some("default"), "copy", None, json!({})))
        .await
        .unwrap()
        .record
        .unwrap();
    assert_eq!(
        config.user_document().unwrap()["agents"][&copied]["extends"],
        "default"
    );
    let outside = paths.app_data_dir.join("outside.toml");
    assert!(config
        .write_user_role_file(&outside, &serde_json::Map::new())
        .is_err());
    assert!(service
        .run_settings_action(action(
            "roleFiles",
            Some("../escape"),
            "add",
            None,
            json!({"instructions": "x"})
        ))
        .await
        .is_err());
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn profiles_edit_role_overrides_atomically_and_keep_unknown_values() {
    let (service, paths, config) = layered(
        "profile-role-management",
        "[profiles.work]\nfutureKey = 7\n[agents.review]\ninstructions = 'base'\n",
        Some("work"),
    );
    service
        .run_settings_action(action(
            "profiles",
            Some("work"),
            "save",
            None,
            json!({"agents.\"review\".instructions": "profile", "agents.\"review\".description": "Profile reviewer", "permissionMode": "full_access"}),
        ))
        .await
        .unwrap();
    assert_eq!(
        service
            .roles()
            .role("review")
            .unwrap()
            .instructions
            .as_deref(),
        Some("profile")
    );
    assert_eq!(
        config.user_document().unwrap()["profiles"]["work"]["futureKey"],
        7
    );
    let before = config.user_document().unwrap();
    assert!(service
        .run_settings_action(action(
            "profiles",
            Some("work"),
            "save",
            None,
            json!({"agents.\"review\".instructions": "invalid write", "permissionMode": "invalid"})
        ))
        .await
        .is_err());
    assert_eq!(config.user_document().unwrap(), before);
    assert!(config
        .layer_document(&hanafubuki_agent_config::SettingsLayer::Project)
        .is_err());
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_persisted_fields_warn_without_hiding_valid_settings_or_deleting_data() {
    let (service, paths, config) = layered("settings-tolerant-profile", "[profiles.work]\nfutureKey = 9\npermissionMode = 123\n[profiles.work.memory]\nenabled = false\n", None);
    let before = std::fs::read(paths.config_dir.join("config.toml")).unwrap();
    let catalog = service.settings_catalog().await.unwrap();
    let profiles = collection(&catalog, "profiles");
    let record = profiles
        .records
        .iter()
        .find(|record| record.id == "work")
        .unwrap();
    assert!(record
        .issue
        .as_ref()
        .is_some_and(|issue| issue.contains("permissionMode")));
    assert_eq!(
        record.values["memory.enabled"],
        SettingValue::Boolean(false)
    );
    assert_eq!(
        std::fs::read(paths.config_dir.join("config.toml")).unwrap(),
        before
    );
    service
        .run_settings_action(action(
            "profiles",
            Some("work"),
            "save",
            None,
            json!({"memory.enabled": true}),
        ))
        .await
        .unwrap();
    assert_eq!(
        config.user_document().unwrap()["profiles"]["work"]["futureKey"],
        9
    );
    assert_eq!(
        config.user_document().unwrap()["profiles"]["work"]["permissionMode"],
        123
    );
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn project_layer_requires_trust_then_writes_its_own_file() {
    let paths = test_paths("settings-project-target");
    let project = paths.app_data_dir.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&paths.config_dir).unwrap();
    std::fs::write(paths.config_dir.join("config.toml"), "").unwrap();
    let config = AgentConfig::layered(
        &paths.config_dir,
        ConfigLayers {
            project: Some(project.clone()),
            ..Default::default()
        },
    );
    let layer = hanafubuki_agent_config::SettingsLayer::Project;
    let specs = hanafubuki_agent_config::config_sections()
        .into_iter()
        .flat_map(|section| section.settings)
        .collect::<Vec<_>>();
    let values = serde_json::from_value(json!({"memory.enabled": false})).unwrap();
    assert!(config.write_layer_fields(&layer, &values, &specs).is_err());
    config
        .set_user_json(
            &format!("projects.{}.trustLevel", json!(project.to_str().unwrap())),
            json!("trusted"),
        )
        .unwrap();
    let path = config.write_layer_fields(&layer, &values, &specs).unwrap();
    assert!(path.ends_with(".hanafubuki-agent/config.toml"));
    assert!(!config.settings().unwrap().memory.enabled);
    assert!(config.user_document().unwrap().get("memory").is_none());
    std::fs::remove_dir_all(paths.app_data_dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn role_files_report_bad_fields_and_preserve_them_when_other_fields_are_edited() {
    let (service, paths, config) = service_with_config("settings-role-tolerant", |paths, _| {
        std::fs::create_dir_all(paths.config_dir.join("agents")).unwrap();
        std::fs::write(
            paths.config_dir.join("agents/damaged.toml"),
            "description = 'Partially readable'\ninstructions = 123\nfutureKey = 'keep'\n",
        )
        .unwrap();
    });
    let catalog = service.settings_catalog().await.unwrap();
    let files = collection(&catalog, "roleFiles");
    let record = files
        .records
        .iter()
        .find(|record| record.id == "damaged")
        .unwrap();
    assert!(record.issue.as_deref().unwrap().contains("instructions"));
    assert_eq!(
        record.values["description"],
        SettingValue::Text("Partially readable".into())
    );
    assert!(record.read_only.is_none());
    service
        .run_settings_action(action(
            "roleFiles",
            Some("damaged"),
            "save",
            None,
            json!({"description": "Updated description"}),
        ))
        .await
        .unwrap();
    let document = config
        .read_role_document(&paths.config_dir.join("agents/damaged.toml"))
        .unwrap();
    assert_eq!(document["instructions"], 123);
    assert_eq!(document["futureKey"], "keep");
    assert_eq!(document["description"], "Updated description");
    done(service, paths);
}

#[tokio::test(flavor = "multi_thread")]
async fn editing_a_hook_keeps_unknown_fields_and_other_unreadable_entries() {
    let (service, paths, _) = service_with_config("settings-hook-tolerant", |paths, _| {
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::write(
            paths.config_dir.join("hooks.json"),
            serde_json::to_vec(&json!({
                "futureRoot": 7,
                "hooks": {"PreToolUse": [
                    {"futureGroup": true, "hooks": [
                        {"type": "command", "command": "before", "futureHandler": "keep"},
                        {"type": "future-kind", "unknownPayload": 12}
                    ]}
                ]}
            }))
            .unwrap(),
        )
        .unwrap();
    });
    let hooks = service.hooks_list().await.unwrap();
    assert_eq!(hooks.hooks.len(), 1);
    service
        .run_settings_action(action(
            "hooks",
            Some(&hooks.hooks[0].key),
            "save",
            None,
            json!({"command": "after"}),
        ))
        .await
        .unwrap();
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(paths.config_dir.join("hooks.json")).unwrap())
            .unwrap();
    assert_eq!(document["futureRoot"], 7);
    let groups = document["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(groups[0]["hooks"][0]["unknownPayload"], 12);
    assert_eq!(groups[1]["futureGroup"], true);
    assert_eq!(groups[1]["hooks"][0]["futureHandler"], "keep");
    assert_eq!(groups[1]["hooks"][0]["command"], "after");
    done(service, paths);
}
