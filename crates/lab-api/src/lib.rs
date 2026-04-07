//! Lab REST API + WebSocket event streaming server.
//!
//! Provides a FastAPI-equivalent server for the AI Research Lab with:
//! - REST endpoints for sessions, tools, memory, agents, pipelines, workflows
//! - WebSocket event streaming for real-time lab events
//! - Built on axum with tower middleware

use axum::{
    extract::{Path, Query, State, WebSocketUpgrade},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use axum::extract::ws::{Message, WebSocket};
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use lab_core::{
    EventBus, LabConfig, LabEvent, ResearchLab,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};

// ─── Application State ──────────────────────────────────────

/// Shared application state accessible by all route handlers.
pub struct AppState {
    lab: RwLock<ResearchLab>,
    ws_clients: Mutex<Vec<tokio::sync::mpsc::Sender<String>>>,
}

impl AppState {
    pub async fn new(config: LabConfig) -> Self {
        let mut lab = ResearchLab::new(config);
        lab.start().await.expect("Failed to start lab");
        
        Self {
            lab: RwLock::new(lab),
            ws_clients: Mutex::new(Vec::new()),
        }
    }
}

// ─── Request/Response DTOs ──────────────────────────────────

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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PipelineResponse {
    pub pipeline: String,
    pub session_id: String,
    pub status: String,
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
    pub agent_type: String,
    pub session_id: String,
    pub task: String,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
    pub pattern: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunAgentResponse {
    pub agent_id: String,
    pub status: String,
    pub data: serde_json::Value,
}

// ─── Route Handlers ─────────────────────────────────────────

/// GET /health — Health check endpoint.
pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "timestamp": Utc::now().to_rfc3339(),
    }))
}

/// POST /sessions — Create a new lab session.
pub async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateSessionRequest>,
) -> impl IntoResponse {
    let mut lab = state.lab.write().await;
    match lab.create_session(&req.name).await {
        Ok(session) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": session.id,
                "name": session.name,
                "status": format!("{:?}", session.status).to_lowercase(),
                "agents_active": session.agents_active,
                "tasks_completed": session.tasks_completed,
            })),
        ),
        Err(e) => {
            error!("Failed to create session: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        }
    }
}

/// GET /sessions — List all sessions.
pub async fn list_sessions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let lab = state.lab.read().await;
    let sessions: Vec<_> = lab.list_sessions().iter().map(|s| SessionResponse {
        id: s.id.clone(),
        name: s.name.clone(),
        status: format!("{:?}", s.status).to_lowercase(),
        agents_active: s.agents_active,
        tasks_completed: s.tasks_completed,
    }).collect();
    Json(sessions)
}

/// GET /sessions/:id — Get session details.
pub async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    let lab = state.lab.read().await;
    match lab.get_session(&session_id) {
        Some(s) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": s.id,
                "name": s.name,
                "status": format!("{:?}", s.status).to_lowercase(),
                "agents_active": s.agents_active,
                "tasks_completed": s.tasks_completed,
            })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Session not found"})),
        ),
    }
}

/// DELETE /sessions/:id — Close a session.
pub async fn close_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    let mut lab = state.lab.write().await;
    match lab.close_session(&session_id).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "closed"})),
        ),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

/// GET /tools — List all registered tools.
pub async fn list_tools(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let lab = state.lab.read().await;
    Json(lab.tools().list_tools())
}

/// POST /tools/execute — Execute a tool.
pub async fn execute_tool(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ToolRequest>,
) -> impl IntoResponse {
    let mut lab = state.lab.write().await;
    let result = lab.execute_tool(&req.tool_name, req.params, None).await;
    Json(result)
}

/// GET /memory/:session_id — List memory keys for a session.
pub async fn list_memory(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let tags: Option<Vec<String>> = params.get("tags")
        .map(|t| t.split(',').map(|s| s.to_string()).collect());
    let lab = state.lab.read().await;
    let keys = lab.memory().list_keys(&session_id, tags.as_deref());
    Json(serde_json::json!({"keys": keys}))
}

/// POST /memory — Store data in memory.
pub async fn store_memory(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MemoryRequest>,
) -> impl IntoResponse {
    let mut lab = state.lab.write().await;
    let entry = lab.memory_mut().store(
        &req.session_id,
        &req.key,
        &req.value,
        Some(req.tags.clone()),
    );
    Json(MemoryResponse {
        key: entry.key,
        value: entry.value,
        tags: entry.tags,
    })
}

/// GET /memory/:session_id/search — Search memory.
pub async fn search_memory(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let query = params.get("q").cloned().unwrap_or_default();
    let limit = params.get("limit")
        .and_then(|l| l.parse().ok())
        .unwrap_or(10);
    let lab = state.lab.read().await;
    let results = lab.memory().search(&session_id, &query, None, limit);
    Json(results)
}

