//! ResearchLab — Master orchestrator for the AI Research Lab.
//! Mirrors core/lab/engine.py (460 lines) with full Python parity.

use crate::config::LabConfig;
use crate::container::{LabContainer, LabContainerBuilder};
use crate::errors::{LabError, Result};
use crate::events::EventBus;
use crate::events::LabEvent;
use crate::llm::LLMClient;
use crate::sessions::SessionStore;
use crate::types::*;
use crate::workflows::{
    register_builtin_templates, TemplateRegistry, Workflow, WorkflowEngine, WorkflowExecution,
};
use lab_memory::MemoryWorkspace;
use lab_permissions::PermissionEngine;
use lab_tools::ToolRegistry;
use std::collections::HashMap;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Master orchestrator for the AI Research Lab.
///
/// The lab manages:
/// - Sessions: Isolated work contexts with memory and state
/// - Agents: Specialized AI research agents (via trait objects)
/// - Tools: A registry of research tools
/// - Pipelines: Multi-stage research workflows
/// - Memory: Persistent workspace for agent knowledge
/// - Events: Pub/sub event system
/// - Permissions: Granular RBAC
/// - Workflows: DAG-based execution with templates
/// - LLM: Provider-agnostic LLM client
pub struct ResearchLab {
    pub config: LabConfig,
    sessions: HashMap<String, LabSession>,
    agent_registry: HashMap<String, HashMap<String, serde_json::Value>>, // session -> agent_id -> metadata
    tool_registry: ToolRegistry,
    permission_engine: PermissionEngine,
    event_bus: EventBus,
    memory: MemoryWorkspace,
    session_store: SessionStore,
    llm_client: Option<Box<dyn LLMClient>>,
    pipeline_configs: HashMap<String, serde_json::Value>,
    operation_counts: OperationCounts,
    running: bool,
    start_time: Option<Instant>,
}

impl ResearchLab {
    /// Create a new lab with the given configuration.
    pub fn new(config: LabConfig) -> Self {
        Self::from_container(LabContainerBuilder::new(config).build())
    }

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

        let mut lab = Self {
            config,
            sessions: HashMap::new(),
            agent_registry: HashMap::new(),
            tool_registry,
            permission_engine,
            event_bus,
            memory,
            session_store,
            llm_client,
            pipeline_configs: HashMap::new(),
            operation_counts: OperationCounts::default(),
            running: false,
            start_time: None,
        };

        // Subscribe all events for auditing
        let verbose = lab.config.verbose_logging;
        lab.event_bus.subscribe_all(move |event| {
            if verbose {
                debug!(
                    "[EVENT] {}: {}",
                    event.event_type,
                    serde_json::to_string(&event.data).unwrap_or_default()
                );
            }
        });

