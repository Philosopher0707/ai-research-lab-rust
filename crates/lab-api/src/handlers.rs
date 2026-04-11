use crate::models::{
    AskRequest, CreateSessionRequest, MemoryRequest, MemoryResponse, PipelineRunRequest,
    RunAgentRequest, RunAgentResponse, SessionResponse, ToolRequest, WorkflowRunRequest,
};
use crate::state::AppState;
use axum::extract::ws::{Message, WebSocket};
use axum::{
    extract::{Path, Query, State, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use lab_core::config::AgentProfile;
use lab_core::llm::ChatMessage;
use lab_core::types::LabSession;
use lab_core::LabConfig;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

type WorkflowStepFuture = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<serde_json::Value, lab_core::LabError>> + Send>,
>;

struct PipelineExecutionContext {
    config: LabConfig,
    pipeline_name: String,
    session_id: String,
}

impl PipelineExecutionContext {
    async fn begin(state: &Arc<AppState>, pipeline_name: &str) -> Result<Self, Response> {
        let lab = &state.lab;
        match lab.begin_pipeline_run(pipeline_name).await {
            Ok(session_id) => Ok(Self {
                config: lab.config.clone(),
                pipeline_name: pipeline_name.to_string(),
                session_id,
            }),
            Err(error) => Err(internal_error_response(error)),
        }
    }

    async fn finish(
        self,
        state: &Arc<AppState>,
        result: lab_pipelines::PipelineResult,
    ) -> Response {
        let result_value =
            serde_json::to_value(&result).unwrap_or_else(|_| pipeline_result_summary(&result));
        let lab = &state.lab;

        match lab
            .finish_pipeline_run(&self.session_id, &self.pipeline_name, &result_value)
            .await
        {
            Ok(()) => (StatusCode::OK, Json(result)).into_response(),
            Err(error) => internal_error_response(error),
        }
    }
}

fn session_response(session: &LabSession) -> SessionResponse {
    SessionResponse {
        id: session.id.clone(),
        name: session.name.clone(),
        status: session.status.as_str().to_string(),
        agents_active: session.agents_active,
        tasks_completed: session.tasks_completed,
    }
}

fn json_error(status: StatusCode, error: impl ToString) -> Response {
    (
        status,
        Json(serde_json::json!({"error": error.to_string()})),
    )
        .into_response()
}

fn internal_error_response(error: impl ToString) -> Response {
    json_error(StatusCode::INTERNAL_SERVER_ERROR, error)
}

fn pipeline_result_summary(result: &lab_pipelines::PipelineResult) -> serde_json::Value {
    result.summary()
}

fn ask_messages(req: &AskRequest) -> Vec<ChatMessage> {
    let mut messages: Vec<ChatMessage> = req
        .history
        .iter()
        .map(|item| ChatMessage {
            role: item.role.clone(),
            content: item.content.clone(),
        })
        .collect();
    messages.push(ChatMessage::user(&req.question));
    messages
}

fn workflow_executor() -> impl Fn(
    &lab_core::workflows::WorkflowStep,
    &HashMap<String, serde_json::Value>,
) -> WorkflowStepFuture
       + Send
       + Sync {
    |_step, _ctx| {
        let future = async { Ok::<_, lab_core::LabError>(serde_json::json!({"executed": true})) };
        Box::pin(future)
    }
}

pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "timestamp": Utc::now().to_rfc3339(),
    }))
}

pub async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateSessionRequest>,
) -> Response {
    let lab = &state.lab;
    match lab.create_session(&req.name).await {
        Ok(session) => (StatusCode::CREATED, Json(session_response(&session))).into_response(),
        Err(error) => internal_error_response(error),
    }
}

pub async fn list_sessions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let lab = &state.lab;
    let sessions: Vec<_> = lab
        .list_sessions()
        .await
        .iter()
        .map(session_response)
        .collect();
    Json(sessions)
}

pub async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    let lab = &state.lab;
    match lab.get_session(&session_id).await {
        Some(session) => (StatusCode::OK, Json(session_response(&session))).into_response(),
        None => json_error(StatusCode::NOT_FOUND, "Session not found"),
    }
}

pub async fn close_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    let lab = &state.lab;
    match lab.close_session(&session_id).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "closed"})),
        )
            .into_response(),
        Err(error) => json_error(StatusCode::NOT_FOUND, error),
    }
}

pub async fn list_tools(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let lab = &state.lab;
    Json(lab.list_tools().await)
}

pub async fn execute_tool(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ToolRequest>,
) -> impl IntoResponse {
    let lab = &state.lab;
    Json(lab.execute_tool(&req.tool_name, req.params, None).await)
}

pub async fn list_memory(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let tags = params.get("tags").map(|tag_list| {
        tag_list
            .split(',')
            .map(|tag| tag.to_string())
            .collect::<Vec<_>>()
    });
    let lab = &state.lab;
    let keys = lab.list_memory_keys(&session_id, tags.as_deref());
    Json(serde_json::json!({"keys": keys}))
}