/// GET /stats — Get lab statistics.
pub async fn get_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let lab = state.lab.read().await;
    Json(lab.get_stats())
}

/// POST /pipelines/run — Run a pipeline.
pub async fn run_pipeline(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PipelineRunRequest>,
) -> impl IntoResponse {
    let mut lab = state.lab.write().await;
    match lab.run_pipeline(&req.pipeline_name, req.targets).await {
        Ok(result) => (
            StatusCode::OK,
            Json(result),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

/// POST /workflows/run — Run a workflow from template.
pub async fn run_workflow(
    State(state): State<Arc<AppState>>,
    Json(req): Json<WorkflowRunRequest>,
) -> impl IntoResponse {
    let mut lab = state.lab.write().await;
    let executor = |_step: &lab_core::workflows::WorkflowStep, _ctx: &std::collections::HashMap<String, serde_json::Value>| {
        let fut = async { Ok::<_, lab_core::LabError>(serde_json::json!({"executed": true})) };
        Box::pin(fut) as std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value, lab_core::LabError>> + Send>>
    };
    match lab.run_workflow_from_template(
        &req.template,
        &req.workflow_name,
        Some(&req.session_id),
        req.params,
        executor,
    ).await {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "workflow_id": result.execution_id,
                "status": result.status,
                "steps_completed": result.step_results.iter()
                    .filter(|s| matches!(s.status, lab_core::workflows::StepStatus::Completed))
                    .count(),
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        ),
    }
}

/// WS /events — WebSocket endpoint for real-time event streaming.
pub async fn ws_events(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: Arc<AppState>) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(100);
    
    // Register client
    {
        let mut clients = state.ws_clients.lock().await;
        clients.push(tx.clone());
    }
    info!("WebSocket client connected. Total clients: {}", {
        state.ws_clients.lock().await.len()
    });
    
    let state_clone = state.clone();
    
    // Spawn task to forward events to client
    let send_task = tokio::spawn(async move {
        let mut ws = socket;
        while let Some(msg) = rx.recv().await {
            if ws.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });
    
    // Wait for send task to complete (channel closed or client disconnected)
    send_task.await.ok();
    
    // Clean up client
    {
        let mut clients = state_clone.ws_clients.lock().await;
        clients.retain(|c| !c.is_closed());
    }
    info!("WebSocket client disconnected");
}

/// Broadcast a message to all WebSocket clients.
pub async fn broadcast_event(clients: &Mutex<Vec<tokio::sync::mpsc::Sender<String>>>, data: &str) {
    let data = data.to_string();
    let mut clients = clients.lock().await;
    clients.retain(|tx| tx.try_send(data.clone()).is_ok());
}

// ─── Router Setup ───────────────────────────────────────────

/// Create the axum router with all API routes.
pub fn create_router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::permissive();
    
    Router::new()
        .route("/health", get(health))
        .route("/sessions", post(create_session))
        .route("/sessions", get(list_sessions))
        .route("/sessions/:id", get(get_session))
        .route("/sessions/:id", delete(close_session))
        .route("/tools", get(list_tools))
        .route("/tools/execute", post(execute_tool))
        .route("/memory/:session_id", get(list_memory))
        .route("/memory", post(store_memory))
        .route("/memory/:session_id/search", get(search_memory))
        .route("/stats", get(get_stats))
        .route("/pipelines/run", post(run_pipeline))
        .route("/workflows/run", post(run_workflow))
        .route("/events", get(ws_events))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Start the API server.
pub async fn start_server(state: Arc<AppState>, port: u16) -> anyhow::Result<()> {
    let app = create_router(state);
    let addr = format!("0.0.0.0:{port}");
    info!("Starting API server on {addr}");
    
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_response_format() {
        // Just a sanity check that JSON serializes correctly
        let json = serde_json::json!({
            "status": "healthy",
            "timestamp": "2024-01-01T00:00:00Z",
        });
        assert_eq!(json.get("status").and_then(|v| v.as_str()).unwrap(), "healthy");
    }

    #[test]
    fn request_deserialization() {
        let req: CreateSessionRequest = serde_json::from_value(
            serde_json::json!({"name": "test-session"})
        ).unwrap();
        assert_eq!(req.name, "test-session");
        
        let tool_req: ToolRequest = serde_json::from_value(
            serde_json::json!({
                "tool_name": "read_file",
                "params": {"path": "test.py"}
            })
        ).unwrap();
        assert_eq!(tool_req.tool_name, "read_file");
        assert!(!tool_req.params.is_empty());
    }
}
