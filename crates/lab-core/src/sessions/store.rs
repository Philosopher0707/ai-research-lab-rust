//! Persistent session store with disk storage and query support.
//! Mirrors core/sessions/store.py (425 lines — partial implementation)

use crate::errors::{LabError, Result};
use crate::types::{LabSession, SessionStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, warn};

// ─── SessionRecord ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub closed_at: Option<String>,
    #[serde(default = "default_active")]
    pub status: String,
    #[serde(default)]
    pub agents_count: usize,
    #[serde(default)]
    pub tasks_completed: usize,
    #[serde(default)]
    pub tasks_failed: usize,
    #[serde(default)]
    pub total_tokens_used: usize,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    pub summary_path: Option<String>,
    #[serde(default)]
    pub memory_keys: Vec<String>,
    #[serde(default)]
    pub pipeline_runs: Vec<String>,
}

fn default_active() -> String { "active".into() }

impl SessionRecord {
    pub fn from_session(session: &LabSession) -> Self {
        Self {
            id: session.id.clone(),
            name: session.name.clone(),
            created_at: session.started_at.to_rfc3339(),
            closed_at: None,
            status: format!("{:?}", session.status).to_lowercase(),
            agents_count: session.agents_active,
            tasks_completed: session.tasks_completed,
            tasks_failed: session.tasks_failed,
            total_tokens_used: session.total_tokens_used,
            artifacts: session.artifacts.clone(),
            tags: Vec::new(),
            metadata: session.metadata.clone(),
            summary_path: None,
            memory_keys: Vec::new(),
            pipeline_runs: Vec::new(),
        }
    }

    pub fn to_session(&self) -> Result<LabSession> {
        let started_at = DateTime::parse_from_rfc3339(&self.created_at)
            .map_err(|e| LabError::ParseError(format!("Invalid created_at: {}", e)))?
            .with_timezone(&Utc);
        Ok(LabSession {
            id: self.id.clone(),
            name: self.name.clone(),
            started_at,
            agents_active: self.agents_count,
            tasks_completed: self.tasks_completed,
            tasks_failed: self.tasks_failed,
            total_tokens_used: self.total_tokens_used,
            status: match self.status.as_str() {
                "active" => SessionStatus::Active,
                "paused" => SessionStatus::Paused,
                "completed" => SessionStatus::Completed,
                "failed" => SessionStatus::Failed,
                _ => SessionStatus::Active,
            },
            artifacts: self.artifacts.clone(),
            metadata: self.metadata.clone(),
        })
    }
}

// ─── SessionStore ───────────────────────────────────────────────────

pub struct SessionStore {
    pub store_root: PathBuf,
    pub sessions_dir: PathBuf,
    pub summaries_dir: PathBuf,
    pub index_path: PathBuf,
    index: HashMap<String, serde_json::Value>,
    records: HashMap<String, SessionRecord>,
}

impl SessionStore {
    pub fn new(store_root: PathBuf) -> Self {
        let sessions_dir = store_root.join("sessions");
        let summaries_dir = store_root.join("summaries");
        let index_path = store_root.join("index.json");

        let _ = std::fs::create_dir_all(&sessions_dir);
        let _ = std::fs::create_dir_all(&summaries_dir);

        let mut store = Self {
            store_root,
            sessions_dir,
            summaries_dir,
            index_path,
            index: HashMap::new(),
            records: HashMap::new(),
        };
        store.load_index();
        store
    }

    /// Persist a session record.
    pub fn save(
        &mut self,
        session: &LabSession,
        tags: Option<Vec<String>>,
        summary_text: Option<&str>,
    ) -> Result<SessionRecord> {
        let mut record = SessionRecord::from_session(session);

        if let Some(t) = tags {
            record.tags = t;
        }

        // Write session JSON
        let session_file = self.sessions_dir.join(format!("{}.json", record.id));
        let content = serde_json::to_string_pretty(&record)?;
        std::fs::write(&session_file, content)?;

        // Write summary if provided
        if let Some(summary) = summary_text {
            let summary_file = self.summaries_dir.join(format!("{}.md", record.id));
            std::fs::write(&summary_file, summary)?;
            record.summary_path = Some(summary_file.to_string_lossy().to_string());
            // Re-write with summary_path
            let content = serde_json::to_string_pretty(&record)?;
            std::fs::write(&session_file, content)?;
        }

        // Update index
        self.index.insert(record.id.clone(), serde_json::json!({
            "name": record.name,
            "status": record.status,
            "created_at": record.created_at,
            "tags": record.tags,
            "agents_count": record.agents_count,
            "tasks_completed": record.tasks_completed,
        }));
        self.records.insert(record.id.clone(), record.clone());
        self.save_index()?;

        info!("Session persisted: {} ({})", record.id, record.name);
        Ok(record)
    }

