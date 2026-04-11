//! Lab Tools — tool registry, built-in tools, and plugin registration.

use async_trait::async_trait;
use globset::{Glob, GlobSetBuilder};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

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
        Self {
            success: true,
            data: Some(data),
            error: None,
            duration_ms: start.elapsed().as_secs_f64() * 1000.0,
        }
    }

    pub fn err(message: impl Into<String>, start: Instant) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(message.into()),
            duration_ms: start.elapsed().as_secs_f64() * 1000.0,
        }
    }
}

// ─── Tool Traits ────────────────────────────────────────────────

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn category(&self) -> &str;
    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult;
}

pub trait ToolPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }
    fn tools(&self, workspace: &Path) -> Vec<Box<dyn Tool>>;
}

// ─── Path Helpers ───────────────────────────────────────────────

fn workspace_root(workspace: &Path) -> PathBuf {
    workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf())
}

fn has_parent_traversal(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn resolve_workspace_path(
    workspace: &Path,
    input: &str,
    require_exists: bool,
) -> Result<PathBuf, String> {
    let requested = PathBuf::from(input);
    if has_parent_traversal(&requested) {
        return Err("Path traversal not allowed".to_string());
    }

    let root = workspace_root(workspace);
    let candidate = if requested.is_absolute() {
        requested
    } else {
        root.join(requested)
    };

    if require_exists {
        let resolved = candidate
            .canonicalize()
            .map_err(|_| format!("Path not found: {input}"))?;
        if !resolved.starts_with(&root) {
            return Err("Access denied: path escapes workspace".to_string());
        }
        Ok(resolved)
    } else {
        if candidate.is_absolute() && !candidate.starts_with(&root) {
            return Err("Access denied: path escapes workspace".to_string());
        }
        Ok(candidate)
    }
}

fn relative_display(workspace: &Path, path: &Path) -> String {
    let root = workspace_root(workspace);
    path.strip_prefix(&root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

// ─── Filesystem Tools ────────────────────────────────────────────

pub struct ReadTool {
    workspace: PathBuf,
}

impl ReadTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file"
    }

    fn category(&self) -> &str {
        "filesystem"
    }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let path = match params.get("path").and_then(|value| value.as_str()) {
            Some(path) => path,
            None => return ToolResult::err("Missing 'path' parameter", start),
        };

        let resolved = match resolve_workspace_path(&self.workspace, path, true) {
            Ok(path) => path,
            Err(error) => return ToolResult::err(error, start),
        };

        match tokio::fs::read_to_string(&resolved).await {
            Ok(content) => {
                let all_lines: Vec<&str> = content.lines().collect();
                let total_lines = all_lines.len();
                let limit = params
                    .get("limit")
                    .and_then(|value| value.as_u64())
                    .map(|value| value as usize);
                let offset = params
                    .get("offset")
                    .and_then(|value| value.as_u64())
                    .map(|value| value as usize)
                    .unwrap_or(0);
                let lines: Vec<String> = if let Some(limit) = limit {
                    all_lines
                        .iter()
                        .skip(offset)
                        .take(limit)
                        .map(|line| (*line).to_string())
                        .collect()
                } else {
                    all_lines
                        .iter()
                        .skip(offset)
                        .map(|line| (*line).to_string())
                        .collect()
                };

                ToolResult::ok(
                    serde_json::json!({
                        "path": relative_display(&self.workspace, &resolved),
                        "content": lines.join("\n"),
                        "total_lines": total_lines,
                    }),
                    start,
                )
            }
            Err(error) => ToolResult::err(error.to_string(), start),
        }
    }
}

pub struct WriteTool {
    workspace: PathBuf,
}

impl WriteTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file"
    }

    fn category(&self) -> &str {
        "filesystem"
    }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let path = match params.get("path").and_then(|value| value.as_str()) {
            Some(path) => path,
            None => return ToolResult::err("Missing 'path' parameter", start),
        };
        let content = match params.get("content").and_then(|value| value.as_str()) {
            Some(content) => content,
            None => return ToolResult::err("Missing 'content' parameter", start),
        };

        let resolved = match resolve_workspace_path(&self.workspace, path, false) {
            Ok(path) => path,
            Err(error) => return ToolResult::err(error, start),
        };

        if let Some(parent) = resolved.parent() {
            if let Err(error) = tokio::fs::create_dir_all(parent).await {
                return ToolResult::err(error.to_string(), start);
            }
        }

        match tokio::fs::write(&resolved, content).await {
            Ok(()) => ToolResult::ok(
                serde_json::json!({
                    "path": relative_display(&self.workspace, &resolved),
                    "bytes_written": content.len(),
                }),
                start,
            ),
            Err(error) => ToolResult::err(error.to_string(), start),
        }
    }
}

