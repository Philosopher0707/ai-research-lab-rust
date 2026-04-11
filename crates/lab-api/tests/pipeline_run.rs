use lab_core::LabConfig;
use serde_json::{json, Value};

mod support;

use support::TestServer;

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
    let server = TestServer::start(config).await;

    let response = reqwest::Client::new()
        .post(format!("{}/pipelines/run", server.base_url))
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

    let lab = server.state.lab.read().await;
    let session = lab
        .list_sessions()
        .into_iter()
        .find(|session| session.name == "pipeline-review")
        .unwrap();
    assert_eq!(session.status.as_str(), "completed");
    assert!(session.tasks_completed >= 5);

    let stored = lab.memory().get(&session.id, "pipeline_result").unwrap();
    assert_eq!(stored["status"], "completed");
    assert_eq!(stored["output_path"], "lab-outputs/test-report.md");

    drop(lab);
    server.shutdown().await;
}
