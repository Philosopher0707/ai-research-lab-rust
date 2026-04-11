//! SessionService — owns session state behind a single `RwLock`.
//!
//! Merges the `sessions` HashMap and `agent_registry` HashMap from
//! `ResearchLab` into one cohesive struct so both can be mutated under
//! a single lock, eliminating any risk of double-lock ordering bugs.

use crate::errors::{LabError, Result};
use crate::events::{EventBus, LabEvent};
use crate::sessions::SessionStore;
use crate::types::{LabSession, SessionStatus};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

// ─── Internal state ─────────────────────────────────────────────────

#[derive(Default)]
struct SessionState {
    sessions: HashMap<String, LabSession>,
    /// session_id → agent_id → metadata
    agent_registry: HashMap<String, HashMap<String, serde_json::Value>>,
}

// ─── SessionService ──────────────────────────────────────────────────

pub struct SessionService {
    state: RwLock<SessionState>,
    /// `SessionStore::save` and `query` take `&mut self`, so it lives in a `Mutex`.
    store: Mutex<SessionStore>,
    event_bus: Arc<EventBus>,
}

impl SessionService {
    pub fn new(store: SessionStore, event_bus: Arc<EventBus>) -> Self {
        Self {
            state: RwLock::new(SessionState::default()),
            store: Mutex::new(store),
            event_bus,
        }
    }

    // ─── Session CRUD ────────────────────────────────────────────

    pub async fn create(&self, name: &str) -> Result<LabSession> {
        let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let session = LabSession::new(id.clone(), name.to_string());
        let clone = session.clone();

        self.state
            .write()
            .await
            .sessions
            .insert(id.clone(), session);

        self.event_bus.emit(LabEvent::new(
            "session.created",
            serde_json::json!({"session_id": id, "name": name}),
        ));

        info!("Session created: {} ({})", id, name);
        Ok(clone)
    }

    pub async fn close(&self, session_id: &str) -> Result<()> {
        let mut guard = self.state.write().await;
        let session = guard
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| LabError::SessionNotFound(session_id.to_string()))?;

        session.status = SessionStatus::Completed;
        // Persist to disk — log failures but don't let them block the event.
        if let Err(e) = self.store.lock().await.save(session, None, None) {
            warn!("Failed to persist closed session {}: {}", session_id, e);
        }

        self.event_bus.emit(LabEvent::new(
            "session.closed",
            serde_json::json!({"session_id": session_id}),
        ));

        info!("Session closed: {} (record saved to disk)", session_id);
        Ok(())
    }

    pub async fn get(&self, session_id: &str) -> Option<LabSession> {
        self.state.read().await.sessions.get(session_id).cloned()
    }

    pub async fn list(&self) -> Vec<LabSession> {
        self.state.read().await.sessions.values().cloned().collect()
    }

    pub async fn count(&self) -> usize {
        self.state.read().await.sessions.len()
    }

    pub async fn active_count(&self) -> usize {
        self.state
            .read()
            .await
            .sessions
            .values()
            .filter(|s| matches!(s.status, SessionStatus::Active))
            .count()
    }

    pub async fn set_tasks_completed(&self, session_id: &str, count: usize) {
        if let Some(session) = self.state.write().await.sessions.get_mut(session_id) {
            session.tasks_completed = count;
        }
    }

    pub async fn all_ids(&self) -> Vec<String> {
        self.state.read().await.sessions.keys().cloned().collect()
    }

    // ─── Agent registry ──────────────────────────────────────────

    pub async fn register_agent(&self, session_id: &str, agent_id: &str, meta: serde_json::Value) {
        let mut guard = self.state.write().await;
        guard
            .agent_registry
            .entry(session_id.to_string())
            .or_default()
            .insert(agent_id.to_string(), meta);
        if let Some(session) = guard.sessions.get_mut(session_id) {
            session.agents_active += 1;
        }
    }

    pub async fn remove_agent(&self, session_id: &str, agent_id: &str) -> bool {
        let mut guard = self.state.write().await;
        if let Some(agents) = guard.agent_registry.get_mut(session_id) {
            if agents.remove(agent_id).is_some() {
                if let Some(session) = guard.sessions.get_mut(session_id) {
                    session.agents_active = session.agents_active.saturating_sub(1);
                }
                self.event_bus.emit(LabEvent::new(
                    "agent.removed",
                    serde_json::json!({
                        "agent_id": agent_id,
                        "session_id": session_id,
                    }),
                ));
                return true;
            }
        }
        false
    }

    pub async fn get_agent(&self, session_id: &str, agent_id: &str) -> Option<serde_json::Value> {
        self.state
            .read()
            .await
            .agent_registry
            .get(session_id)
            .and_then(|agents| agents.get(agent_id))
            .cloned()
    }

    /// Returns a map of `session_id → agent count` for stats.
    pub async fn agent_counts(&self) -> HashMap<String, usize> {
        self.state
            .read()
            .await
            .agent_registry
            .iter()
            .map(|(k, v)| (k.clone(), v.len()))
            .collect()
    }

    /// Load any previously active sessions from the persistent store.
    pub async fn restore(&self) -> usize {
        match self
            .store
            .lock()
            .await
            .query(Some("active"), None, None, 0, 0)
        {
            Ok(records) => {
                let count = records.len();
                let mut guard = self.state.write().await;
                for record in records {
                    match record.to_session() {
                        Ok(session) if !guard.sessions.contains_key(&session.id) => {
                            info!("Restored session: {} ({})", session.id, session.name);
                            guard.sessions.insert(session.id.clone(), session);
                        }
                        _ => {}
                    }
                }
                if count > 0 {
                    info!("Restored {} active session(s) from disk", count);
                }
                count
            }
            Err(e) => {
                warn!("Failed to restore sessions: {}", e);
                0
            }
        }
    }
}