pub struct ListDirectoryTool {
    workspace: PathBuf,
}

impl ListDirectoryTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List files and directories inside a workspace path"
    }

    fn category(&self) -> &str {
        "filesystem"
    }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let requested_path = params
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or(".");
        let limit = params
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(200) as usize;

        let resolved = match resolve_workspace_path(&self.workspace, requested_path, true) {
            Ok(path) => path,
            Err(error) => return ToolResult::err(error, start),
        };

        let metadata = match tokio::fs::metadata(&resolved).await {
            Ok(metadata) => metadata,
            Err(error) => return ToolResult::err(error.to_string(), start),
        };
        if !metadata.is_dir() {
            return ToolResult::err("Path is not a directory", start);
        }

        let mut entries = match tokio::fs::read_dir(&resolved).await {
            Ok(entries) => entries,
            Err(error) => return ToolResult::err(error.to_string(), start),
        };

        let mut rows = Vec::new();
        while rows.len() < limit {
            match entries.next_entry().await {
                Ok(Some(entry)) => {
                    let entry_path = entry.path();
                    let entry_metadata = entry.metadata().await.ok();
                    let kind = entry_metadata
                        .as_ref()
                        .map(|metadata| {
                            if metadata.is_dir() {
                                "dir"
                            } else if metadata.is_file() {
                                "file"
                            } else {
                                "other"
                            }
                        })
                        .unwrap_or("unknown");
                    rows.push(serde_json::json!({
                        "name": entry.file_name().to_string_lossy(),
                        "path": relative_display(&self.workspace, &entry_path),
                        "kind": kind,
                        "size_bytes": entry_metadata.map(|metadata| metadata.len()),
                    }));
                }
                Ok(None) => break,
                Err(error) => return ToolResult::err(error.to_string(), start),
            }
        }

        ToolResult::ok(
            serde_json::json!({
                "path": relative_display(&self.workspace, &resolved),
                "entries": rows,
                "count": rows.len(),
            }),
            start,
        )
    }
}

pub struct FileInfoTool {
    workspace: PathBuf,
}

impl FileInfoTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for FileInfoTool {
    fn name(&self) -> &str {
        "file_info"
    }

    fn description(&self) -> &str {
        "Return file metadata for a workspace path"
    }

    fn category(&self) -> &str {
        "filesystem"
    }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let path = match params.get("path").and_then(|value| value.as_str()) {
            Some(path) => path,
            None => return ToolResult::err("Missing 'path' parameter", start),
        };

        let resolved = match resolve_workspace_path(&self.workspace, path, true) {
            Ok(path) => path,
            Err(error) => return ToolResult::err(error, start),
        };

        match tokio::fs::metadata(&resolved).await {
            Ok(metadata) => ToolResult::ok(
                serde_json::json!({
                    "path": relative_display(&self.workspace, &resolved),
                    "absolute_path": resolved.to_string_lossy(),
                    "is_file": metadata.is_file(),
                    "is_dir": metadata.is_dir(),
                    "size_bytes": metadata.len(),
                    "readonly": metadata.permissions().readonly(),
                }),
                start,
            ),
            Err(error) => ToolResult::err(error.to_string(), start),
        }
    }
}

pub struct GlobTool {
    workspace: PathBuf,
}

impl GlobTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob_search"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern"
    }

    fn category(&self) -> &str {
        "filesystem"
    }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let pattern = match params.get("pattern").and_then(|value| value.as_str()) {
            Some(pattern) => pattern,
            None => return ToolResult::err("Missing 'pattern' parameter", start),
        };
        let search_path = match params.get("path").and_then(|value| value.as_str()) {
            Some(path) => match resolve_workspace_path(&self.workspace, path, true) {
                Ok(path) => path,
                Err(error) => return ToolResult::err(error, start),
            },
            None => workspace_root(&self.workspace),
        };

        let mut builder = GlobSetBuilder::new();
        let Ok(glob) = Glob::new(pattern) else {
            return ToolResult::err("Invalid glob pattern", start);
        };
        builder.add(glob);
        let Ok(globset) = builder.build() else {
            return ToolResult::err("Invalid glob pattern", start);
        };

        let max_files = params
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(500) as usize;
        let mut matches = Vec::new();

        for entry in walkdir::WalkDir::new(&search_path)
            .max_depth(12)
            .follow_links(false)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if matches.len() >= max_files {
                break;
            }

            if let Some(name) = entry.file_name().to_str() {
                if matches!(
                    name,
                    ".git"
                        | ".hg"
                        | ".svn"
                        | "node_modules"
                        | ".cargo"
                        | ".rustup"
                        | "__pycache__"
                        | ".nvm"
                ) {
                    continue;
                }
                if name.starts_with('.') && entry.file_type().is_dir() {
                    continue;
                }
            }

            if entry.file_type().is_file() {
                let path = entry.path();
                let relative = relative_display(&self.workspace, path);
                if globset.is_match(&relative) || globset.is_match(path) {
                    matches.push(relative);
                }
            }
        }

        ToolResult::ok(
            serde_json::json!({
                "matches": matches,
                "count": matches.len(),
                "truncated": matches.len() >= max_files,
            }),
            start,
        )
    }
}

