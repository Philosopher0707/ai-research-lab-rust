//! Lab Tools — Tool registry and built-in tools.
//! Mirrors tools/registry.py (359 lines)

use async_trait::async_trait;
use globset::{Glob, GlobSetBuilder};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use tracing::warn;

// ─── Tool Result ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ToolResult {
    pub success: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
    pub duration_ms: f64,
}

impl ToolResult {
    pub fn ok(data: serde_json::Value, start: Instant) -> Self {
        Self { success: true, data: Some(data), error: None, duration_ms: start.elapsed().as_secs_f64() * 1000.0 }
    }
    pub fn err(message: impl Into<String>, start: Instant) -> Self {
        Self { success: false, data: None, error: Some(message.into()), duration_ms: start.elapsed().as_secs_f64() * 1000.0 }
    }
}

// ─── Tool Trait (dyn compatible via async_trait) ──────────────

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn category(&self) -> &str;
    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult;
}

// ─── Read Tool ────────────────────────────────────────────────

pub struct ReadTool { workspace: PathBuf }

impl ReadTool {
    pub fn new(workspace: PathBuf) -> Self { Self { workspace } }
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str { "read_file" }
    fn description(&self) -> &str { "Read the contents of a file" }
    fn category(&self) -> &str { "filesystem" }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::err("Missing 'path' parameter", start),
        };
        if path.contains("..") {
            return ToolResult::err("Path traversal not allowed", start);
        }
        let file_path = self.workspace.join(path);
        let workspace_root = self.workspace.canonicalize().unwrap_or_else(|_| self.workspace.clone());
        let Ok(resolved) = file_path.canonicalize() else {
            return ToolResult::err(format!("File not found: {path}"), start);
        };
        if !resolved.starts_with(&workspace_root) {
            return ToolResult::err("Access denied: path escapes workspace", start);
        }
        match tokio::fs::read_to_string(&resolved).await {
            Ok(content) => {
                let all_lines: Vec<&str> = content.lines().collect();
                let total_lines = all_lines.len();
                let limit = params.get("limit").and_then(|v| v.as_u64()).map(|l| l as usize);
                let offset = params.get("offset").and_then(|v| v.as_u64()).map(|o| o as usize).unwrap_or(0);
                let lines: Vec<String> = if let Some(lim) = limit {
                    all_lines.iter().skip(offset).take(lim).map(|s| s.to_string()).collect()
                } else {
                    all_lines.iter().skip(offset).map(|s| s.to_string()).collect()
                };
                ToolResult::ok(serde_json::json!({"content": lines.join("\n"), "total_lines": total_lines}), start)
            }
            Err(e) => ToolResult::err(e.to_string(), start),
        }
    }
}

// ─── Write Tool ───────────────────────────────────────────────

pub struct WriteTool { workspace: PathBuf }

impl WriteTool {
    pub fn new(workspace: PathBuf) -> Self { Self { workspace } }
}

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str { "write_file" }
    fn description(&self) -> &str { "Write content to a file" }
    fn category(&self) -> &str { "filesystem" }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::err("Missing 'path' parameter", start),
        };
        let content = match params.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::err("Missing 'content' parameter", start),
        };
        if path.contains("..") {
            return ToolResult::err("Path traversal not allowed", start);
        }
        let file_path = self.workspace.join(path);
        // Create parent directories if needed
        if let Some(parent) = file_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ToolResult::err(e.to_string(), start);
            }
        }
        match tokio::fs::write(&file_path, content).await {
            Ok(()) => ToolResult::ok(
                serde_json::json!({"path": path, "bytes_written": content.len()}), start),
            Err(e) => ToolResult::err(e.to_string(), start),
        }
    }
}

// ─── Bash Tool ────────────────────────────────────────────────

pub struct BashTool { workspace: PathBuf }

impl BashTool {
    pub fn new(workspace: PathBuf) -> Self { Self { workspace } }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str { "bash" }
    fn description(&self) -> &str { "Execute a shell command" }
    fn category(&self) -> &str { "system" }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let command = match params.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::err("Missing 'command' parameter", start),
        };
        let timeout_secs = params.get("timeout").and_then(|v| v.as_u64()).unwrap_or(60);
        match tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            tokio::process::Command::new("bash")
                .arg("-c")
                .arg(command)
                .current_dir(&self.workspace)
                .output(),
        ).await {
            Ok(Ok(output)) => ToolResult::ok(serde_json::json!({
                "stdout": String::from_utf8_lossy(&output.stdout),
                "stderr": String::from_utf8_lossy(&output.stderr),
                "returncode": output.status.code().unwrap_or(-1),
            }), start),
            Ok(Err(e)) => ToolResult::err(e.to_string(), start),
            Err(_) => ToolResult::err(format!("Command timed out after {timeout_secs}s"), start),
        }
    }
}

// ─── Glob Tool ───────────────────────────────────────────────

pub struct GlobTool { workspace: PathBuf }