        lab
    }

    // ─── Session Management ─────────────────────────────────────

    /// Create a new isolated research session.
    pub async fn create_session(&mut self, name: &str) -> Result<&LabSession> {
        let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let session = LabSession::new(id, name.to_string());
        let session_id = session.id.clone();

        self.sessions.insert(session_id.clone(), session);

        self.event_bus.emit(LabEvent::new(
            "session.created",
            serde_json::json!({"session_id": session_id, "name": name}),
        ));

        info!("Session created: {} ({})", session_id, name);
        Ok(self.sessions.get(&session_id).unwrap())
    }

    /// Close a session and persist its artifacts.
    pub async fn close_session(&mut self, session_id: &str) -> Result<()> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| LabError::SessionNotFound(session_id.to_string()))?;
        session.status = SessionStatus::Completed;
        let _record = self.session_store.save(session, None, None)?;

        self.event_bus.emit(LabEvent::new(
            "session.closed",
            serde_json::json!({"session_id": session_id}),
        ));

        info!("Session closed: {} (record saved to disk)", session_id);
        Ok(())
    }

    /// List all sessions.
    pub fn list_sessions(&self) -> Vec<&LabSession> {
        self.sessions.values().collect()
    }

    /// Get a session by ID.
    pub fn get_session(&self, session_id: &str) -> Option<&LabSession> {
        self.sessions.get(session_id)
    }

    // ─── Agent Lifecycle ────────────────────────────────────────

    /// Register an agent with a session (stored as metadata).
    pub fn register_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
        agent_meta: serde_json::Value,
    ) {
        self.agent_registry
            .entry(session_id.to_string())
            .or_default()
            .insert(agent_id.to_string(), agent_meta);

        if let Some(session) = self.sessions.get_mut(session_id) {
            session.agents_active += 1;
        }
        self.operation_counts.agent_spawns += 1;
    }

    /// Remove an agent from a session.
    pub async fn remove_agent(&mut self, session_id: &str, agent_id: &str) -> bool {
        if let Some(agents) = self.agent_registry.get_mut(session_id) {
            if agents.remove(agent_id).is_some() {
                if let Some(session) = self.sessions.get_mut(session_id) {
                    session.agents_active = session.agents_active.saturating_sub(1);
                }
                self.event_bus.emit(LabEvent::new(
                    "agent.removed",
                    serde_json::json!({"agent_id": agent_id, "session_id": session_id}),
                ));
                return true;
            }
        }
        false
    }

    /// Get agent metadata.
    pub fn get_agent(&self, session_id: &str, agent_id: &str) -> Option<&serde_json::Value> {
        self.agent_registry
            .get(session_id)
            .and_then(|agents| agents.get(agent_id))
    }

    // ─── Pipeline Management ───────────────────────────────────

    /// Access the tool registry.
    pub fn tools(&self) -> &ToolRegistry {
        &self.tool_registry
    }

    /// Access the tool registry mutably.
    pub fn tools_mut(&mut self) -> &mut ToolRegistry {
        &mut self.tool_registry
    }

    /// Check if an agent can use a tool.
    pub async fn check_permission(
        &mut self,
        agent_id: &str,
        tool_name: &str,
        params: &HashMap<String, serde_json::Value>,
    ) -> serde_json::Value {
        self.operation_counts.permission_checks += 1;

        let result = self
            .permission_engine
            .check(agent_id, tool_name, params)
            .await;
        if result
            .get("allowed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            self.operation_counts.permission_approved += 1;
        } else {
            self.operation_counts.permission_denied += 1;
        }
        serde_json::to_value(result).unwrap_or_default()
    }

    /// Execute a tool with full permission checking.
    pub async fn execute_tool(
        &mut self,
        tool_name: &str,
        params: HashMap<String, serde_json::Value>,
        agent_id: Option<&str>,
    ) -> serde_json::Value {
        if let Some(aid) = agent_id {
            let perm = self.check_permission(aid, tool_name, &params).await;
            if !perm
                .get("allowed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return serde_json::json!({
                    "success": false,
                    "error": "permission_denied",
                    "message": perm.get("reason").cloned().unwrap_or(serde_json::Value::Null),
                });
            }
        }

        self.operation_counts.tool_calls += 1;
        self.tool_registry.execute(tool_name, &params).await
    }

    // ─── Pipeline Management ────────────────────────────────────

    /// Register a pipeline configuration.
    pub fn register_pipeline(&mut self, name: &str, config: serde_json::Value) {
        self.pipeline_configs.insert(name.to_string(), config);
    }

    pub async fn begin_pipeline_run(&mut self, pipeline_name: &str) -> Result<String> {
        self.operation_counts.pipeline_runs += 1;
        let session = self
            .create_session(&format!("pipeline-{}", pipeline_name))
            .await?;
        let session_id = session.id.clone();

        self.event_bus.emit(LabEvent::new(
            "pipeline.started",
            serde_json::json!({"pipeline": pipeline_name, "session_id": session_id}),
        ));

        Ok(session_id)
    }

    fn completed_pipeline_steps(result: &serde_json::Value) -> usize {
        result
            .get("stage_results")
            .and_then(|value| value.as_array())
            .map(|stages| {
                stages
                    .iter()
                    .filter(|stage| {
                        stage.get("status").and_then(|value| value.as_str()) == Some("completed")
                    })
                    .count()
            })
            .unwrap_or(0)
    }

    fn pipeline_result_field(result: &serde_json::Value, field: &str) -> serde_json::Value {
        result
            .get(field)
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }

    pub async fn finish_pipeline_run(
        &mut self,
        session_id: &str,
        pipeline_name: &str,
        result: &serde_json::Value,
    ) -> Result<()> {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.tasks_completed = Self::completed_pipeline_steps(result);
        }

        let _ = self.memory.store(
            session_id,
            "pipeline_result",
            result,
            Some(vec!["pipeline".into(), pipeline_name.to_string()]),
        );

        self.event_bus.emit(LabEvent::new(
            "pipeline.completed",
            serde_json::json!({
                "pipeline": pipeline_name,
                "session_id": session_id,
                "status": Self::pipeline_result_field(result, "status"),
                "output_path": Self::pipeline_result_field(result, "output_path"),
            }),
        ));

        self.close_session(session_id).await
    }

    /// Legacy lightweight pipeline path.
    ///
    /// The CLI and API now run real staged pipelines through `lab-pipelines`.
    /// This method remains for simple registered-pipeline bookkeeping.
    pub async fn run_pipeline(
        &mut self,
        pipeline_name: &str,
        _targets: Vec<String>,
    ) -> Result<serde_json::Value> {
        if !self.pipeline_configs.contains_key(pipeline_name) {
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

        self.finish_pipeline_run(&session_id, pipeline_name, &result)
            .await?;
        Ok(result)
    }

    // ─── Memory ──────────────────────────────────────────────────

    /// Access memory workspace.
    pub fn memory(&self) -> &MemoryWorkspace {
        &self.memory
    }

    /// Access memory workspace mutably.
    pub fn memory_mut(&mut self) -> &mut MemoryWorkspace {
        &mut self.memory
    }

    // ─── Events ──────────────────────────────────────────────────

    /// Access the event bus.
    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }

    /// Access the event bus mutably.
    pub fn event_bus_mut(&mut self) -> &mut EventBus {
        &mut self.event_bus
    }

    // ─── LLM ──────────────────────────────────────────────────────

    /// Check if LLM is configured.
    pub fn has_llm(&self) -> bool {
        self.llm_client.is_some() && self.config.llm_configured()
    }

    /// Get a reference to the LLM client (borrowed).
    pub fn llm_client(&self) -> Option<&dyn LLMClient> {
        self.llm_client.as_ref().map(|c| c.as_ref())
    }

    /// Get the configured model name.
    pub fn model(&self) -> &str {
        &self.config.model
    }

    /// Get the LLM error from the last operation (for debugging).
    pub fn llm_error(&self) -> Option<&str> {
        None // Placeholder for future error tracking
    }

    /// Send a prompt to the configured LLM.
    pub async fn ask_llm(
        &self,
        prompt: &str,
        system: &str,
        temperature: f64,
        max_tokens: u32,
    ) -> Option<String> {
        if !self.has_llm() {
            warn!("LLM not configured — ask_llm returning None");
            return None;
        }

        let client = self.llm_client.as_ref()?;
        let messages = vec![
            crate::llm::ChatMessage::system(system),
            crate::llm::ChatMessage::user(prompt),
        ];

        match client
            .chat(messages, &self.config.model, temperature, max_tokens)
            .await
        {
            Ok(resp) => Some(resp.content),
            Err(e) => {
                warn!("LLM call failed: {}", e);
                None
            }
        }
    }

    // ─── Workflow Integration ───────────────────────────────────

    /// Run a DAG-based workflow through the lab's task system.
    pub async fn run_workflow(
        &mut self,
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

        // Persist execution to session memory on success
        if let Some(sid) = session_id {
            if result.status == "completed" {
                let key = format!("wf_result_{}", result.execution_id);
                let _ = self.memory.store(
                    sid,
                    &key,
                    &result.to_dict(),
                    Some(vec!["workflow".to_string(), "result".to_string()]),
                );
            }
        }

        Ok(result)
    }

    /// Run a workflow from a built-in template.
    pub async fn run_workflow_from_template(
        &mut self,
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
        self.run_workflow(workflow, session_id, params, executor_fn)
            .await
    }

    /// Restore active sessions saved to disk on a previous run.
    /// Call this after `start()` to resume previous work.
    pub fn restore_sessions(&mut self) -> usize {
        match self.session_store.query(Some("active"), None, None, 0, 0) {
            Ok(records) => {
                let count = records.len();
                for record in records {
                    match record.to_session() {
                        Ok(session) if !self.sessions.contains_key(&session.id) => {
                            info!("Restored session: {} ({})", session.id, session.name);
                            self.sessions.insert(session.id.clone(), session);
                        }
                        _ => {}
                    }
                }
                if count > 0 {
                    info!("Restored {} active session(s) from disk", count);
                }
                count
            }
            Err(e) => {
                warn!("Failed to restore sessions: {}", e);
                0
            }
        }
    }

    /// Send a full message history to the configured LLM (for multi-turn chat).
    pub async fn ask_llm_messages(
        &self,
        messages: Vec<crate::llm::ChatMessage>,
        temperature: f64,
        max_tokens: u32,
    ) -> Option<String> {
        if !self.has_llm() {
            warn!("LLM not configured — ask_llm_messages returning None");
            return None;
        }
        let client = self.llm_client.as_ref()?;
        match client
            .chat(messages, &self.config.model, temperature, max_tokens)
            .await
        {
            Ok(resp) => Some(resp.content),
            Err(e) => {
                warn!("LLM chat failed: {}", e);
                None
            }
        }
    }

    // ─── Lifecycle ──────────────────────────────────────────────

    /// Initialize the lab and all subsystems.
    pub async fn start(&mut self) -> Result<()> {
        self.running = true;
        self.start_time = Some(Instant::now());

        self.event_bus.emit(LabEvent::new(
            "lab.started",
            serde_json::json!({"config": self.config.to_dict()}),
        ));

        self.config.ensure_directories();
        info!("Research Lab started");
        Ok(())
    }

    /// Gracefully shut down all lab subsystems.
    pub async fn shutdown(&mut self) -> Result<()> {
        self.running = false;

        // Close all sessions
        let session_ids: Vec<String> = self.sessions.keys().cloned().collect();
        for sid in session_ids {
            let _ = self.close_session(&sid).await;
        }

        // Clean up agents
        let agent_sessions: Vec<(String, Vec<String>)> = self
            .agent_registry
            .iter()
            .map(|(sid, agents)| (sid.clone(), agents.keys().cloned().collect()))
            .collect();
        for (sid, agent_ids) in agent_sessions {
            for aid in agent_ids {
                let _ = self.remove_agent(&sid, &aid).await;
            }
        }

        let uptime = self
            .start_time
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        info!("Research Lab shutdown (uptime: {:.1}s)", uptime);
        Ok(())
    }

    // ─── Status & Debugging ─────────────────────────────────────

    /// Get comprehensive lab statistics.
    pub fn get_stats(&self) -> LabStats {
        LabStats {
            running: self.running,
            uptime_seconds: self
                .start_time
                .map(|t| t.elapsed().as_secs_f64())
                .unwrap_or(0.0),
            sessions: SessionStats {
                total: self.sessions.len(),
                active: self
                    .sessions
                    .values()
                    .filter(|s| matches!(s.status, SessionStatus::Active))
                    .count(),
            },
            agents: self
                .agent_registry
                .iter()
                .map(|(k, v)| (k.clone(), v.len()))
                .collect(),
            operations: self.operation_counts.clone(),
            pipeline_configs: self.pipeline_configs.keys().cloned().collect(),
            memory_entries: self.memory.entry_count(),
            permission_policy: self
                .config
                .permission_policy
                .default_restriction_level
                .clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile;

    #[tokio::test]
    async fn lab_creates_session() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = LabConfig::with_workspace(temp_dir.path().to_path_buf());
        let mut lab = ResearchLab::new(config);
        lab.start().await.unwrap();

        let session = lab.create_session("test").await.unwrap();
        assert_eq!(session.name, "test");
        assert_eq!(lab.list_sessions().len(), 1);

        lab.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn lab_executes_tool() {
        let temp_dir = tempfile::tempdir().unwrap();
        let workspace = temp_dir.path().to_path_buf();
        let config = LabConfig::with_workspace(workspace.clone());
        let mut lab = ResearchLab::new(config);
        lab.start().await.unwrap();

        // Create a test file
        let test_file = workspace.join("test.txt");
        std::fs::write(&test_file, "hello world").unwrap();

        // Execute read_file tool
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
        let mut lab = ResearchLab::new(config);
        lab.start().await.unwrap();

        let session = lab.create_session("temp").await.unwrap();
        let session_id = session.id.clone();
        lab.close_session(&session_id).await.unwrap();

        let s = lab.get_session(&session_id).unwrap();
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
        let mut lab = ResearchLab::from_container(container);
        lab.start().await.unwrap();

        assert!(lab.tools().get("git_status").is_some());
        lab.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn lab_stats() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = LabConfig::with_workspace(temp_dir.path().to_path_buf());
        let mut lab = ResearchLab::new(config);
        lab.start().await.unwrap();

        lab.create_session("s1").await.unwrap();
        lab.create_session("s2").await.unwrap();

        let stats = lab.get_stats();
        assert!(stats.running);
        assert_eq!(stats.sessions.total, 2);
        assert_eq!(stats.sessions.active, 2);

        lab.shutdown().await.unwrap();
    }
}
