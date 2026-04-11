//! MemoryService — wraps `MemoryWorkspace` behind a `std::sync::RwLock`.
//!
//! All `MemoryWorkspace` operations are synchronous (no internal I/O await),
//! so a `std::sync::RwLock` is correct here: reads are concurrent, writes
//! are exclusive. Using `std` rather than `tokio` avoids blocking the async
//! runtime on lock acquisition, which is fine for these brief operations.

use lab_memory::{MemoryEntry, MemoryWorkspace};
use std::sync::RwLock;

pub struct MemoryService {
    workspace: RwLock<MemoryWorkspace>,
}

impl MemoryService {
    pub fn new(workspace: MemoryWorkspace) -> Self {
        Self {
            workspace: RwLock::new(workspace),
        }
    }

    pub fn store(
        &self,
        session_id: &str,
        key: &str,
        value: &serde_json::Value,
        tags: Option<Vec<String>>,
    ) -> MemoryEntry {
        self.workspace
            .write()
            .unwrap()
            .store(session_id, key, value, tags)
    }

    pub fn get(&self, session_id: &str, key: &str) -> Option<serde_json::Value> {
        self.workspace.read().unwrap().get(session_id, key)
    }

    pub fn list_keys(&self, session_id: &str, tags: Option<&[String]>) -> Vec<String> {
        self.workspace.read().unwrap().list_keys(session_id, tags)
    }

    pub fn search(
        &self,
        session_id: &str,
        query: &str,
        tags: Option<&[String]>,
        limit: usize,
    ) -> Vec<serde_json::Value> {
        self.workspace
            .read()
            .unwrap()
            .search(session_id, query, tags, limit)
    }

    pub fn entry_count(&self) -> usize {
        self.workspace.read().unwrap().entry_count()
    }

    pub fn sessions(&self) -> Vec<String> {
        self.workspace.read().unwrap().sessions()
    }

    pub fn keys_for_session(&self, session_id: &str) -> Vec<String> {
        self.workspace.read().unwrap().keys_for_session(session_id)
    }
}
