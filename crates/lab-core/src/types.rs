//! Extended domain types — LabSession, enums, metrics.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

// ─── Session ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    #[default]
    Active,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabSession {
    pub id: String,
    pub name: String,
    pub started_at: DateTime<Utc>,
    pub agents_active: usize,
    pub tasks_completed: usize,
    pub tasks_failed: usize,
    pub total_tokens_used: usize,
    #[serde(default)]
    pub status: SessionStatus,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl LabSession {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            started_at: Utc::now(),
            agents_active: 0,
            tasks_completed: 0,
            tasks_failed: 0,
            total_tokens_used: 0,
            status: SessionStatus::Active,
            artifacts: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    pub fn to_dict(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "started_at": self.started_at.to_rfc3339(),
            "agents_active": self.agents_active,
            "tasks_completed": self.tasks_completed,
            "tasks_failed": self.tasks_failed,
            "total_tokens_used": self.total_tokens_used,
            "status": format!("{:?}", self.status).to_lowercase(),
            "artifacts": self.artifacts,
            "metadata": self.metadata,
        })
    }
}

// ─── Agent Trait ────────────────────────────────────────────────────

use crate::config::AgentProfile;
use crate::errors::Result;
use async_trait::async_trait;

/// Trait that all research agents implement.
#[async_trait]
pub trait Agent: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn session_id(&self) -> &str;
    fn profile(&self) -> &AgentProfile;
    async fn start(&mut self) -> Result<AgentResult>;
    async fn run_task(&mut self, task: &str, kwargs: HashMap<String, serde_json::Value>) -> Result<AgentResult>;
    async fn cleanup(&mut self);
    fn state(&self) -> AgentState;
}

// ─── Agent State ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Initialized,
    Running,
    Paused,
    Waiting,
    Completed,
    Failed,
    Cancelled,
}

// ─── Agent Result ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct AgentResult {
    pub success: bool,
    #[serde(default)]
    pub data: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub metrics: AgentMetrics,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub reasoning_trace: Vec<String>,
}

impl AgentResult {
    pub fn ok(data: serde_json::Value) -> Self {
        Self {
            success: true,
            data,
            error: None,
            metrics: AgentMetrics::default(),
            artifacts: Vec::new(),
            reasoning_trace: Vec::new(),
        }
    }

    pub fn fail(error: impl Into<String>, data: Option<serde_json::Value>) -> Self {
        Self {
            success: false,
            data: data.unwrap_or(serde_json::json!({})),
            error: Some(error.into()),
            metrics: AgentMetrics::default(),
            artifacts: Vec::new(),
            reasoning_trace: Vec::new(),
        }
    }
}

// ─── Agent Metrics ──────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentMetrics {
    pub tool_calls: usize,
    pub tool_errors: usize,
    pub permission_checks: usize,
    pub permission_denials: usize,
    pub memory_reads: usize,
    pub memory_writes: usize,
    pub tokens_used: usize,
    pub iteration_count: usize,
    #[serde(skip)]
    pub start_time: Option<Instant>,
    #[serde(skip)]
    pub end_time: Option<Instant>,
}

impl AgentMetrics {
    pub fn elapsed_secs(&self) -> f64 {
        match (self.start_time, self.end_time) {
            (Some(s), Some(e)) => e.duration_since(s).as_secs_f64(),
            (Some(s), None) => s.elapsed().as_secs_f64(),
            _ => 0.0,
        }
    }
}

// ─── Operation Counts ───────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
pub struct OperationCounts {
    pub tool_calls: usize,
    pub agent_spawns: usize,
    pub pipeline_runs: usize,
    pub permission_checks: usize,
    pub permission_denied: usize,
    pub permission_approved: usize,
}

// ─── Lab Stats ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct LabStats {
    pub running: bool,
    pub uptime_seconds: f64,
    pub sessions: SessionStats,
    pub agents: HashMap<String, usize>,
    pub operations: OperationCounts,
    pub pipeline_configs: Vec<String>,
    pub memory_entries: usize,
    pub permission_policy: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionStats {
    pub total: usize,
    pub active: usize,
}
