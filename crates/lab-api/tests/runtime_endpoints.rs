mod support;

use lab_core::LabConfig;
use reqwest::StatusCode;
use serde_json::{json, Value};
use support::TestServer;

#[tokio::test]
async fn runtime_endpoints_cover_sessions_memory_tools_agents_and_ask() {
    let workspace = tempfile::tempdir().expect("Failed to create temp workspace");
    std::fs::create_dir_all(workspace.path().join("src")).expect("Failed to create src directory");
    std::fs::write(
        workspace.path().join("src/lib.rs"),
        "pub fn meaning() -> i32 {\n    42\n}\n",
    )
    .expect("Failed to write src/lib.rs");

    let server = TestServer::start(LabConfig::with_workspace(workspace.path().to_path_buf())).await;
    let client = reqwest::Client::new();

    let health: Value = client
        .get(format!("{}/health", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .expect("Failed to create src directory");
    assert_eq!(health["status"], "healthy");

    let tools: Value = client
        .get(format!("{}/tools", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tool_names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(tool_names.contains(&"list_directory"));
    assert!(tool_names.contains(&"file_info"));
    assert!(tool_names.contains(&"git_status"));

    let create_session: Value = client
        .post(format!("{}/sessions", server.base_url))
        .json(&json!({"name": "runtime-check"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = create_session["id"].as_str().unwrap().to_string();

    let fetched_session: Value = client
        .get(format!("{}/sessions/{}", server.base_url, session_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(fetched_session["name"], "runtime-check");

    let tool_exec: Value = client
        .post(format!("{}/tools/execute", server.base_url))
        .json(&json!({
            "tool_name": "list_directory",
            "params": {"path": "src"}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(tool_exec["success"].as_bool().unwrap_or(false));
    assert!(tool_exec["data"]["entries"].as_array().unwrap().iter().any(|entry| {
        entry["name"].as_str() == Some("lib.rs")
    }));

    client
        .post(format!("{}/memory", server.base_url))
        .json(&json!({
            "session_id": session_id,
            "key": "notes",
            "value": {"summary": "architecture mapping complete"},
            "tags": ["notes", "architecture"]
        }))
        .send()
        .await
        .unwrap();

    let keys: Value = client
        .get(format!("{}/memory/{}", server.base_url, session_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(keys["keys"]
        .as_array()
        .unwrap()
        .iter()
        .any(|key| key.as_str() == Some("notes")));

    let search_results: Value = client
        .get(format!(
            "{}/memory/{}/search?q=architecture&limit=5",
            server.base_url, session_id
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(search_results.as_array().unwrap().iter().any(|entry| {
        entry["key"].as_str() == Some("notes")
    }));

    let agent_response: Value = client
        .post(format!("{}/agents/run", server.base_url))
        .json(&json!({
            "agent_type": "researcher",
            "session_id": session_id,
            "task": "inspect rust code",
            "pattern": "**/*.rs"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(agent_response["status"], "completed");
    let agent_id = agent_response["agent_id"].as_str().unwrap().to_string();

    {
        let lab = &server.state.lab;
        let stored = lab
            .memory()
            .get(&session_id, &format!("agent_result_{agent_id}"))
            .unwrap();
        assert!(stored.is_object());
    }

    let ask_response = client
        .post(format!("{}/ask", server.base_url))
        .json(&json!({"question": "What is this project?"}))
        .send()
        .await
        .unwrap();
    assert_eq!(ask_response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let close_response: Value = client
        .delete(format!("{}/sessions/{}", server.base_url, session_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(close_response["status"], "closed");

    server.shutdown().await;
}