impl GlobTool {
    pub fn new(workspace: PathBuf) -> Self { Self { workspace } }
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str { "glob_search" }
    fn description(&self) -> &str { "Find files matching a glob pattern" }
    fn category(&self) -> &str { "filesystem" }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let pattern = match params.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::err("Missing 'pattern' parameter", start),
        };
        let search_path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => PathBuf::from(p),
            None => self.workspace.clone(),
        };
        let mut builder = GlobSetBuilder::new();
        if let Ok(glob) = Glob::new(pattern) {
            builder.add(glob);
        }
        let Ok(globset) = builder.build() else {
            return ToolResult::err("Invalid glob pattern", start);
        };
        let max_files = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(500) as usize;
        let mut matches = Vec::new();
        if search_path.exists() && search_path.is_dir() {
            for entry in walkdir::WalkDir::new(&search_path)
                .max_depth(12)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if matches.len() >= max_files {
                    break;
                }
                // Skip common noise directories
                if let Some(name) = entry.file_name().to_str() {
                    if matches!(name, ".git" | ".hg" | ".svn" | "node_modules" | ".cargo" | ".rustup" | "__pycache__" | ".nvm") {
                        continue;
                    }
                    if name.starts_with('.') && entry.file_type().is_dir() {
                        continue;
                    }
                }
                if entry.file_type().is_file() {
                    let path = entry.path();
                    if let Ok(rel) = path.strip_prefix(&search_path) {
                        let rel_str = rel.to_string_lossy();
                        if globset.is_match(&*rel_str) || globset.is_match(path) {
                            matches.push(path.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
        let truncated = matches.len() > max_files;
        matches.truncate(max_files);
        let result = serde_json::json!({
            "matches": matches,
            "count": matches.len(),
            "truncated": truncated,
        });
        ToolResult::ok(result, start)
    }
}

// ─── Grep Tool ───────────────────────────────────────────────

pub struct GrepTool { workspace: PathBuf }

impl GrepTool {
    pub fn new(workspace: PathBuf) -> Self { Self { workspace } }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str { "grep_search" }
    fn description(&self) -> &str { "Search file contents with regex" }
    fn category(&self) -> &str { "filesystem" }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let pattern = match params.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::err("Missing 'pattern' parameter", start),
        };
        let search_path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => PathBuf::from(p),
            None => self.workspace.clone(),
        };
        let max_files = params.get("max_files").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
        let Ok(re) = regex::Regex::new(pattern) else {
            return ToolResult::err(format!("Invalid regex pattern: {pattern}"), start);
        };
        let mut results = Vec::new();
        let mut files_searched = 0;
        if search_path.exists() && search_path.is_dir() {
            for entry in walkdir::WalkDir::new(&search_path).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_file() && files_searched < max_files {
                    files_searched += 1;
                    if let Ok(content) = tokio::fs::read_to_string(entry.path()).await {
                        for (i, line) in content.lines().enumerate() {
                            if re.is_match(line) {
                                results.push(serde_json::json!({
                                    "file": entry.path().to_string_lossy(),
                                    "line": i + 1,
                                    "content": line.trim()
                                }));
                            }
                        }
                    }
                }
            }
        }
        ToolResult::ok(serde_json::json!({
            "matches": results, "count": results.len(), "files_searched": files_searched
        }), start)
    }
}

// ─── Tool Registry ────────────────────────────────────────────

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    execution_log: Vec<serde_json::Value>,
    config_workspace: PathBuf,
}

impl ToolRegistry {
    pub fn new(workspace: PathBuf) -> Self {
        Self { tools: HashMap::new(), execution_log: Vec::new(), config_workspace: workspace.clone() }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    pub fn register_builtins(&mut self) {
        let ws = self.config_workspace.clone();
        self.register(Box::new(ReadTool::new(ws.clone())));
        self.register(Box::new(WriteTool::new(ws.clone())));
        self.register(Box::new(BashTool::new(ws.clone())));
        self.register(Box::new(GlobTool::new(ws.clone())));
        self.register(Box::new(GrepTool::new(ws)));
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn list_tools(&self) -> Vec<serde_json::Value> {
        self.tools.values().map(|t| {
            serde_json::json!({"name": t.name(), "description": t.description(), "category": t.category()})
        }).collect()
    }

    pub async fn execute(&mut self, name: &str, params: &HashMap<String, serde_json::Value>) -> serde_json::Value {
        let Some(tool) = self.tools.get(name) else {
            return serde_json::json!({"success": false, "error": format!("Tool '{name}' not found")});
        };
        let result = tool.execute(params).await;
        self.execution_log.push(serde_json::json!({
            "tool": name, "params": params, "success": result.success, "duration_ms": result.duration_ms,
        }));
        serde_json::json!({"success": result.success, "data": result.data, "error": result.error, "duration_ms": result.duration_ms})
    }

    pub fn get_stats(&self) -> serde_json::Value {
        let mut by_category: HashMap<String, usize> = HashMap::new();
        for tool in self.tools.values() {
            *by_category.entry(tool.category().to_string()).or_insert(0) += 1;
        }
        serde_json::json!({"total_tools": self.tools.len(), "total_executions": self.execution_log.len(), "tools_by_category": by_category})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_result_ok() {
        let start = Instant::now();
        let r = ToolResult::ok(serde_json::json!({"ok": true}), start);
        assert!(r.success);
        assert!(r.data.is_some());
    }

    #[test]
    fn tool_result_err() {
        let start = Instant::now();
        let r = ToolResult::err("test error", start);
        assert!(!r.success);
        assert!(r.error.is_some());
    }
}
