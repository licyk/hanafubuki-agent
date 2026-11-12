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

mod base_instructions;
#[cfg(target_os = "linux")]
mod compaction_golden;
mod compression;
#[cfg(target_os = "linux")]
mod compression_golden;
mod descriptors;
mod executions;
mod features;
mod goal;
#[cfg(target_os = "linux")]
mod golden;
#[cfg(unix)]
mod hooks;
mod memory;
mod model_discovery;
mod model_reasoning;
mod model_vision;
mod network_proxy;
mod packages;
#[cfg(unix)]
mod pipeline;
mod plugin_faults;
mod plugin_host;
mod quarantine;
mod restart;
mod sessions;
mod settings_catalog;
mod shutdown;
mod standalone;
mod startup;
mod titles;
mod tools;
mod workspace;
mod worktrees;
mod world_state;
