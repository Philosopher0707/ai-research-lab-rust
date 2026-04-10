use crate::handlers::{
    ask_llm, close_session, create_session, event_history, execute_tool, get_session, get_stats,
    health, list_memory, list_sessions, list_tools, run_agent, run_pipeline, run_workflow,
    search_memory, store_memory, ws_events,
};
use crate::state::AppState;
use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/sessions/{id}", get(get_session).delete(close_session))
        .route("/tools", get(list_tools))
        .route("/tools/execute", post(execute_tool))
        .route("/memory/{session_id}", get(list_memory))
        .route("/memory", post(store_memory))
        .route("/memory/{session_id}/search", get(search_memory))
        .route("/agents/run", post(run_agent))
        .route("/ask", post(ask_llm))
        .route("/stats", get(get_stats))
        .route("/events", get(ws_events))
        .route("/events/history", get(event_history))
        .route("/pipelines/run", post(run_pipeline))
        .route("/workflows/run", post(run_workflow))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

pub async fn start_server(state: Arc<AppState>, port: u16) -> anyhow::Result<()> {
    let app = create_router(state);
    let addr = format!("0.0.0.0:{port}");
    info!("Starting API server on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
