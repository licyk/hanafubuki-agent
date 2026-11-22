use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hanafubuki_agent_kernel::AgentEventKind;
use serde_json::json;

use super::service::AgentMcpApplication;
use super::skills::AgentSkillApplication;
use super::*;
use crate::test_support::*;
use hanafubuki_agent_runtime::{ContainedRuntime, RuntimeTaskFailure};

mod fixtures;

use fixtures::*;
use sessions::approved_command;
use tools::{configured_service, service_with, service_with_config};

mod advisor;
mod background_terminals;
mod base_instructions;
mod colour;
mod compaction_golden;
mod compression;
mod compression_golden;
mod context;
mod context_usage;
mod context_usage_live;
mod descriptors;
mod executions;
mod features;
mod gemini;
mod goal;
mod goal_display;
mod golden;
mod golden_snapshot;
#[cfg(unix)]
mod hooks;
mod image_storage;
mod memory;
mod model_discovery;
mod model_list;
mod model_reasoning;
mod model_vision;
mod model_vision_live;
mod network_proxy;
mod packages;
mod patch_lines;
#[cfg(unix)]
mod pipeline;
mod plugin_faults;
mod plugin_host;
mod quarantine;
mod restart;
mod session_workspaces;
mod sessions;
mod settings_catalog;
mod settings_collections;
mod settings_environment;
mod shutdown;
mod standalone;
mod startup;
mod structured_output;
mod titles;
mod tool_list;
mod tool_search_live;
mod tools;
mod user_shell;
mod wake;
mod wake_golden;
mod workspace;
mod worktrees;
mod world_state;