pub struct GrepTool {
    workspace: PathBuf,
}

impl GrepTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep_search"
    }

    fn description(&self) -> &str {
        "Search file contents with regex"
    }

    fn category(&self) -> &str {
        "filesystem"
    }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let pattern = match params.get("pattern").and_then(|value| value.as_str()) {
            Some(pattern) => pattern,
            None => return ToolResult::err("Missing 'pattern' parameter", start),
        };
        let search_path = match params.get("path").and_then(|value| value.as_str()) {
            Some(path) => match resolve_workspace_path(&self.workspace, path, true) {
                Ok(path) => path,
                Err(error) => return ToolResult::err(error, start),
            },
            None => workspace_root(&self.workspace),
        };
        let max_files = params
            .get("max_files")
            .and_then(|value| value.as_u64())
            .unwrap_or(100) as usize;
        let context_lines = params
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        // Optional glob filter (e.g. "*.rs", "**/*.toml")
        let file_glob: Option<globset::GlobSet> =
            params.get("glob").and_then(|v| v.as_str()).and_then(|g| {
                let mut b = GlobSetBuilder::new();
                b.add(Glob::new(g).ok()?);
                b.build().ok()
            });

        let Ok(regex) = regex::Regex::new(pattern) else {
            return ToolResult::err(format!("Invalid regex pattern: {pattern}"), start);
        };

        let mut results = Vec::new();
        let mut files_searched = 0usize;

        for entry in walkdir::WalkDir::new(&search_path)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if !entry.file_type().is_file() || files_searched >= max_files {
                continue;
            }

            // Apply glob filter if provided
            if let Some(ref gs) = file_glob {
                let rel = relative_display(&self.workspace, entry.path());
                let name = entry.file_name().to_string_lossy().to_string();
                if !gs.is_match(&rel) && !gs.is_match(&name) {
                    continue;
                }
            }

            files_searched += 1;
            if let Ok(content) = tokio::fs::read_to_string(entry.path()).await {
                let lines: Vec<&str> = content.lines().collect();
                for (index, line) in lines.iter().enumerate() {
                    if regex.is_match(line) {
                        let mut entry_json = serde_json::json!({
                            "file": relative_display(&self.workspace, entry.path()),
                            "line": index + 1,
                            "content": line.trim(),
                        });
                        if context_lines > 0 {
                            let before_start = index.saturating_sub(context_lines);
                            let after_end = (index + context_lines + 1).min(lines.len());
                            let before: Vec<&str> = lines[before_start..index].to_vec();
                            let after: Vec<&str> = lines[index + 1..after_end].to_vec();
                            entry_json["context_before"] = serde_json::json!(before);
                            entry_json["context_after"] = serde_json::json!(after);
                        }
                        results.push(entry_json);
                    }
                }
            }
        }

        ToolResult::ok(
            serde_json::json!({
                "matches": results,
                "count": results.len(),
                "files_searched": files_searched,
            }),
            start,
        )
    }
}

// ─── System / VCS Tools ───────────────────────────────────────────

pub struct BashTool {
    workspace: PathBuf,
}

impl BashTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command"
    }

    fn category(&self) -> &str {
        "system"
    }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let command = match params.get("command").and_then(|value| value.as_str()) {
            Some(command) => command,
            None => return ToolResult::err("Missing 'command' parameter", start),
        };
        let timeout_secs = params
            .get("timeout")
            .and_then(|value| value.as_u64())
            .unwrap_or(60);

        match tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            tokio::process::Command::new("bash")
                .arg("-c")
                .arg(command)
                .current_dir(&self.workspace)
                .output(),
        )
        .await
        {
            Ok(Ok(output)) => ToolResult::ok(
                serde_json::json!({
                    "stdout": String::from_utf8_lossy(&output.stdout),
                    "stderr": String::from_utf8_lossy(&output.stderr),
                    "returncode": output.status.code().unwrap_or(-1),
                }),
                start,
            ),
            Ok(Err(error)) => ToolResult::err(error.to_string(), start),
            Err(_) => ToolResult::err(format!("Command timed out after {timeout_secs}s"), start),
        }
    }
}