    /// Load a session record from disk.
    pub fn load_record(&mut self, session_id: &str) -> Result<Option<SessionRecord>> {
        if let Some(record) = self.records.get(session_id) {
            return Ok(Some(record.clone()));
        }

        let session_file = self.sessions_dir.join(format!("{}.json", session_id));
        if !session_file.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&session_file)?;
        let record: SessionRecord = serde_json::from_str(&content)
            .map_err(|e| LabError::ParseError(format!("Failed to load session {}: {}", session_id, e)))?;
        self.records.insert(session_id.to_string(), record.clone());
        Ok(Some(record))
    }

    /// Load summary text for a session.
    pub fn load_summary(&self, session_id: &str) -> Option<String> {
        let summary_file = self.summaries_dir.join(format!("{}.md", session_id));
        if summary_file.exists() {
            std::fs::read_to_string(&summary_file).ok()
        } else {
            None
        }
    }

    /// Query persisted sessions with filters.
    pub fn query(
        &mut self,
        status: Option<&str>,
        tag: Option<&str>,
        name_query: Option<&str>,
        min_tasks: usize,
        min_agents: usize,
    ) -> Result<Vec<SessionRecord>> {
        let mut results = Vec::new();

        let ids: Vec<String> = self.index.keys().cloned().collect();
        for sid in ids {
            if let Some(record) = self.load_record(&sid)? {
                if let Some(s) = status {
                    if record.status != s { continue; }
                }
                if let Some(t) = tag {
                    if !record.tags.contains(&t.to_string()) { continue; }
                }
                if let Some(nq) = name_query {
                    if !record.name.to_lowercase().contains(&nq.to_lowercase()) { continue; }
                }
                if record.tasks_completed < min_tasks { continue; }
                if record.agents_count < min_agents { continue; }
                results.push(record);
            }
        }

        // Sort by created_at descending
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(results)
    }

    /// Remove a session from disk and index.
    pub fn purge(&mut self, session_id: &str) -> bool {
        self.records.remove(session_id);
        self.index.remove(session_id);

        let session_file = self.sessions_dir.join(format!("{}.json", session_id));
        let summary_file = self.summaries_dir.join(format!("{}.md", session_id));

        for f in &[session_file, summary_file] {
            if f.exists() {
                let _ = std::fs::remove_file(f);
            }
        }
        let _ = self.save_index();
        true
    }

    /// Get aggregate statistics.
    pub fn statistics(&mut self) -> Result<serde_json::Value> {
        let records = self.query(None, None, None, 0, 0)?;
        if records.is_empty() {
            return Ok(serde_json::json!({"total_sessions": 0}));
        }

        let mut statuses: HashMap<String, usize> = HashMap::new();
        let mut total_agents = 0;
        let mut total_tasks = 0;
        let mut total_tasks_failed = 0;
        let mut total_tokens = 0;

        for r in &records {
            *statuses.entry(r.status.clone()).or_insert(0) += 1;
            total_agents += r.agents_count;
            total_tasks += r.tasks_completed;
            total_tasks_failed += r.tasks_failed;
            total_tokens += r.total_tokens_used;
        }

        let n = records.len();
        let completed = statuses.get("completed").copied().unwrap_or(0);
        Ok(serde_json::json!({
            "total_sessions": n,
            "by_status": statuses,
            "average_agents": (total_agents as f64 / n as f64).round(),
            "average_tasks_completed": (total_tasks as f64 / n as f64).round(),
            "total_tasks_completed": total_tasks,
            "total_tasks_failed": total_tasks_failed,
            "total_tokens_used": total_tokens,
            "success_rate": (completed as f64 / n as f64 * 100.0).round(),
        }))
    }

    // ─── Internal ──────────────────────────────────────────────

    fn load_index(&mut self) {
        if self.index_path.exists() {
            match std::fs::read_to_string(&self.index_path) {
                Ok(content) => {
                    match serde_json::from_str(&content) {
                        Ok(index) => self.index = index,
                        Err(e) => {
                            warn!("Corrupt session index, starting fresh: {}", e);
                            self.index = HashMap::new();
                        }
                    }
                }
                Err(e) => warn!("Failed to read session index: {}", e),
            }
        }
        info!("SessionStore loaded {} session entries", self.index.len());
    }

    fn save_index(&self) -> std::io::Result<()> {
        let content = serde_json::to_string_pretty(&self.index)?;
        std::fs::write(&self.index_path, content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_record_roundtrip() {
        let session = LabSession::new("test-123".into(), "test-session".into());
        let record = SessionRecord::from_session(&session);
        assert_eq!(record.id, "test-123");
        assert_eq!(record.name, "test-session");
        assert_eq!(record.status, "active");

        let restored = record.to_session().unwrap();
        assert_eq!(restored.id, "test-123");
        assert_eq!(restored.name, "test-session");
    }
}
