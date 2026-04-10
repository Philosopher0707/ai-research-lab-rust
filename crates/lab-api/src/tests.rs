use crate::{AskRequest, CreateSessionRequest, PipelineRunRequest, RunAgentRequest, ToolRequest};

#[test]
fn health_response_format() {
    let json = serde_json::json!({"status": "healthy", "timestamp": "2024-01-01T00:00:00Z"});
    assert_eq!(
        json.get("status").and_then(|value| value.as_str()).unwrap(),
        "healthy"
    );
}

#[test]
fn request_deserialization() {
    let session_req: CreateSessionRequest =
        serde_json::from_value(serde_json::json!({"name": "test-session"})).unwrap();
    assert_eq!(session_req.name, "test-session");

    let tool_req: ToolRequest = serde_json::from_value(serde_json::json!({
        "tool_name": "read_file",
        "params": {"path": "test.py"}
    }))
    .unwrap();
    assert_eq!(tool_req.tool_name, "read_file");

    let agent_req: RunAgentRequest = serde_json::from_value(serde_json::json!({
        "agent_type": "researcher",
        "session_id": "s1",
        "task": "analyze codebase"
    }))
    .unwrap();
    assert_eq!(agent_req.agent_type, "researcher");
    assert!(agent_req.pattern.is_none());

    let pipeline_req: PipelineRunRequest = serde_json::from_value(serde_json::json!({
        "pipeline_name": "review",
        "targets": ["**/*.rs"],
        "no_review": true,
        "output_path": "lab-outputs/custom-report.md"
    }))
    .unwrap();
    assert_eq!(pipeline_req.pipeline_name, "review");
    assert!(pipeline_req.no_review);
    assert_eq!(
        pipeline_req.output_path.as_deref(),
        Some("lab-outputs/custom-report.md")
    );

    let ask_req: AskRequest = serde_json::from_value(serde_json::json!({
        "question": "What does this do?",
        "history": [{"role": "system", "content": "helpful"}]
    }))
    .unwrap();
    assert_eq!(ask_req.question, "What does this do?");
    assert_eq!(ask_req.history.len(), 1);
}