pub struct GitStatusTool {
    workspace: PathBuf,
}

impl GitStatusTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for GitStatusTool {
    fn name(&self) -> &str {
        "git_status"
    }

    fn description(&self) -> &str {
        "Inspect git branch and working tree status"
    }

    fn category(&self) -> &str {
        "vcs"
    }

    async fn execute(&self, _params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        match tokio::process::Command::new("git")
            .arg("status")
            .arg("--short")
            .arg("--branch")
            .current_dir(&self.workspace)
            .output()
            .await
        {
            Ok(output) if output.status.success() => ToolResult::ok(
                serde_json::json!({
                    "status": String::from_utf8_lossy(&output.stdout),
                    "returncode": output.status.code().unwrap_or(0),
                }),
                start,
            ),
            Ok(output) => ToolResult::err(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
                start,
            ),
            Err(error) => ToolResult::err(error.to_string(), start),
        }
    }
}

// ─── HTML helpers (used by FetchUrlTool) ─────────────────────────

fn strip_html(html: &str) -> String {
    use std::sync::OnceLock;
    static SCRIPT_RE: OnceLock<regex::Regex> = OnceLock::new();
    static TAG_RE: OnceLock<regex::Regex> = OnceLock::new();
    static WS_RE: OnceLock<regex::Regex> = OnceLock::new();

    let script_re = SCRIPT_RE.get_or_init(|| {
        regex::Regex::new(r"(?is)<(script|style)[^>]*>.*?</(script|style)>").unwrap()
    });
    let tag_re = TAG_RE.get_or_init(|| regex::Regex::new(r"<[^>]+>").unwrap());
    let ws_re = WS_RE.get_or_init(|| regex::Regex::new(r"\s+").unwrap());

    let step1 = script_re.replace_all(html, " ");
    let step2 = tag_re.replace_all(step1.as_ref(), " ");
    let step3 = ws_re.replace_all(step2.as_ref(), " ");

    step3
        .into_owned()
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .trim()
        .to_string()
}

// ─── Network / Web Search Tools ──────────────────────────────────

pub struct FetchUrlTool;

#[async_trait]
impl Tool for FetchUrlTool {
    fn name(&self) -> &str {
        "fetch_url"
    }

    fn description(&self) -> &str {
        "Fetch a URL and return its text content (HTML is stripped)"
    }

    fn category(&self) -> &str {
        "network"
    }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let url = match params.get("url").and_then(|v| v.as_str()) {
            Some(u) => u.to_string(),
            None => return ToolResult::err("Missing 'url' parameter", start),
        };
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return ToolResult::err("URL must start with http:// or https://", start);
        }
        let max_chars = params
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .unwrap_or(8000) as usize;

        let client = match reqwest::Client::builder()
            .user_agent("lab-research-tool/0.2.0")
            .timeout(std::time::Duration::from_secs(20))
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolResult::err(e.to_string(), start),
        };

        let response = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => return ToolResult::err(format!("Request failed: {e}"), start),
        };

        if !response.status().is_success() {
            return ToolResult::err(format!("HTTP {}", response.status()), start);
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body = match response.text().await {
            Ok(t) => t,
            Err(e) => return ToolResult::err(format!("Failed to read response: {e}"), start),
        };

        let text = if content_type.contains("html") {
            strip_html(&body)
        } else {
            body
        };

        let truncated = text.len() > max_chars;
        let text = if truncated {
            // Truncate at a char boundary
            text.char_indices()
                .nth(max_chars)
                .map(|(i, _)| &text[..i])
                .unwrap_or(&text)
                .to_string()
        } else {
            text
        };
        let char_count = text.chars().count();

        ToolResult::ok(
            serde_json::json!({
                "url": url,
                "content": text,
                "content_type": content_type,
                "char_count": char_count,
                "truncated": truncated,
            }),
            start,
        )
    }
}

// ─── DDG HTML fallback ───────────────────────────────────────────
//
// The Instant Answer API often returns 0 results for multi-word or
// technical queries.  When that happens we fall back to DDG's JS-free
// HTML page (html.duckduckgo.com/html/) and parse result links out of
// the returned markup with lightweight regex extraction.

