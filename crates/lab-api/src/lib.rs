//! Lab REST API + WebSocket event streaming server.
//!
//! Provides a FastAPI-equivalent server for the AI Research Lab with:
//! - REST endpoints for sessions, tools, memory, agents, pipelines, workflows
//! - WebSocket event streaming for real-time lab events (wired to EventBus)
//! - Built on axum with tower middleware

mod handlers;
mod models;
mod router;
mod state;

#[cfg(test)]
mod tests;

pub use handlers::{
    ask_llm, close_session, create_session, event_history, execute_tool, get_session, get_stats,
    health, list_memory, list_sessions, list_tools, run_agent, run_pipeline, run_workflow,
    search_memory, store_memory, ws_events,
};
pub use models::{
    AskRequest, ChatHistoryItem, CreateSessionRequest, MemoryRequest, MemoryResponse,
    PipelineRunRequest, RunAgentRequest, RunAgentResponse, SessionResponse, ToolRequest,
    ToolResponse, WorkflowResponse, WorkflowRunRequest,
};
pub use router::{create_router, start_server};
pub use state::AppState;
