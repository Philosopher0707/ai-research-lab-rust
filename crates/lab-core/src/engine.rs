//! ResearchLab — thin shim over `Arc<LabRuntime>`.
//!
//! All mutable state lives in the runtime's services behind their own locks,
//! so every method here takes `&self`. Cloning a `ResearchLab` is O(1) and
//! gives a second handle to the same underlying runtime.

use crate::config::LabConfig;
use crate::container::{LabContainer, LabContainerBuilder};
use crate::errors::{LabError, Result};
use crate::events::EventBus;
use crate::events::LabEvent;
use crate::llm::LLMClient;
use crate::llm::ChatMessage;
use crate::runtime::LabRuntime;
use crate::types::*;
use crate::workflows::{
    register_builtin_templates, TemplateRegistry, Workflow, WorkflowEngine, WorkflowExecution,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::info;

/// Master orchestrator for the AI Research Lab.
///
/// Backed by `Arc<LabRuntime>` — cheap to clone, all methods are `&self`.
#[derive(Clone)]
pub struct ResearchLab {
    pub config: LabConfig,
    pub(crate) runtime: Arc<LabRuntime>,
}

impl ResearchLab {
    pub fn new(config: LabConfig) -> Self {
        Self::from_container(LabContainerBuilder::new(config).build())
    }

    pub fn from_container(container: LabContainer) -> Self {
        let config = container.config.clone();
        let runtime = Arc::new(LabRuntime::from_container(container));
        Self { config, runtime }
    }

    // ─── Session Management ─────────────────────────────────────

    /// Create a new isolated research session, returning an owned clone.
    pub async fn create_session(&self, name: &str) -> Result<LabSession> {
        self.runtime.sessions.create(name).await
    }

    /// Close a session and persist its artifacts.
    pub async fn close_session(&self, session_id: &str) -> Result<()> {
        self.runtime.sessions.close(session_id).await
    }

    /// List all sessions (owned clones).
    pub async fn list_sessions(&self) -> Vec<LabSession> {
        self.runtime.sessions.list().await
    }

    /// Get a session by ID (owned clone).
    pub async fn get_session(&self, session_id: &str) -> Option<LabSession> {
        self.runtime.sessions.get(session_id).await
    }

    // ─── Agent Lifecycle ────────────────────────────────────────

    pub async fn register_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        agent_meta: serde_json::Value,
    ) {
        self.runtime.sessions.register_agent(session_id, agent_id, agent_meta).await;
        self.runtime.counts.lock().unwrap().agent_spawns += 1;
    }

    pub async fn remove_agent(&self, session_id: &str, agent_id: &str) -> bool {
        self.runtime.sessions.remove_agent(session_id, agent_id).await
    }

    pub async fn get_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<serde_json::Value> {
        self.runtime.sessions.get_agent(session_id, agent_id).await
    }

    // ─── Tool access ────────────────────────────────────────────

    /// Direct access to the `ToolService`.
    pub fn tools(&self) -> &crate::services::ToolService {
        &self.runtime.tools
    }

    /// Call `f` with a read-only reference to the `ToolRegistry`.
    pub async fn with_tools<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&lab_tools::ToolRegistry) -> R,
    {
        self.runtime.tools.with_registry(f).await
    }

    /// Check if a tool exists.  Convenience wrapper over `with_tools`.
    pub async fn tool_exists(&self, name: &str) -> bool {
        self.runtime.tools.with_registry(|r| r.get(name).is_some()).await
    }

    /// List tools as JSON metadata.
    pub async fn list_tools(&self) -> Vec<serde_json::Value> {
        self.runtime.tools.with_registry(|r| r.list_tools()).await
    }

    /// Check if an agent can use a tool.
    pub async fn check_permission(
        &self,
        agent_id: &str,
        tool_name: &str,
        params: &HashMap<String, serde_json::Value>,
    ) -> serde_json::Value {
        // Increment and drop the lock *before* the async permission check.
        { self.runtime.counts.lock().unwrap().permission_checks += 1; }

        let result = self.runtime.tools.check_permission(agent_id, tool_name, params).await;
        let allowed = result.get("allowed").and_then(|v| v.as_bool()).unwrap_or(false);

        {
            let mut counts = self.runtime.counts.lock().unwrap();
            if allowed { counts.permission_approved += 1; } else { counts.permission_denied += 1; }
        }

        serde_json::to_value(result).unwrap_or_default()
    }

    /// Execute a tool with optional permission checking.
    pub async fn execute_tool(
        &self,
        tool_name: &str,
        params: HashMap<String, serde_json::Value>,
        agent_id: Option<&str>,
    ) -> serde_json::Value {
        self.runtime.counts.lock().unwrap().tool_calls += 1;
        self.runtime.tools.execute(tool_name, params, agent_id).await
    }

    // ─── Pipeline Management ────────────────────────────────────

    pub fn register_pipeline(&self, name: &str, config: serde_json::Value) {
        self.runtime.pipeline_configs.write().unwrap().insert(name.to_string(), config);
    }

    pub async fn begin_pipeline_run(&self, pipeline_name: &str) -> Result<String> {
        self.runtime.counts.lock().unwrap().pipeline_runs += 1;
        let session = self.create_session(&format!("pipeline-{}", pipeline_name)).await?;
        let session_id = session.id.clone();

        self.runtime.event_bus.emit(LabEvent::new(
            "pipeline.started",
            serde_json::json!({"pipeline": pipeline_name, "session_id": session_id}),
        ));

        Ok(session_id)
    }

    pub async fn finish_pipeline_run(
        &self,
        session_id: &str,
        pipeline_name: &str,
        result: &serde_json::Value,
    ) -> Result<()> {
        let steps = result
            .get("stage_results")
            .and_then(|v| v.as_array())
            .map(|stages| {
                stages
                    .iter()
                    .filter(|s| s.get("status").and_then(|v| v.as_str()) == Some("completed"))
                    .count()
            })
            .unwrap_or(0);
        self.runtime.sessions.set_tasks_completed(session_id, steps).await;

        self.runtime.memory.store(
            session_id,
            "pipeline_result",
            result,
            Some(vec!["pipeline".into(), pipeline_name.to_string()]),
        );

        self.runtime.event_bus.emit(LabEvent::new(
            "pipeline.completed",
            serde_json::json!({
                "pipeline": pipeline_name,
                "session_id": session_id,
                "status": result.get("status").cloned().unwrap_or_default(),
                "output_path": result.get("output_path").cloned().unwrap_or_default(),
            }),
        ));

        self.close_session(session_id).await
    }

    /// Legacy lightweight pipeline path.
    pub async fn run_pipeline(
        &self,
        pipeline_name: &str,
        _targets: Vec<String>,
    ) -> Result<serde_json::Value> {
        if !self.runtime.pipeline_configs.read().unwrap().contains_key(pipeline_name) {
            return Err(LabError::PipelineError {
                pipeline: pipeline_name.to_string(),
                error: format!("Pipeline '{}' not registered", pipeline_name),
            });
        }

        let session_id = self.begin_pipeline_run(pipeline_name).await?;
        let result = serde_json::json!({
            "pipeline": pipeline_name,
            "session_id": session_id,
            "status": "completed",
        });
        self.finish_pipeline_run(&session_id, pipeline_name, &result).await?;
        Ok(result)
    }

    // ─── Memory ──────────────────────────────────────────────────

    /// Direct access to the `MemoryService` — useful when multiple memory ops
    /// are needed in a row (avoids re-acquiring the runtime's Arc each call).
    pub fn memory(&self) -> &crate::services::MemoryService {
        &self.runtime.memory
    }

    pub fn store_memory(
        &self,
        session_id: &str,
        key: &str,
        value: &serde_json::Value,
        tags: Option<Vec<String>>,
    ) -> lab_memory::MemoryEntry {
        self.runtime.memory.store(session_id, key, value, tags)
    }

    pub fn get_memory(&self, session_id: &str, key: &str) -> Option<serde_json::Value> {
        self.runtime.memory.get(session_id, key)
    }

    pub fn list_memory_keys(
        &self,
        session_id: &str,
        tags: Option<&[String]>,
    ) -> Vec<String> {
        self.runtime.memory.list_keys(session_id, tags)
    }

    pub fn search_memory(
        &self,
        session_id: &str,
        query: &str,
        tags: Option<&[String]>,
        limit: usize,
    ) -> Vec<serde_json::Value> {
        self.runtime.memory.search(session_id, query, tags, limit)
    }

    // ─── Events ──────────────────────────────────────────────────

    pub fn event_bus(&self) -> &EventBus {
        &self.runtime.event_bus
    }

    pub fn event_bus_arc(&self) -> Arc<EventBus> {
        Arc::clone(&self.runtime.event_bus)
    }

    // ─── LLM ──────────────────────────────────────────────────────

    pub fn has_llm(&self) -> bool {
        self.runtime.llm.has_client() && self.config.llm_configured()
    }

    pub fn llm_client(&self) -> Option<&dyn LLMClient> {
        self.runtime.llm.client()
    }

    pub fn model(&self) -> &str {
        self.runtime.llm.model()
    }

    pub fn llm_error(&self) -> Option<&str> {
        None
    }

    pub async fn ask_llm(
        &self,
        prompt: &str,
        system: &str,
        temperature: f64,
        max_tokens: u32,
    ) -> Option<String> {
        if !self.has_llm() {
            return None;
        }
        self.runtime.llm.ask(prompt, system, temperature, max_tokens).await
    }

    pub async fn ask_llm_messages(
        &self,
        messages: Vec<ChatMessage>,
        temperature: f64,
        max_tokens: u32,
    ) -> Option<String> {
        if !self.has_llm() {
            return None;
        }
        self.runtime.llm.ask_messages(messages, temperature, max_tokens).await
    }

    // ─── Workflow Integration ───────────────────────────────────

    pub async fn run_workflow(
        &self,
        workflow: Workflow,
        session_id: Option<&str>,
        params: HashMap<String, serde_json::Value>,
        executor_fn: impl Fn(
                &crate::workflows::WorkflowStep,
                &HashMap<String, serde_json::Value>,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<serde_json::Value>> + Send>,
            > + Send
            + Sync,
    ) -> Result<WorkflowExecution> {
        let engine = WorkflowEngine::new();
        let wf = workflow.clone();
        wf.validate()?;

        let result = engine.execute(wf, executor_fn, session_id, params).await?;

        if let Some(sid) = session_id {
            if result.status == "completed" {
                let key = format!("wf_result_{}", result.execution_id);
                self.runtime.memory.store(
                    sid,
                    &key,
                    &result.to_dict(),
                    Some(vec!["workflow".to_string(), "result".to_string()]),
                );
            }
        }

        Ok(result)
    }

    pub async fn run_workflow_from_template(
        &self,
        template_name: &str,
        workflow_name: &str,
        session_id: Option<&str>,
        params: HashMap<String, serde_json::Value>,
        executor_fn: impl Fn(
                &crate::workflows::WorkflowStep,
                &HashMap<String, serde_json::Value>,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<serde_json::Value>> + Send>,
            > + Send
            + Sync,
    ) -> Result<WorkflowExecution> {
        let mut registry = TemplateRegistry::new();
        register_builtin_templates(&mut registry);

        let template = registry.get(template_name).ok_or_else(|| {
            let available: Vec<&str> = registry
                .list_templates()
                .iter()
                .map(|t| t.name.as_str())
                .collect();
            LabError::WorkflowError(format!(
                "Template '{}' not found. Available: {:?}",
                template_name, available
            ))
        })?;

        let workflow = template.instantiate(workflow_name, Some(&params));
        self.run_workflow(workflow, session_id, params, executor_fn).await
    }

    // ─── Lifecycle ──────────────────────────────────────────────

    pub async fn start(&self) -> Result<()> {
        *self.runtime.running.lock().unwrap() = true;
        *self.runtime.start_time.lock().unwrap() = Some(Instant::now());

        self.runtime.event_bus.emit(LabEvent::new(
            "lab.started",
            serde_json::json!({"config": self.config.to_dict()}),
        ));

        self.config.ensure_directories();
        info!("Research Lab started");
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<()> {
        *self.runtime.running.lock().unwrap() = false;

        let session_ids = self.runtime.sessions.all_ids().await;
        for sid in session_ids {
            let _ = self.close_session(&sid).await;
        }

        let uptime = self
            .runtime
            .start_time
            .lock()
            .unwrap()
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        info!("Research Lab shutdown (uptime: {:.1}s)", uptime);
        Ok(())
    }

    pub async fn restore_sessions(&self) -> usize {
        self.runtime.sessions.restore().await
    }

    // ─── Status & Debugging ─────────────────────────────────────

    pub async fn get_stats(&self) -> LabStats {
        let running = *self.runtime.running.lock().unwrap();
        let uptime = self
            .runtime
            .start_time
            .lock()
            .unwrap()
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        let sessions_total = self.runtime.sessions.count().await;
        let sessions_active = self.runtime.sessions.active_count().await;
        let agents = self.runtime.sessions.agent_counts().await;
        let operations = self.runtime.counts.lock().unwrap().clone();
        let pipeline_configs = self.runtime.pipeline_configs.read().unwrap().keys().cloned().collect();
        let memory_entries = self.runtime.memory.entry_count();
        let permission_policy = self.config.permission_policy.default_restriction_level.clone();

        LabStats {
            running,
            uptime_seconds: uptime,
            sessions: SessionStats { total: sessions_total, active: sessions_active },
            agents,
            operations,
            pipeline_configs,
            memory_entries,
            permission_policy,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lab_memory::MemoryWorkspace;
    use lab_tools::ToolRegistry;

    #[tokio::test]
    async fn lab_creates_session() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = LabConfig::with_workspace(temp_dir.path().to_path_buf());
        let lab = ResearchLab::new(config);
        lab.start().await.unwrap();

        let session = lab.create_session("test").await.unwrap();
        assert_eq!(session.name, "test");
        assert_eq!(lab.list_sessions().await.len(), 1);

        lab.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn lab_executes_tool() {
        let temp_dir = tempfile::tempdir().unwrap();
        let workspace = temp_dir.path().to_path_buf();
        let config = LabConfig::with_workspace(workspace.clone());
        let lab = ResearchLab::new(config);
        lab.start().await.unwrap();

        let test_file = workspace.join("test.txt");
        std::fs::write(&test_file, "hello world").unwrap();

        let result = lab
            .execute_tool(
                "read_file",
                HashMap::from([("path".to_string(), serde_json::json!("test.txt"))]),
                None,
            )
            .await;
        assert!(result["success"].as_bool().unwrap_or(false));
        assert!(result["data"]["content"].as_str().is_some());

        lab.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn lab_close_session() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = LabConfig::with_workspace(temp_dir.path().to_path_buf());
        let lab = ResearchLab::new(config);
        lab.start().await.unwrap();

        let session = lab.create_session("temp").await.unwrap();
        let session_id = session.id.clone();
        lab.close_session(&session_id).await.unwrap();

        let s = lab.get_session(&session_id).await.unwrap();
        assert!(matches!(s.status, SessionStatus::Completed));

        lab.shutdown().await.unwrap();
    }

    #[test]
    fn lab_llm_state_matches_config() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = LabConfig::with_workspace(temp_dir.path().to_path_buf());
        let lab = ResearchLab::new(config);
        assert_eq!(lab.has_llm(), lab.config.llm_configured());
    }

    #[tokio::test]
    async fn lab_can_start_from_injected_container() {
        let temp_dir = tempfile::tempdir().unwrap();
        let workspace = temp_dir.path().to_path_buf();
        let config = LabConfig::with_workspace(workspace.clone());
        let custom_memory = MemoryWorkspace::new(workspace.join("custom-memory"));
        let mut custom_tools = ToolRegistry::new(workspace.clone());
        custom_tools.register_builtins();

        let container = LabContainerBuilder::new(config)
            .with_memory(custom_memory)
            .with_tool_registry(custom_tools)
            .build();
        let lab = ResearchLab::from_container(container);
        lab.start().await.unwrap();

        assert!(lab.with_tools(|r| r.get("git_status").is_some()).await);
        lab.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn lab_stats() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = LabConfig::with_workspace(temp_dir.path().to_path_buf());
        let lab = ResearchLab::new(config);
        lab.start().await.unwrap();

        lab.create_session("s1").await.unwrap();
        lab.create_session("s2").await.unwrap();

        let stats = lab.get_stats().await;
        assert!(stats.running);
        assert_eq!(stats.sessions.total, 2);
        assert_eq!(stats.sessions.active, 2);

        lab.shutdown().await.unwrap();
    }
}