/// Percent-decode a URL-encoded string (handles %XX sequences only;
/// `+` is left as-is since DDG href values don't use it for spaces).
fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((((h << 4) | l) as u8) as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Resolve a URL that may be a DDG redirect (`/l/?uddg=<encoded>` or
/// `//duckduckgo.com/l/?uddg=<encoded>`) to the real destination URL.
fn resolve_ddg_url(raw: &str) -> String {
    if let Some(pos) = raw.find("uddg=") {
        let encoded = &raw[pos + 5..];
        // Strip any trailing query params after the encoded URL
        let encoded = encoded.split('&').next().unwrap_or(encoded);
        let decoded = percent_decode(encoded);
        if decoded.starts_with("http://") || decoded.starts_with("https://") {
            return decoded;
        }
    }
    // Normalise protocol-relative URLs
    if raw.starts_with("//") {
        return format!("https:{raw}");
    }
    raw.to_string()
}

/// Strip all HTML tags and collapse whitespace to a single space.
fn strip_tags(s: &str) -> String {
    use std::sync::OnceLock;
    static TAG_RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = TAG_RE.get_or_init(|| regex::Regex::new(r"<[^>]+>").unwrap());
    re.replace_all(s, " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&nbsp;", " ")
}

/// Parse DDG HTML-page markup into a list of `{title, url, snippet}` objects.
///
/// DDG lite markup looks like:
/// ```html
/// <a rel="nofollow" class="result__a" href="https://…">Title text</a>
/// …
/// <a class="result__snippet" href="…">Snippet text here.</a>
/// ```
fn parse_ddg_html_results(html: &str, max: usize) -> Vec<serde_json::Value> {
    use std::sync::OnceLock;
    // Opening <a> tag + inner text  (group 1 = tag, group 2 = inner text)
    static ANCHOR_RE: OnceLock<regex::Regex> = OnceLock::new();
    // href value inside any tag
    static HREF_RE: OnceLock<regex::Regex> = OnceLock::new();
    // result__snippet anchor inner text
    static SNIPPET_RE: OnceLock<regex::Regex> = OnceLock::new();

    let anchor_re = ANCHOR_RE.get_or_init(|| {
        regex::Regex::new(r#"(?si)(<a[^>]*\bclass="result__a"[^>]*>)(.*?)</a>"#).unwrap()
    });
    let href_re = HREF_RE.get_or_init(|| regex::Regex::new(r#"(?i)\bhref="([^"]+)""#).unwrap());
    let snippet_re = SNIPPET_RE.get_or_init(|| {
        regex::Regex::new(r#"(?si)\bclass="result__snippet"[^>]*>(.*?)</a>"#).unwrap()
    });

    // Collect (url, title) pairs from result__a links
    let links: Vec<(String, String)> = anchor_re
        .captures_iter(html)
        .filter_map(|cap| {
            let open_tag = cap.get(1)?.as_str();
            let inner = cap.get(2)?.as_str();
            let raw_url = href_re
                .captures(open_tag)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str())?;
            let url = resolve_ddg_url(raw_url);
            if !url.starts_with("http") {
                return None;
            }
            let title = strip_tags(inner);
            if title.is_empty() {
                return None;
            }
            Some((url, title))
        })
        .collect();

    // Collect snippets in document order
    let snippets: Vec<String> = snippet_re
        .captures_iter(html)
        .map(|cap| strip_tags(cap.get(1).map(|m| m.as_str()).unwrap_or("")))
        .collect();

    // Zip and return up to max results
    links
        .into_iter()
        .zip(snippets.into_iter().chain(std::iter::repeat(String::new())))
        .take(max)
        .map(|((url, title), snippet)| {
            serde_json::json!({"title": title, "url": url, "snippet": snippet})
        })
        .collect()
}

/// Fetch the DDG lite HTML page for `query` and parse results from it.
/// Returns an empty Vec on any network or parse error (silent fallback).
async fn ddg_html_fallback(
    client: &reqwest::Client,
    query: &str,
    max_results: usize,
) -> Vec<serde_json::Value> {
    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        query.split_whitespace().collect::<Vec<_>>().join("+")
    );

    let html = match client
        .get(&url)
        .header("Accept", "text/html,application/xhtml+xml")
        // DDG requires a real browser-like Accept-Language header to return results
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
    {
        Ok(r) => match r.text().await {
            Ok(t) => t,
            Err(_) => return Vec::new(),
        },
        Err(_) => return Vec::new(),
    };

    parse_ddg_html_results(&html, max_results)
}

pub struct DuckDuckGoTool;

#[async_trait]
impl Tool for DuckDuckGoTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web using DuckDuckGo Instant Answer API (no API key required)"
    }

    fn category(&self) -> &str {
        "network"
    }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let query = match params.get("query").and_then(|v| v.as_str()) {
            Some(q) => q.to_string(),
            None => return ToolResult::err("Missing 'query' parameter", start),
        };
        let max_results = params
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        let client = match reqwest::Client::builder()
            .user_agent("lab-research-tool/0.2.0")
            .timeout(std::time::Duration::from_secs(15))
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolResult::err(e.to_string(), start),
        };

        let response = match client
            .get("https://api.duckduckgo.com/")
            .query(&[
                ("q", query.as_str()),
                ("format", "json"),
                ("no_html", "1"),
                ("skip_disambig", "1"),
            ])
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return ToolResult::err(format!("Request failed: {e}"), start),
        };

        let body: serde_json::Value = match response.json().await {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Failed to parse response: {e}"), start),
        };

        let mut results: Vec<serde_json::Value> = Vec::new();

        // Collect from Results array
        if let Some(raw_results) = body.get("Results").and_then(|v| v.as_array()) {
            for item in raw_results {
                if results.len() >= max_results {
                    break;
                }
                let text = item
                    .get("Text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let url = item
                    .get("FirstURL")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !url.is_empty() {
                    results.push(serde_json::json!({
                        "title": text,
                        "url": url,
                        "snippet": text,
                    }));
                }
            }
        }

        // Collect from RelatedTopics, flattening nested category groups
        if let Some(topics) = body.get("RelatedTopics").and_then(|v| v.as_array()) {
            for item in topics {
                if results.len() >= max_results {
                    break;
                }
                // Flat topic has FirstURL directly
                if let Some(url) = item.get("FirstURL").and_then(|v| v.as_str()) {
                    if !url.is_empty() {
                        let text = item
                            .get("Text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let (title, snippet) = match text.find(" - ") {
                            Some(pos) => (text[..pos].to_string(), text[pos + 3..].to_string()),
                            None => (text.clone(), text),
                        };
                        results.push(serde_json::json!({
                            "title": title,
                            "url": url,
                            "snippet": snippet,
                        }));
                    }
                }
                // Category group with nested topics
                if let Some(sub_topics) = item.get("Topics").and_then(|v| v.as_array()) {
                    for sub in sub_topics {
                        if results.len() >= max_results {
                            break;
                        }
                        if let Some(url) = sub.get("FirstURL").and_then(|v| v.as_str()) {
                            if !url.is_empty() {
                                let text = sub
                                    .get("Text")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let (title, snippet) = match text.find(" - ") {
                                    Some(pos) => {
                                        (text[..pos].to_string(), text[pos + 3..].to_string())
                                    }
                                    None => (text.clone(), text),
                                };
                                results.push(serde_json::json!({
                                    "title": title,
                                    "url": url,
                                    "snippet": snippet,
                                }));
                            }
                        }
                    }
                }
            }
        }

        let answer = body
            .get("Answer")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                body.get("AbstractText")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
            })
            .map(|s| serde_json::Value::String(s.to_string()))
            .unwrap_or(serde_json::Value::Null);

        // ── Fallback: Instant Answer API returned nothing ────────
        // Fetch the DDG lite HTML page and scrape real search results.
        let (results, source) = if results.is_empty() && answer.is_null() {
            let fallback = ddg_html_fallback(&client, &query, max_results).await;
            let src = if fallback.is_empty() {
                "duckduckgo"
            } else {
                "duckduckgo-html"
            };
            (fallback, src)
        } else {
            (results, "duckduckgo")
        };

        ToolResult::ok(
            serde_json::json!({
                "query": query,
                "results": results,
                "answer": answer,
                "source": source,
            }),
            start,
        )
    }
}

