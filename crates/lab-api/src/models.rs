use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionResponse {
    pub id: String,
    pub name: String,
    pub status: String,
    pub agents_active: usize,
    pub tasks_completed: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolRequest {
    pub tool_name: String,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
}

pub type ToolResponse = serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryRequest {
    pub session_id: String,
    pub key: String,
    pub value: serde_json::Value,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryResponse {
    pub key: String,
    pub value: serde_json::Value,
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PipelineRunRequest {
    pub pipeline_name: String,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub no_review: bool,
    #[serde(default)]
    pub no_code: bool,
    #[serde(default)]
    pub output_path: Option<String>,
}

impl PipelineRunRequest {
    pub fn into_execution_request(self) -> lab_pipelines::ExecutionRequest {
        lab_pipelines::ExecutionRequest {
            pipeline_name: self.pipeline_name,
            input_targets: self.targets,
            no_review: self.no_review,
            no_code: self.no_code,
            output_path: self.output_path.map(std::path::PathBuf::from),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkflowRunRequest {
    pub template: String,
    pub workflow_name: String,
    pub session_id: String,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkflowResponse {
    pub workflow_id: String,
    pub status: String,
    pub steps_completed: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunAgentRequest {
    /// One of: "researcher" | "reviewer" | "coder" | "summarizer"
    pub agent_type: String,
    pub session_id: String,
    pub task: String,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
    pub pattern: Option<String>,
    pub path: Option<String>,
    pub output_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunAgentResponse {
    pub agent_id: String,
    pub status: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AskRequest {
    pub question: String,
    #[serde(default)]
    pub history: Vec<ChatHistoryItem>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatHistoryItem {
    pub role: String,
    pub content: String,
}
