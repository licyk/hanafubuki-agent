//! Read mention capabilities off the UI thread, without connecting disabled servers.

use super::{AgentBridge, AgentReply, TurnUpdate};
use crate::bottom_pane::mentions::{Candidate, MentionType, Selection};
use crate::mentions::Catalog;
use hanafubuki_agent::AgentSessionListRequest;
use std::sync::atomic::Ordering;
use std::time::Duration;

impl AgentBridge {
    pub(super) fn load_mentions(&self) {
        let session = self.session_id();
        self.spawn(move |agent| async move {
            let statuses = agent.plugin_status();
            let enabled = |name: &str| {
                statuses
                    .iter()
                    .any(|status| status.name == name && status.enabled)
            };
            let skills = if enabled("skills") {
                match session {
                    Some(id) => agent.list_skills_for_session(id),
                    None => agent.list_skills(),
                }?
                .entries
            } else {
                Vec::new()
            };
            let plugins = if enabled("plugins") {
                agent.packages().await?
            } else {
                Vec::new()
            };
            let mut servers: Vec<String> = if enabled("mcp") {
                agent
                    .mcp_local_status(false)
                    .await?
                    .servers
                    .into_iter()
                    .filter(|s| s.enabled)
                    .map(|s| s.server_id.as_str().to_owned())
                    .collect()
            } else {
                Vec::new()
            };
            if enabled("mcp") {
                servers.extend(
                    plugins
                        .iter()
                        .filter(|plugin| plugin.enabled)
                        .flat_map(|plugin| plugin.content.mcp_servers.iter().cloned()),
                );
                servers.sort();
                servers.dedup();
            }
            Ok(Some(AgentReply::Update(Box::new(TurnUpdate::Mentions {
                session_id: session,
                catalog: Catalog::build(skills, plugins, servers),
            }))))
        });
    }

    pub(super) fn search_mentions(&self, query: String) {
        let current = self.session_id();
        let pending = self.mention_search.clone();
        let generation = pending.fetch_add(1, Ordering::Relaxed) + 1;
        if query.trim().is_empty() {
            return;
        }
        self.spawn(move |agent| async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if pending.load(Ordering::Relaxed) != generation {
                return Ok(None);
            }
            if !agent
                .plugin_status()
                .iter()
                .any(|status| status.name == "threads" && status.enabled)
            {
                return Ok(None);
            }
            let page = agent
                .list_sessions_page(AgentSessionListRequest {
                    search: Some(query.clone()),
                    page_size: 50,
                    ..Default::default()
                })
                .map_err(|e| e.to_string())?;
            let candidates = page
                .sessions
                .into_iter()
                .filter(|s| Some(s.session_id) != current)
                .map(|s| {
                    let title = s
                        .title
                        .map(|title| title.value)
                        .unwrap_or_else(|| format!("Session {}", s.session_id));
                    let title = title
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .chars()
                        .take(160)
                        .collect::<String>();
                    Candidate {
                        display_name: title.clone(),
                        description: s.cwd,
                        search_terms: vec![title.clone()],
                        mention_type: MentionType::Task,
                        selection: Selection::Tool {
                            insert_text: format!("@{title}"),
                            path: Some(format!("thread://{}", s.session_id)),
                        },
                    }
                })
                .collect();
            Ok(Some(AgentReply::Update(Box::new(
                TurnUpdate::MentionTasks {
                    session_id: current,
                    query,
                    candidates,
                },
            ))))
        });
    }
}
