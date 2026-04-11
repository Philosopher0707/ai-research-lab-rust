mod support;

use lab_core::LabConfig;
use reqwest::StatusCode;
use serde_json::{json, Value};
use support::TestServer;

// ─── Helpers ────────────────────────────────────────────────────────

async fn make_server() -> (TestServer, reqwest::Client) {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(workspace.path().join("src")).unwrap();
    std::fs::write(workspace.path().join("src/lib.rs"), "// hello\n").unwrap();
    let server =
        TestServer::start(LabConfig::with_workspace(workspace.path().to_path_buf())).await;
    let client = reqwest::Client::new();
    (server, client)
}

async fn create_session(client: &reqwest::Client, base: &str, name: &str) -> String {
    let resp: Value = client
        .post(format!("{base}/sessions"))
        .json(&json!({"name": name}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    resp["id"].as_str().unwrap().to_string()
}

// ─── Session list ────────────────────────────────────────────────────

#[tokio::test]
async fn list_sessions_returns_all_created() {
    let (server, client) = make_server().await;

    create_session(&client, &server.base_url, "alpha").await;
    create_session(&client, &server.base_url, "beta").await;
    create_session(&client, &server.base_url, "gamma").await;

    let sessions: Value = client
        .get(format!("{}/sessions", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let arr = sessions.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    let names: Vec<&str> = arr.iter().filter_map(|s| s["name"].as_str()).collect();
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));
    assert!(names.contains(&"gamma"));

    server.shutdown().await;
}

// ─── Session 404 errors ──────────────────────────────────────────────

#[tokio::test]
async fn get_nonexistent_session_returns_404() {
    let (server, client) = make_server().await;

    let resp = client
        .get(format!("{}/sessions/does-not-exist", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    server.shutdown().await;
}

#[tokio::test]
async fn close_nonexistent_session_returns_404() {
    let (server, client) = make_server().await;

    let resp = client
        .delete(format!("{}/sessions/does-not-exist", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    server.shutdown().await;
}

// ─── Stats ──────────────────────────────────────────────────────────

#[tokio::test]
async fn stats_reflect_session_count() {
    let (server, client) = make_server().await;

    create_session(&client, &server.base_url, "s1").await;
    create_session(&client, &server.base_url, "s2").await;

    let stats: Value = client
        .get(format!("{}/stats", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(stats["sessions"]["total"], 2);
    assert_eq!(stats["sessions"]["active"], 2);
    assert!(stats["running"].as_bool().unwrap_or(false));
    assert!(stats["memory_entries"].as_u64().is_some());

    server.shutdown().await;
}

#[tokio::test]
async fn stats_count_tool_calls() {
    let (server, client) = make_server().await;

    // Execute two tools
    for _ in 0..2 {
        client
            .post(format!("{}/tools/execute", server.base_url))
            .json(&json!({"tool_name": "list_directory", "params": {"path": "src"}}))
            .send()
            .await
            .unwrap();
    }

    let stats: Value = client
        .get(format!("{}/stats", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(stats["operations"]["tool_calls"].as_u64().unwrap_or(0) >= 2);

    server.shutdown().await;
}

// ─── Event history ───────────────────────────────────────────────────

#[tokio::test]
async fn event_history_records_session_events() {
    let (server, client) = make_server().await;

    let session_id = create_session(&client, &server.base_url, "evt-test").await;

    client
        .delete(format!("{}/sessions/{}", server.base_url, session_id))
        .send()
        .await
        .unwrap();

    let history: Value = client
        .get(format!("{}/events/history", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let events = history.as_array().unwrap();
    assert!(!events.is_empty());

    let types: Vec<&str> = events.iter().filter_map(|e| e["type"].as_str()).collect();
    assert!(types.contains(&"session.created"), "expected session.created in {:?}", types);
    assert!(types.contains(&"session.closed"),  "expected session.closed in {:?}", types);

    server.shutdown().await;
}

#[tokio::test]
async fn event_history_filter_by_type() {
    let (server, client) = make_server().await;

    create_session(&client, &server.base_url, "filter-test").await;

    let history: Value = client
        .get(format!("{}/events/history?filter=session.created", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let events = history.as_array().unwrap();
    assert!(!events.is_empty());
    for event in events {
        assert_eq!(event["type"], "session.created");
    }

    server.shutdown().await;
}

// ─── Tool execution errors ───────────────────────────────────────────

#[tokio::test]
async fn execute_unknown_tool_returns_failure() {
    let (server, client) = make_server().await;

    let resp: Value = client
        .post(format!("{}/tools/execute", server.base_url))
        .json(&json!({"tool_name": "no_such_tool", "params": {}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(!resp["success"].as_bool().unwrap_or(true));

    server.shutdown().await;
}

// ─── Memory tag filtering ────────────────────────────────────────────

#[tokio::test]
async fn memory_tag_filter_returns_only_matching_keys() {
    let (server, client) = make_server().await;

    let session_id = create_session(&client, &server.base_url, "tag-test").await;

    // Store two entries with different tags
    for (key, tags) in [("report", vec!["report"]), ("notes", vec!["scratch"])] {
        client
            .post(format!("{}/memory", server.base_url))
            .json(&json!({
                "session_id": session_id,
                "key": key,
                "value": {"x": 1},
                "tags": tags,
            }))
            .send()
            .await
            .unwrap();
    }

    let keys: Value = client
        .get(format!("{}/memory/{}?tags=report", server.base_url, session_id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let key_list = keys["keys"].as_array().unwrap();
    assert_eq!(key_list.len(), 1);
    assert_eq!(key_list[0], "report");

    server.shutdown().await;
}

#[tokio::test]
async fn memory_store_and_retrieve_value_roundtrip() {
    let (server, client) = make_server().await;

    let session_id = create_session(&client, &server.base_url, "mem-roundtrip").await;

    let entry: Value = client
        .post(format!("{}/memory", server.base_url))
        .json(&json!({
            "session_id": session_id,
            "key": "checkpoint",
            "value": {"step": 7, "status": "ok"},
            "tags": [],
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(entry["key"], "checkpoint");
    assert_eq!(entry["value"]["step"], 7);

    server.shutdown().await;
}

// ─── Workflow execution ──────────────────────────────────────────────

#[tokio::test]
async fn workflow_run_with_builtin_template_completes() {
    let (server, client) = make_server().await;

    let session_id = create_session(&client, &server.base_url, "wf-session").await;

    let resp: Value = client
        .post(format!("{}/workflows/run", server.base_url))
        .json(&json!({
            "template": "code-review",
            "workflow_name": "test-review",
            "session_id": session_id,
            "params": {"topic": "test"},
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(resp["status"], "completed", "workflow response: {resp}");
    assert!(resp["workflow_id"].as_str().is_some());
    assert!(resp["steps_completed"].as_u64().unwrap_or(0) > 0);

    server.shutdown().await;
}

#[tokio::test]
async fn workflow_run_with_unknown_template_returns_error() {
    let (server, client) = make_server().await;

    let session_id = create_session(&client, &server.base_url, "wf-err").await;

    let resp = client
        .post(format!("{}/workflows/run", server.base_url))
        .json(&json!({
            "template": "no-such-template",
            "workflow_name": "nope",
            "session_id": session_id,
            "params": {},
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    server.shutdown().await;
}

// ─── Concurrent session creation ─────────────────────────────────────

#[tokio::test]
async fn concurrent_session_creation_all_succeed() {
    let (server, client) = make_server().await;

    let handles: Vec<_> = (0..10)
        .map(|i| {
            let client = client.clone();
            let base = server.base_url.clone();
            tokio::spawn(async move {
                let resp: Value = client
                    .post(format!("{base}/sessions"))
                    .json(&json!({"name": format!("concurrent-{i}")}))
                    .send()
                    .await
                    .unwrap()
                    .json()
                    .await
                    .unwrap();
                resp["id"].as_str().unwrap().to_string()
            })
        })
        .collect();

    let mut ids = Vec::new();
    for h in handles {
        ids.push(h.await.unwrap());
    }

    // All IDs must be unique
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 10);

    // List endpoint must see all 10
    let sessions: Value = client
        .get(format!("{}/sessions", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(sessions.as_array().unwrap().len(), 10);

    server.shutdown().await;
}
