//! All error types for the AI Research Lab.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LabError {
    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Session already exists: {0}")]
    SessionExists(String),

    #[error("Agent not found: {agent_id} in session {session_id}")]
    AgentNotFound { agent_id: String, session_id: String },

    #[error("Permission denied: {reason}")]
    PermissionDenied { reason: String },

    #[error("Tool execution failed: {tool}: {error}")]
    ToolError { tool: String, error: String },

    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    #[error("Task failed: {id}: {error}")]
    TaskError { id: String, error: String },

    #[error("Task timeout after {0}s")]
    TaskTimeout(u64),

    #[error("Workflow execution failed: {0}")]
    WorkflowError(String),

    #[error("Pipeline failed: {pipeline}: {error}")]
    PipelineError { pipeline: String, error: String },

    #[error("Agent error: {0}")]
    AgentError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("LLM request failed: {0}")]
    LLMError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Parse error: {0}")]
    ParseError(String),
}

pub type Result<T> = std::result::Result<T, LabError>;