pub struct TavilyTool;

#[async_trait]
impl Tool for TavilyTool {
    fn name(&self) -> &str {
        "tavily_search"
    }

    fn description(&self) -> &str {
        "Search the web using Tavily API (requires TAVILY_API_KEY environment variable)"
    }

    fn category(&self) -> &str {
        "network"
    }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let query = match params.get("query").and_then(|v| v.as_str()) {
            Some(q) => q.to_string(),
            None => return ToolResult::err("Missing 'query' parameter", start),
        };
        let api_key = match std::env::var("TAVILY_API_KEY") {
            Ok(key) if !key.is_empty() => key,
            _ => return ToolResult::err("TAVILY_API_KEY environment variable is not set", start),
        };
        let max_results = params
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;
        let search_depth = params
            .get("search_depth")
            .and_then(|v| v.as_str())
            .unwrap_or("basic")
            .to_string();
        let include_answer = params
            .get("include_answer")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let client = match reqwest::Client::builder()
            .user_agent("lab-research-tool/0.2.0")
            .timeout(std::time::Duration::from_secs(30))
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolResult::err(e.to_string(), start),
        };

        let body = serde_json::json!({
            "api_key": api_key,
            "query": query,
            "max_results": max_results,
            "search_depth": search_depth,
            "include_answer": include_answer,
        });

        let response = match client
            .post("https://api.tavily.com/search")
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return ToolResult::err(format!("Request failed: {e}"), start),
        };

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return ToolResult::err(format!("Tavily API error {status}: {text}"), start);
        }

        let data: serde_json::Value = match response.json().await {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!("Failed to parse response: {e}"), start),
        };

        let results: Vec<serde_json::Value> = data
            .get("results")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|item| {
                        serde_json::json!({
                            "title": item.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                            "url": item.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                            "snippet": item.get("content").and_then(|v| v.as_str()).unwrap_or(""),
                            "score": item.get("score").cloned().unwrap_or(serde_json::Value::Null),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let answer = data
            .get("answer")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        ToolResult::ok(
            serde_json::json!({
                "query": query,
                "results": results,
                "answer": answer,
                "source": "tavily",
            }),
            start,
        )
    }
}

// ─── Edit File Tool ──────────────────────────────────────────────

pub struct EditFileTool {
    workspace: PathBuf,
}

impl EditFileTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Find and replace text in a file (fails if old_string not found)"
    }

    fn category(&self) -> &str {
        "filesystem"
    }

    async fn execute(&self, params: &HashMap<String, serde_json::Value>) -> ToolResult {
        let start = Instant::now();
        let path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::err("Missing 'path' parameter", start),
        };
        let old_string = match params.get("old_string").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return ToolResult::err("Missing 'old_string' parameter", start),
        };
        let new_string = match params.get("new_string").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return ToolResult::err("Missing 'new_string' parameter", start),
        };
        let replace_all = params
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let resolved = match resolve_workspace_path(&self.workspace, path, true) {
            Ok(p) => p,
            Err(e) => return ToolResult::err(e, start),
        };

        let content = match tokio::fs::read_to_string(&resolved).await {
            Ok(c) => c,
            Err(e) => return ToolResult::err(e.to_string(), start),
        };

        let occurrences = content.matches(old_string.as_str()).count();
        if occurrences == 0 {
            return ToolResult::err("old_string not found in file", start);
        }

        let new_content = if replace_all {
            content.replace(old_string.as_str(), &new_string)
        } else {
            content.replacen(old_string.as_str(), &new_string, 1)
        };

        match tokio::fs::write(&resolved, new_content).await {
            Ok(()) => ToolResult::ok(
                serde_json::json!({
                    "path": relative_display(&self.workspace, &resolved),
                    "occurrences_found": occurrences,
                    "replacements_made": if replace_all { occurrences } else { 1 },
                }),
                start,
            ),
            Err(e) => ToolResult::err(e.to_string(), start),
        }
    }
}

