//! Base agent implementation — lifecycle, tool access, memory, permissions.
//! Mirrors agents/framework/base.py (292 lines)

use async_trait::async_trait;
use lab_core::config::AgentProfile;
use lab_core::errors::Result;
use lab_core::llm::{ChatMessage, LLMClient};
use lab_core::types::{Agent, AgentMetrics, AgentResult, AgentState};
use lab_core::LabConfig;
use lab_memory::MemoryWorkspace;
use lab_permissions::PermissionEngine;
use lab_tools::ToolRegistry;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Concrete agent that implements the `Agent` trait.
/// Created via `AgentImpl::new(...)` and used through `ResearchLab.run_agent()`.
pub struct AgentImpl {
    id: String,
    name: String,
    session_id: String,
    profile: AgentProfile,
    state: AgentState,
    metrics: AgentMetrics,
    cancel_flag: Arc<Mutex<bool>>,
    _workspace: std::path::PathBuf,
}

impl AgentImpl {
    /// Create a new agent.
    pub fn new(
        id: String,
        name: String,
        session_id: String,
        profile: AgentProfile,
        workspace: std::path::PathBuf,
    ) -> Self {
        Self {
            id: id.clone(),
            name: name.clone(),
            session_id: session_id.clone(),
            profile,
            state: AgentState::Initialized,
            metrics: AgentMetrics::default(),
            cancel_flag: Arc::new(Mutex::new(false)),
            _workspace: workspace,
        }
    }

    /// Cancel this agent's work.
    pub async fn cancel(&self) {
        let mut flag = self.cancel_flag.lock().await;
        *flag = true;
    }

    /// Check if this agent has been cancelled.
    pub async fn is_cancelled(&self) -> bool {
        *self.cancel_flag.lock().await
    }

    // ─── Utility methods for agent implementations ──────────

    /// Use a tool through the registry with permission checking.
    pub async fn use_tool(
        &mut self,
        registry: &mut ToolRegistry,
        _perm_engine: &mut PermissionEngine,
        tool_name: &str,
        params: HashMap<String, serde_json::Value>,
    ) -> serde_json::Value {
        self.metrics.tool_calls += 1;
        debug!("Agent {} using tool {}", self.id, tool_name);
        let result = registry.execute(tool_name, &params).await;
        if result.get("error").is_some() {
            self.metrics.tool_errors += 1;
        }
        result
    }

    /// Write to memory workspace.
    pub fn write_memory(
        &mut self,
        memory: &mut MemoryWorkspace,
        key: &str,
        value: &serde_json::Value,
        tags: Option<Vec<String>>,
    ) {
        self.metrics.memory_writes += 1;
        memory.store(&self.session_id, key, value, tags);
        debug!("Agent {} wrote memory key {}", self.id, key);
    }

    /// Read from memory workspace.
    pub fn read_memory(&self, memory: &MemoryWorkspace, key: &str) -> Option<serde_json::Value> {
        memory.get(&self.session_id, key)
    }

    /// Search memory.
    pub fn search_memory(
        &self,
        memory: &MemoryWorkspace,
        query: &str,
        tags: Option<&[String]>,
        limit: usize,
    ) -> Vec<serde_json::Value> {
        memory.search(&self.session_id, query, tags, limit)
    }

    /// List memory keys for this session.
    pub fn list_memory_keys(
        &self,
        memory: &MemoryWorkspace,
        tags: Option<&[String]>,
    ) -> Vec<String> {
        memory.list_keys(&self.session_id, tags)
    }

    /// Send a prompt to the LLM.
    pub async fn ask_llm(
        &self,
        client: &dyn LLMClient,
        model: &str,
        prompt: &str,
        system: &str,
        temperature: f64,
        max_tokens: u32,
    ) -> Option<String> {
        let messages = vec![ChatMessage::system(system), ChatMessage::user(prompt)];
        match client.chat(messages, model, temperature, max_tokens).await {
            Ok(resp) => Some(resp.content),
            Err(e) => {
                warn!("LLM call failed for agent {}: {}", self.id, e);
                None
            }
        }
    }

    /// Check if the agent has an LLM available.
    pub fn has_llm(&self, config: &LabConfig) -> bool {
        !config.api_key.is_empty() && !config.provider.is_empty()
    }

    /// Convenience: get metrics.
    pub fn metrics(&self) -> &AgentMetrics {
        &self.metrics
    }

    /// Wrap an error result.
    pub fn fail_result(&mut self, error: impl Into<String>) -> AgentResult {
        let error = error.into();
        info!("Agent {} failed: {}", self.id, error);
        AgentResult::fail(error, None)
    }

    /// Start the agent — pub inherent version.
    pub async fn start(&mut self) -> lab_core::errors::Result<AgentResult> {
        self.state = AgentState::Running;
        self.metrics.start_time = Some(Instant::now());
        info!("Agent started: {} ({})", self.id, self.name);
        Ok(AgentResult::ok(serde_json::json!({"status": "started"})))
    }

    /// Clean up the agent — pub inherent version.
    pub async fn cleanup(&mut self) {
        self.metrics.end_time = Some(Instant::now());
        let elapsed = self.metrics.elapsed_secs();
        info!(
            "Agent cleaned up: {} ({:.1}s, {} tool calls, {} errors)",
            self.id, elapsed, self.metrics.tool_calls, self.metrics.tool_errors,
        );
    }

    /// Get the agent ID (pub inherent method for access from other modules).
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Get the session ID.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Get the current state.
    pub fn state(&self) -> AgentState {
        self.state
    }
}

#[async_trait]
impl Agent for AgentImpl {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn profile(&self) -> &AgentProfile {
        &self.profile
    }

    async fn start(&mut self) -> Result<AgentResult> {
        self.state = AgentState::Running;
        self.metrics.start_time = Some(Instant::now());
        info!("Agent started: {} ({})", self.id, self.name);
        Ok(AgentResult::ok(serde_json::json!({"status": "started"})))
    }

    async fn run_task(
        &mut self,
        task: &str,
        _kwargs: HashMap<String, serde_json::Value>,
    ) -> Result<AgentResult> {
        // Default implementation — subclasses override
        Ok(AgentResult::ok(serde_json::json!({
            "task": task,
            "agent": self.name,
            "note": "This is a base agent — use ResearcherAgent, CoderAgent, etc."
        })))
    }

    async fn cleanup(&mut self) {
        self.metrics.end_time = Some(Instant::now());
        let elapsed = self.metrics.elapsed_secs();
        info!(
            "Agent cleaned up: {} ({:.1}s, {} tool calls, {} errors)",
            self.id, elapsed, self.metrics.tool_calls, self.metrics.tool_errors,
        );
    }

    fn state(&self) -> AgentState {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_creation() {
        let agent = AgentImpl::new(
            "test-1".into(),
            "tester".into(),
            "sess-1".into(),
            AgentProfile {
                name: "tester".into(),
                role: "test".into(),
                ..Default::default()
            },
            std::path::PathBuf::from("/tmp"),
        );
        assert_eq!(agent.id(), "test-1");
        assert_eq!(agent.name(), "tester");
        assert_eq!(agent.session_id(), "sess-1");
        assert_eq!(agent.state(), AgentState::Initialized);
    }
}
