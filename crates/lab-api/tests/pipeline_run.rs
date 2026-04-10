use lab_api::{create_router, AppState};
use lab_core::LabConfig;
use serde_json::{json, Value};
use std::sync::Arc;

async fn spawn_test_server(
    state: Arc<AppState>,
) -> (
    String,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let router = create_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });

    (format!("http://{addr}"), shutdown_tx, server)
}

#[tokio::test]
async fn pipeline_run_endpoint_executes_and_persists_results() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(workspace.path().join("src")).unwrap();
    std::fs::write(
        workspace.path().join("src/lib.rs"),
        "pub fn answer() -> i32 {\n    42\n}\n",
    )
    .unwrap();
    std::fs::write(
        workspace.path().join("Cargo.toml"),
        "[package]\nname = \"pipeline-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();

    let config = LabConfig::with_workspace(workspace.path().to_path_buf());
    let state = Arc::new(AppState::new(config).await);
    let (base_url, shutdown_tx, server) = spawn_test_server(state.clone()).await;

    let response = reqwest::Client::new()
        .post(format!("{base_url}/pipelines/run"))
        .json(&json!({
            "pipeline_name": "review",
            "targets": ["**/*.rs"],
            "no_review": true,
            "no_code": true,
            "output_path": "lab-outputs/test-report.md"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["pipeline_name"], "review");
    assert_eq!(body["status"], "completed");
    assert_eq!(body["output_path"], "lab-outputs/test-report.md");

    let stages = body["stage_results"].as_array().unwrap();
    assert!(stages
        .iter()
        .any(|stage| { stage["name"] == "discover" && stage["status"] == "completed" }));
    assert!(stages
        .iter()
        .any(|stage| { stage["name"] == "report" && stage["status"] == "completed" }));
    assert!(workspace.path().join("lab-outputs/test-report.md").exists());

    let lab = state.lab.read().await;
    let session = lab
        .list_sessions()
        .into_iter()
        .find(|session| session.name == "pipeline-review")
        .unwrap();
    assert_eq!(format!("{:?}", session.status).to_lowercase(), "completed");
    assert!(session.tasks_completed >= 5);

    let stored = lab.memory().get(&session.id, "pipeline_result").unwrap();
    assert_eq!(stored["status"], "completed");
    assert_eq!(stored["output_path"], "lab-outputs/test-report.md");

    let _ = shutdown_tx.send(());
    let _ = server.await;
}