// ─── Tool Plugins ────────────────────────────────────────────────

pub struct BuiltinToolPlugin;

impl ToolPlugin for BuiltinToolPlugin {
    fn name(&self) -> &str {
        "builtin-tools"
    }

    fn tools(&self, workspace: &Path) -> Vec<Box<dyn Tool>> {
        let workspace = workspace.to_path_buf();
        vec![
            Box::new(ReadTool::new(workspace.clone())),
            Box::new(WriteTool::new(workspace.clone())),
            Box::new(ListDirectoryTool::new(workspace.clone())),
            Box::new(FileInfoTool::new(workspace.clone())),
            Box::new(GlobTool::new(workspace.clone())),
            Box::new(GrepTool::new(workspace.clone())),
            Box::new(BashTool::new(workspace.clone())),
            Box::new(GitStatusTool::new(workspace.clone())),
            Box::new(EditFileTool::new(workspace)),
            Box::new(DuckDuckGoTool),
            Box::new(TavilyTool),
            Box::new(FetchUrlTool),
        ]
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginRegistration {
    pub name: String,
    pub version: String,
}

// ─── Tool Registry ───────────────────────────────────────────────

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    tool_sources: HashMap<String, String>,
    execution_log: Vec<serde_json::Value>,
    config_workspace: PathBuf,
    plugins: Vec<PluginRegistration>,
}

impl ToolRegistry {
    pub fn new(workspace: PathBuf) -> Self {
        Self {
            tools: HashMap::new(),
            tool_sources: HashMap::new(),
            execution_log: Vec::new(),
            config_workspace: workspace,
            plugins: Vec::new(),
        }
    }

