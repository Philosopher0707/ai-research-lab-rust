//! LabRuntime — the interior of `ResearchLab`.
//!
//! `ResearchLab` is a thin `Arc<LabRuntime>` shim, so cloning the lab is O(1).
//! All mutable state lives in the services, which carry their own locks.

use crate::config::LabConfig;
use crate::container::LabContainer;
use crate::events::EventBus;
use crate::services::{LlmService, MemoryService, SessionService, ToolService};
use crate::types::OperationCounts;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

pub struct LabRuntime {
    pub config: LabConfig,
    pub sessions: Arc<SessionService>,
    pub llm: Arc<LlmService>,
    pub tools: Arc<ToolService>,
    pub memory: Arc<MemoryService>,
    pub event_bus: Arc<EventBus>,
    /// Registered pipeline configurations (name → JSON config).
    pub pipeline_configs: RwLock<std::collections::HashMap<String, serde_json::Value>>,
    /// Aggregate operation counters; updated by `ResearchLab` shim methods.
    pub counts: Mutex<OperationCounts>,
    pub running: Mutex<bool>,
    pub start_time: Mutex<Option<Instant>>,
}

impl LabRuntime {
    pub fn from_container(container: LabContainer) -> Self {
        let LabContainer {
            config,
            memory,
            tool_registry,
            permission_engine,
            session_store,
            event_bus,
            llm_client,
        } = container;

        let llm_model = config.model.clone();

        Self {
            sessions: Arc::new(SessionService::new(session_store, Arc::clone(&event_bus))),
            llm: Arc::new(LlmService::new(llm_client, llm_model)),
            tools: Arc::new(ToolService::new(tool_registry, permission_engine)),
            memory: Arc::new(MemoryService::new(memory)),
            event_bus,
            pipeline_configs: RwLock::new(std::collections::HashMap::new()),
            counts: Mutex::new(OperationCounts::default()),
            running: Mutex::new(false),
            start_time: Mutex::new(None),
            config,
        }
    }
}