pub async fn store_memory(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MemoryRequest>,
) -> impl IntoResponse {
    let lab = &state.lab;
    let entry = lab.store_memory(
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

pub async fn search_memory(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let query = params.get("q").cloned().unwrap_or_default();
    let limit = params
        .get("limit")
        .and_then(|value| value.parse().ok())
        .unwrap_or(10);
    let lab = &state.lab;
    Json(lab.search_memory(&session_id, &query, None, limit))
}

pub async fn get_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let lab = &state.lab;
    Json(lab.get_stats().await)
}

pub async fn event_history(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let filter = params.get("filter").map(|value| value.as_str());
    let limit = params
        .get("limit")
        .and_then(|value| value.parse().ok())
        .unwrap_or(50);
    // Access event history directly from the shared Arc — no lab lock needed.
    Json(state.events.get_history_json(filter, limit))
}

pub async fn run_pipeline(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PipelineRunRequest>,
) -> Response {
    let execution = match PipelineExecutionContext::begin(&state, &req.pipeline_name).await {
        Ok(execution) => execution,
        Err(response) => return response,
    };

    let result = lab_pipelines::run_pipeline_with_config(
        &execution.config,
        req.into_execution_request(),
        execution.session_id.clone(),
    )
    .await;

    execution.finish(&state, result).await
}

pub async fn run_workflow(
    State(state): State<Arc<AppState>>,
    Json(req): Json<WorkflowRunRequest>,
) -> Response {
    let lab = &state.lab;
    match lab
        .run_workflow_from_template(
            &req.template,
            &req.workflow_name,
            Some(&req.session_id),
            req.params,
            workflow_executor(),
        )
        .await
    {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "workflow_id": result.execution_id,
                "status": result.status,
                "steps_completed": result
                    .step_results
                    .iter()
                    .filter(|step| matches!(step.status, lab_core::workflows::StepStatus::Completed))
                    .count(),
            })),
        )
            .into_response(),
        Err(error) => internal_error_response(error),
    }
}

pub async fn run_agent(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RunAgentRequest>,
) -> Response {
    let config = {
        let lab = &state.lab;
        lab.config.clone()
    };
    let agent_id = format!(
        "{}-{}",
        req.agent_type,
        &uuid::Uuid::new_v4().to_string()[..6]
    );
    let profile = AgentProfile::default();
    let content = req
        .params
        .get("content")
        .and_then(|value| value.as_str())
        .map(String::from);
    let template = req
        .params
        .get("template")
        .and_then(|value| value.as_str())
        .map(String::from);
    let request = lab_agents::AgentExecutionRequest {
        agent_type: req.agent_type.clone(),
        agent_id: agent_id.clone(),
        session_id: req.session_id.clone(),
        task: req.task.clone(),
        pattern: req.pattern.clone(),
        path: req.path.clone(),
        output_path: req.output_path.clone(),
        content,
        template,
        profile,
        dry_run: false,
    };
    let result = match lab_agents::execute_agent(&config, request).await {
        Ok(result) => result,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };

    {
        let lab = &state.lab;
        let key = format!("agent_result_{agent_id}");
        lab.store_memory(
            &req.session_id,
            &key,
            &result.data,
            Some(vec!["agent_result".into()]),
        );
    }

    (
        StatusCode::OK,
        Json(RunAgentResponse {
            agent_id,
            status: if result.success {
                "completed"
            } else {
                "failed"
            }
            .to_string(),
            data: result.data,
        }),
    )
        .into_response()
}

pub async fn ask_llm(State(state): State<Arc<AppState>>, Json(req): Json<AskRequest>) -> Response {
    let lab = &state.lab;
    if !lab.has_llm() {
        let hint = if lab.config.provider == "local" {
            "Set LAB_PROVIDER=local and LAB_BASE_URL in .env, or run `lab setup`.".to_string()
        } else if let Some(env_key) = lab.config.expected_api_key_env() {
            format!("Set {env_key} in .env, or run `lab setup`.")
        } else {
            "Configure an LLM provider and API key.".to_string()
        };
        return json_error(StatusCode::SERVICE_UNAVAILABLE, hint);
    }

    match lab.ask_llm_messages(ask_messages(&req), 0.7, 1024).await {
        Some(answer) => {
            (StatusCode::OK, Json(serde_json::json!({"answer": answer}))).into_response()
        }
        None => internal_error_response("LLM returned no response"),
    }
}

pub async fn ws_events(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: Arc<AppState>) {
    // Subscribe to the shared event bus directly — no bridge channel needed.
    let mut rx = state.events.subscribe();
    let (mut ws_tx, mut ws_rx) = socket.split();

    let _ = ws_tx
        .send(Message::Text(
            serde_json::json!({
                "type": "connected",
                "timestamp": Utc::now().to_rfc3339(),
            })
            .to_string()
            .into(),
        ))
        .await;

    info!("WebSocket client connected");

    loop {
        tokio::select! {
            message = rx.recv() => {
                match message {
                    Ok(event) => {
                        let data = serde_json::json!({
                            "type": event.event_type,
                            "data": event.data,
                            "source": event.source,
                        })
                        .to_string();
                        if ws_tx.send(Message::Text(data.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        warn!("WebSocket client lagged {} messages", count);
                        continue;
                    }
                    Err(_) => break,
                }
            }
            client_message = ws_rx.next() => {
                match client_message {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Text(text))) if text.as_str() == "ping" => {
                        let _ = ws_tx.send(Message::Text("pong".into())).await;
                    }
                    _ => {}
                }
            }
        }
    }

    info!("WebSocket client disconnected");
}