    pub fn workspace(&self) -> &Path {
        &self.config_workspace
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.register_with_source(tool, "manual");
    }

    pub fn register_with_source(&mut self, tool: Box<dyn Tool>, source: impl Into<String>) {
        let name = tool.name().to_string();
        self.tool_sources.insert(name.clone(), source.into());
        self.tools.insert(name, tool);
    }

    pub fn register_builtins(&mut self) {
        self.register_plugin(Box::new(BuiltinToolPlugin));
    }

    pub fn register_plugin(&mut self, plugin: Box<dyn ToolPlugin>) {
        let plugin_name = plugin.name().to_string();
        let plugin_version = plugin.version().to_string();
        for tool in plugin.tools(&self.config_workspace) {
            self.register_with_source(tool, format!("{plugin_name}@{plugin_version}"));
        }
        self.plugins.push(PluginRegistration {
            name: plugin_name,
            version: plugin_version,
        });
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|tool| tool.as_ref())
    }

    pub fn list_tools(&self) -> Vec<serde_json::Value> {
        let mut tools: Vec<_> = self
            .tools
            .values()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "category": tool.category(),
                    "source": self.tool_sources.get(tool.name()).cloned().unwrap_or_else(|| "manual".to_string()),
                })
            })
            .collect();
        tools.sort_by(|left, right| {
            left["name"]
                .as_str()
                .unwrap_or_default()
                .cmp(right["name"].as_str().unwrap_or_default())
        });
        tools
    }

    pub async fn execute(
        &mut self,
        name: &str,
        params: &HashMap<String, serde_json::Value>,
    ) -> serde_json::Value {
        let Some(tool) = self.tools.get(name) else {
            return serde_json::json!({
                "success": false,
                "error": format!("Tool '{name}' not found")
            });
        };

        let result = tool.execute(params).await;
        self.execution_log.push(serde_json::json!({
            "tool": name,
            "source": self.tool_sources.get(name).cloned().unwrap_or_else(|| "manual".to_string()),
            "params": params,
            "success": result.success,
            "duration_ms": result.duration_ms,
        }));

        serde_json::json!({
            "success": result.success,
            "data": result.data,
            "error": result.error,
            "duration_ms": result.duration_ms,
        })
    }

    pub fn get_stats(&self) -> serde_json::Value {
        let mut by_category: HashMap<String, usize> = HashMap::new();
        for tool in self.tools.values() {
            *by_category.entry(tool.category().to_string()).or_insert(0) += 1;
        }

        serde_json::json!({
            "total_tools": self.tools.len(),
            "total_executions": self.execution_log.len(),
            "tools_by_category": by_category,
            "plugins": self.plugins,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_result_ok() {
        let start = Instant::now();
        let result = ToolResult::ok(serde_json::json!({"ok": true}), start);
        assert!(result.success);
        assert!(result.data.is_some());
    }

    #[test]
    fn tool_result_err() {
        let start = Instant::now();
        let result = ToolResult::err("test error", start);
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[test]
    fn builtins_register_plugin_metadata_and_richer_tools() {
        let workspace = tempfile::tempdir().unwrap();
        let mut registry = ToolRegistry::new(workspace.path().to_path_buf());
        registry.register_builtins();

        let tool_names: Vec<String> = registry
            .list_tools()
            .into_iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_string))
            .collect();

        assert!(tool_names.contains(&"read_file".to_string()));
        assert!(tool_names.contains(&"list_directory".to_string()));
        assert!(tool_names.contains(&"file_info".to_string()));
        assert!(tool_names.contains(&"git_status".to_string()));
        assert!(tool_names.contains(&"web_search".to_string()));
        assert!(tool_names.contains(&"tavily_search".to_string()));
        assert!(tool_names.contains(&"fetch_url".to_string()));
        assert!(tool_names.contains(&"edit_file".to_string()));
        assert_eq!(registry.plugins.len(), 1);
    }

    #[tokio::test]
    async fn list_directory_reports_entries() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("demo.txt"), "hello").unwrap();

        let mut registry = ToolRegistry::new(workspace.path().to_path_buf());
        registry.register_builtins();

        let result = registry.execute("list_directory", &HashMap::new()).await;

        assert!(result["success"].as_bool().unwrap_or(false));
        assert!(result["data"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["name"].as_str() == Some("demo.txt") }));
    }
}
