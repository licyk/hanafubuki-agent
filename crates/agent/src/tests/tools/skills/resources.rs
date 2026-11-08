use super::*;

#[tokio::test]
async fn skills_read_pages_a_reference_utf8_with_content_bound_cursors() {
    let paths = test_paths("skill-utf8-pages");
    let system = paths.data_dir.join("skills-system");
    let package = system.join("paged");
    std::fs::create_dir_all(package.join("references")).unwrap();
    let body = "界".repeat(4_000);
    std::fs::write(
        package.join("SKILL.md"),
        "---\nname: paged\ndescription: UTF-8 paging\n---\nRead references/long.md.\n",
    )
    .unwrap();
    std::fs::write(package.join("references/long.md"), &body).unwrap();
    let skills = AgentSkillApplication::open(
        SkillRoots {
            system,
            user: paths.data_dir.join("skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("skills-state.json"),
        },
        None,
    );
    let entry = skills.list().unwrap().entries.remove(0);
    skills.enable(&entry.id).unwrap();
    let catalog = skills.turn_catalog().unwrap();
    let mounted = MountedSkills::mount(&skills).await;
    let skill = catalog.entries[0].id.clone();
    let reference = json!({ "skill": skill, "path": "references/long.md" });
    let first = mounted
        .call("skills_read", &catalog, reference.clone())
        .await
        .unwrap();
    let first = first.output.structured_content.unwrap();
    let cursor = first["next_cursor"].as_str().unwrap().to_string();
    let second = mounted
        .call(
            "skills_read",
            &catalog,
            json!({ "skill": skill, "path": "references/long.md", "cursor": cursor }),
        )
        .await
        .unwrap()
        .output
        .structured_content
        .unwrap();
    assert_eq!(
        format!(
            "{}{}",
            first["contents"].as_str().unwrap(),
            second["contents"].as_str().unwrap()
        ),
        body
    );
    let mut stale = cursor.clone();
    let replacement = if &stale[3..4] == "f" { "e" } else { "f" };
    stale.replace_range(3..4, replacement);
    assert!(mounted
        .call(
            "skills_read",
            &catalog,
            json!({ "skill": skill, "path": "references/long.md", "cursor": stale }),
        )
        .await
        .unwrap_err()
        .contains("snapshot"));
    // SKILL.md is delivered whole, as the Skill's context.
    assert!(mounted
        .call(
            "skills_read",
            &catalog,
            json!({ "skill": skill, "cursor": cursor }),
        )
        .await
        .unwrap_err()
        .contains("read whole"));
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn skill_resource_tool_rechecks_the_frozen_exact_resource_and_returns_a_safe_projection() {
    let paths = test_paths("skill-resource-tool");
    let bundled = paths.data_dir.join("skills-bundled");
    let package = bundled.join("reader");
    std::fs::create_dir_all(package.join("references")).unwrap();
    std::fs::write(
        package.join("SKILL.md"),
        "---\nname: reader\ndescription: Read a verified reference\n---\nUse the reference.\n",
    )
    .unwrap();
    std::fs::write(package.join("references/guide.md"), "verified guidance").unwrap();
    let skills = AgentSkillApplication::open(
        SkillRoots {
            system: bundled,
            user: paths.data_dir.join("skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("skills-state.json"),
        },
        None,
    );
    let entry = skills.list().unwrap().entries.remove(0);
    skills.enable(&entry.id).unwrap();
    let snapshot = skills.turn_catalog().unwrap();
    let resource = snapshot.entries[0].resources[0].clone();
    let call = json!({ "skill": snapshot.entries[0].id, "path": resource.path });
    let mounted = MountedSkills::mount(&skills).await;
    let output = mounted
        .call("skills_read", &snapshot, call)
        .await
        .unwrap()
        .output
        .structured_content
        .expect("reference output is structured");
    assert_eq!(output["contents"], "verified guidance");
    assert!(output["next_cursor"].is_null());
    assert!(output.get("relativePath").is_none());
    assert!(output.get("contentDigest").is_none());

    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn skill_image_asset_uses_the_existing_normalized_model_media_transport() {
    let paths = test_paths("skill-image-resource");
    let system = paths.data_dir.join("skills-system");
    let package = system.join("image-reader");
    std::fs::create_dir_all(package.join("assets")).unwrap();
    std::fs::write(
        package.join("SKILL.md"),
        "---\nname: image-reader\ndescription: Read an image\n---\nInspect the image.\n",
    )
    .unwrap();
    image::DynamicImage::new_rgb8(2, 2)
        .save(package.join("assets/sample.png"))
        .unwrap();
    let skills = AgentSkillApplication::open(
        SkillRoots {
            system,
            user: paths.data_dir.join("skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("skills-state.json"),
        },
        None,
    );
    let entry = skills.list().unwrap().entries.remove(0);
    skills.enable(&entry.id).unwrap();
    let snapshot = skills.turn_catalog().unwrap();
    let resource = snapshot.entries[0]
        .resources
        .iter()
        .find(|resource| resource.kind == SkillResourceKind::Asset)
        .unwrap();
    let call = json!({ "skill": snapshot.entries[0].id, "path": resource.path });
    let mounted = MountedSkills::mount(&skills).await;
    let output = mounted.call("skills_read", &snapshot, call).await.unwrap();
    assert_eq!(output.images.len(), 1);
    assert_eq!(output.images[0].mime_type, "image/png");
    assert_eq!(output.images[0].width, 2);
    assert_eq!(output.images[0].height, 2);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn non_image_skill_asset_returns_bounded_embedded_binary() {
    let paths = test_paths("skill-binary-asset");
    let system = paths.data_dir.join("system");
    let package = system.join("binary-reader");
    std::fs::create_dir_all(package.join("assets")).unwrap();
    std::fs::write(
        package.join("SKILL.md"),
        "---\nname: binary-reader\ndescription: Read a binary asset\n---\nInspect the asset.\n",
    )
    .unwrap();
    std::fs::write(package.join("assets/sample.bin"), [0_u8, 1, 2, 3]).unwrap();
    let skills = AgentSkillApplication::open(
        SkillRoots {
            system,
            user: paths.data_dir.join("skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("skills-state.json"),
        },
        None,
    );
    let entry = skills.list().unwrap().entries.remove(0);
    skills.enable(&entry.id).unwrap();
    let snapshot = skills.turn_catalog().unwrap();
    let resource = snapshot.entries[0]
        .resources
        .iter()
        .find(|resource| resource.kind == SkillResourceKind::Asset)
        .unwrap();
    let call = json!({ "skill": snapshot.entries[0].id, "path": resource.path });
    let mounted = MountedSkills::mount(&skills).await;
    let output = mounted.call("skills_read", &snapshot, call).await.unwrap();
    assert!(output.images.is_empty());
    assert!(matches!(
        output.output.content.as_slice(),
        [hanafubuki_agent_tool_contracts::ToolOutputContent::EmbeddedResource {
            mime_type: Some(mime_type),
            data: Some(data),
            ..
        }] if mime_type == "application/octet-stream" && data == "AAECAw=="
    ));
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn reading_an_unavailable_skill_says_what_it_is_missing() {
    let paths = test_paths("skill-unavailable-read");
    let skills = AgentSkillApplication::open(
        SkillRoots {
            system: paths.data_dir.join("skills-system"),
            user: paths.data_dir.join("skills-user"),
            install_bundled_system_skills: false,
            state_file: paths.data_dir.join("skills-state.json"),
        },
        None,
    );
    let mounted = MountedSkills::mount(&skills).await;
    let skill = SkillId::new(SkillScope::User, "remote").unwrap();
    let catalog = hanafubuki_agent_skills::SkillTurnCatalog {
        unavailable: vec![hanafubuki_agent_skills::SkillTurnUnavailable {
            id: skill.clone(),
            name: "remote".to_string(),
            reason: "MCP server `github` is missing: required MCP server is not configured"
                .to_string(),
        }],
        ..Default::default()
    };

    let error = mounted
        .call("skills_read", &catalog, json!({ "skill": skill }))
        .await
        .unwrap_err();

    assert_eq!(
        error,
        "Skill is unavailable this turn: MCP server `github` is missing: required MCP server is not configured"
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
