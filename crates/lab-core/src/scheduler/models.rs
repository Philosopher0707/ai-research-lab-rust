//! Task models — TaskSpec, priorities, statuses, and types.
//! Mirrors core/scheduler/models.py

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid;

// ─── Priority ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum TaskPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl Default for TaskPriority {
    fn default() -> Self {
        Self::Normal
    }
}

// ─── Status ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Scheduled,
    Running,
    Completed,
    Failed,
    Cancelled,
    Retrying,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::Scheduled => write!(f, "scheduled"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Retrying => write!(f, "retrying"),
        }
    }
}

// ─── Task Type ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    Research,
    Review,
    CodeGen,
    Summary,
    #[default]
    Custom,
    Pipeline,
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

// ─── TaskSpec ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    pub id: String,
    #[serde(default)]
    pub task_type: TaskType,
    #[serde(default)]
    pub task_name: String,
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_params: HashMap<String, serde_json::Value>,
    pub agent_class: Option<String>,
    #[serde(default)]
    pub task_string: String,
    #[serde(default)]
    pub priority: TaskPriority,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    pub schedule_at: Option<f64>,
    pub recurring: Option<String>,
    #[serde(skip, default = "default_instant")]
    pub created_at: std::time::Instant,
    #[serde(skip, default = "default_instant_opt")]
    pub started_at: Option<std::time::Instant>,
    #[serde(skip, default = "default_instant_opt")]
    pub completed_at: Option<std::time::Instant>,
    #[serde(default = "default_queued")]
    pub status: TaskStatus,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub retries: u32,
    #[serde(default)]
    pub assigned_agent_id: Option<String>,
}

fn default_max_retries() -> u32 {
    2
}
fn default_timeout() -> u64 {
    1800
}
fn default_queued() -> TaskStatus {
    TaskStatus::Queued
}
fn default_instant() -> std::time::Instant {
    std::time::Instant::now()
}
fn default_instant_opt() -> Option<std::time::Instant> {
    None
}

impl Default for TaskSpec {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string()[..10].to_string(),
            task_type: TaskType::Custom,
            task_name: String::new(),
            tool_name: None,
            tool_params: HashMap::new(),
            agent_class: None,
            task_string: String::new(),
            priority: TaskPriority::Normal,
            session_id: String::new(),
            tags: Vec::new(),
            dependencies: Vec::new(),
            max_retries: 2,
            timeout_seconds: 1800,
            schedule_at: None,
            recurring: None,
            created_at: std::time::Instant::now(),
            started_at: None,
            completed_at: None,
            status: TaskStatus::Queued,
            result: None,
            error: String::new(),
            retries: 0,
            assigned_agent_id: None,
        }
    }
}

impl TaskSpec {
    pub fn new(tool_name: impl Into<String>, params: HashMap<String, serde_json::Value>) -> Self {
        Self {
            tool_name: Some(tool_name.into()),
            tool_params: params,
            ..Default::default()
        }
    }

    pub fn with_priority(mut self, priority: TaskPriority) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = session_id.into();
        self
    }

    pub fn with_dependencies(mut self, deps: Vec<String>) -> Self {
        self.dependencies = deps;
        self
    }

    pub fn to_dict(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "task_type": format!("{:?}", self.task_type).to_lowercase(),
            "task_name": self.task_name,
            "priority": self.priority as u8,
            "status": self.status.to_string(),
            "session_id": self.session_id,
            "tags": self.tags,
            "dependencies": self.dependencies,
            "max_retries": self.max_retries,
            "timeout_seconds": self.timeout_seconds,
            "retries": self.retries,
            "result": self.result.as_ref().map(|v| v.to_string().chars().take(500).collect::<String>()),
            "error": self.error,
            "assigned_agent_id": self.assigned_agent_id,
        })
    }
}

// ─── Task Queue Stats ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct TaskQueueStats {
    pub total: usize,
    pub running: usize,
    pub queue_size: usize,
    pub by_status: HashMap<String, usize>,
    pub by_type: HashMap<String, usize>,
}
