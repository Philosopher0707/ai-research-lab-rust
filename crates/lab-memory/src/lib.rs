//! Lab Memory — Persistent knowledge store with TF-IDF vector search,
//! session/project/global scopes, TTL-based expiration, and filesystem persistence.
//! Mirrors core/memory/workspace.py (554 lines)

use chrono::{DateTime, Utc};
use md5::Md5;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::debug;

// ─── Stop words ───────────────────────────────────────────────────

const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "shall", "should", "may", "might", "must", "can",
    "could", "it", "its", "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into",
    "through", "during", "before", "after", "and", "but", "or", "nor", "not", "so", "yet", "both",
    "either", "neither", "each", "every", "all", "any", "few", "more", "most", "other", "some",
    "such", "no", "only", "own", "same", "than", "too", "very", "just", "if", "because", "until",
    "while", "about", "between", "under", "again", "there", "then", "once", "here", "where",
    "when", "what", "which", "who", "whom", "that", "this", "these", "those", "me", "my", "we",
    "our", "you", "your", "he", "him", "his", "she", "her", "they", "them", "their", "also", "up",
    "out", "over",
];

const DIMS: usize = 256;

// ─── MemoryEntry ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub key: String,
    pub value: serde_json::Value,
    #[serde(default = "default_scope")]
    pub scope: MemoryScope,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub ttl: Option<u64>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub vector: Option<Vec<f32>>,
}

fn default_scope() -> MemoryScope {
    MemoryScope::Session
}

impl MemoryEntry {
    pub fn is_expired(&self) -> bool {
        if let Some(ttl) = self.ttl {
            let age = (Utc::now() - self.created_at).num_seconds() as u64;
            return age > ttl;
        }
        false
    }
}

// ─── MemoryScope ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    Session,
    Project,
    Global,
}

// ─── MemoryWorkspace ──────────────────────────────────────────────

pub struct MemoryWorkspace {
    pub workspace: PathBuf,
    stores: HashMap<String, HashMap<String, MemoryEntry>>,
    global_store: HashMap<String, MemoryEntry>,
    max_entries: usize,
    default_ttl: u64,
    vocabulary_dirty: bool,
}

impl MemoryWorkspace {
    pub fn new(workspace: PathBuf) -> Self {
        let mut ws = Self {
            workspace,
            stores: HashMap::new(),
            global_store: HashMap::new(),
            max_entries: 10000,
            default_ttl: 86400 * 7,
            vocabulary_dirty: true,
        };
        let _ = std::fs::create_dir_all(&ws.workspace);
        ws.load_from_disk();
        ws
    }

    pub fn with_max_entries(mut self, max: usize) -> Self {
        self.max_entries = max;
        self
    }

    pub fn with_default_ttl(mut self, ttl: u64) -> Self {
        self.default_ttl = ttl;
        self
    }

    // ─── Core Operations ─────────────────────────────────────

    /// Store a value in memory.
    pub fn store(
        &mut self,
        session_id: &str,
        key: &str,
        value: &serde_json::Value,
        tags: Option<Vec<String>>,
    ) -> MemoryEntry {
        let vec = value.as_str().map(|text| Self::text_to_vector(text, DIMS));

        let entry = MemoryEntry {
            key: key.to_string(),
            value: value.clone(),
            scope: MemoryScope::Session,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            ttl: Some(self.default_ttl),
            tags: tags.unwrap_or_default(),
            metadata: HashMap::new(),
            source: String::new(),
            vector: vec,
        };

        // Get store mutably, insert, then drop the borrow before calling persist
        let entry_id = key.to_string();
        let sid = session_id.to_string();
        if let Some(store) = self.stores.get_mut(&sid) {
            store.insert(entry_id.clone(), entry.clone());
        } else {
            let mut new_store = HashMap::new();
            new_store.insert(entry_id.clone(), entry.clone());
            self.stores.insert(sid.clone(), new_store);
        }
        self.vocabulary_dirty = true;

        let _ = self.persist_entry(session_id, key, &entry);
        entry
    }

    /// Retrieve a value from memory.
    pub fn get(&self, session_id: &str, key: &str) -> Option<serde_json::Value> {
        self.find_key(key, Some(session_id), MemoryScope::Session, true, true)
            .filter(|e| !e.is_expired())
            .map(|e| e.value.clone())
    }

    /// Delete a memory entry.
    pub fn delete(&mut self, session_id: &str, key: &str) -> bool {
        let mut removed = false;
        if let Some(store) = self.stores.get_mut(session_id) {
            if store.remove(key).is_some() {
                removed = true;
            }
        }
        if self.global_store.remove(key).is_some() {
            removed = true;
        }
        removed
    }

    // ─── Search ────────────────────────────────────────────────

    /// Search memory by keyword overlap, tags, and vector similarity.
    pub fn search(
        &self,
        session_id: &str,
        query: &str,
        tags: Option<&[String]>,
        limit: usize,
    ) -> Vec<serde_json::Value> {
        let session_store = self.stores.get(session_id);
        let mut results = Vec::new();

        let mut all_entries: Vec<(&String, &MemoryEntry)> = session_store
            .map(|s| s.iter().collect())
            .unwrap_or_default();
        for (k, v) in &self.global_store {
            all_entries.push((k, v));
        }

        for (key, entry) in all_entries {
            if entry.is_expired() {
                continue;
            }
            let mut score = 0.0f32;

            // Tag matching
            if let Some(t) = tags {
                let tag_score =
                    t.iter().filter(|tag| entry.tags.contains(tag)).count() as f32 * 2.0;
                score += tag_score;
            }

            // Keyword overlap
            if !query.is_empty() {
                let overlap = Self::keyword_score(query, &entry.value);
                score += overlap * 1.5;
            }

            // Vector similarity
            if let Some(ref vec) = entry.vector {
                let query_vec = Self::text_to_vector(query, DIMS);
                let sim = Self::vector_cosine(&query_vec, vec);
                score += sim * 0.8;
            }

            if score > 0.0 {
                results.push(serde_json::json!({
                    "key": key,
                    "value": entry.value,
                    "score": (score * 1000.0).round() / 1000.0,
                    "tags": entry.tags,
                }));
            }
        }

        results.sort_by(|a, b| {
            let sa = a.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let sb = b.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        results
    }

    /// Pure vector-based similarity search.
    pub fn similarity_search(
        &self,
        session_id: &str,
        query: &str,
        limit: usize,
        threshold: f32,
    ) -> Vec<serde_json::Value> {
        let query_vector = Self::text_to_vector(query, DIMS);
        if query_vector.iter().all(|&v| v == 0.0) {
            return Vec::new();
        }

        let session_store = self.stores.get(session_id);
        let mut scored = Vec::new();

        let mut all_entries: Vec<&MemoryEntry> = session_store
            .map(|s| s.values().collect())
            .unwrap_or_default();
        for v in self.global_store.values() {
            all_entries.push(v);
        }

        for entry in all_entries {
            if entry.is_expired() {
                continue;
            }
            if let Some(ref vec) = entry.vector {
                let sim = Self::vector_cosine(&query_vector, vec);
                if sim >= threshold {
                    scored.push(serde_json::json!({
                        "key": entry.key,
                        "value": entry.value,
                        "similarity": (sim * 10000.0).round() / 10000.0,
                        "tags": entry.tags,
                    }));
                }
            }
        }

        scored.sort_by(|a, b| {
            let sa = a.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let sb = b.get("similarity").and_then(|v| v.as_f64()).unwrap_or(0.0);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        scored
    }

    /// List all memory keys for a session.
    pub fn list_keys(&self, session_id: &str, tags: Option<&[String]>) -> Vec<String> {
        let session_store = self.stores.get(session_id);
        let mut keys = Vec::new();

        let mut all_entries: Vec<(&String, &MemoryEntry)> = session_store
            .map(|s| s.iter().collect())
            .unwrap_or_default();
        for (k, v) in &self.global_store {
            all_entries.push((k, v));
        }

        for (key, entry) in all_entries {
            if entry.is_expired() {
                continue;
            }
            if let Some(t) = tags {
                if t.iter().any(|tag| entry.tags.contains(tag)) {
                    keys.push(key.clone());
                }
            } else {
                keys.push(key.clone());
            }
        }
        keys.sort();
        keys
    }

    /// Get total entry count.
    pub fn entry_count(&self) -> usize {
        let store_count: usize = self.stores.values().map(|s| s.len()).sum();
        store_count + self.global_store.len()
    }

    /// List all session IDs that have stored entries.
    pub fn sessions(&self) -> Vec<String> {
        self.stores.keys().cloned().collect()
    }

    /// List all (non-expired) keys stored under a given session.
    pub fn keys_for_session(&self, session_id: &str) -> Vec<String> {
        self.stores
            .get(session_id)
            .map(|store| {
                store
                    .iter()
                    .filter(|(_, e)| !e.is_expired())
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List all (non-expired) entries for a session as (key, value, tags) tuples.
    pub fn entries_for_session(
        &self,
        session_id: &str,
    ) -> Vec<(&str, &serde_json::Value, &[String])> {
        self.stores
            .get(session_id)
            .map(|store| {
                store
                    .values()
                    .filter(|e| !e.is_expired())
                    .map(|e| (e.key.as_str(), &e.value, e.tags.as_slice()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Clear memory entries.
    pub fn clear(&mut self, session_id: Option<&str>, scope: MemoryScope) {
        match scope {
            MemoryScope::Session => {
                if let Some(sid) = session_id {
                    self.stores.remove(sid);
                }
            }
            MemoryScope::Global => {
                self.global_store.clear();
            }
            MemoryScope::Project => {
                self.stores.remove("_project_memory");
            }
        }
    }

    // ─── Persistence ──────────────────────────────────────────

    fn load_from_disk(&mut self) {
        if let Ok(entries) = std::fs::read_dir(&self.workspace) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_dir() {
                    let session_id = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();

                    let memory_file = path.join("memory.json");
                    if memory_file.exists() {
                        if let Ok(content) = std::fs::read_to_string(&memory_file) {
                            if let Ok(data) =
                                serde_json::from_str::<HashMap<String, MemoryEntry>>(&content)
                            {
                                self.stores.insert(session_id, data);
                                debug!("Loaded session memory: {}", entry.path().display());
                            }
                        }
                    }
                }
            }
        }
    }

    fn persist_entry(
        &self,
        session_id: &str,
        key: &str,
        entry: &MemoryEntry,
    ) -> std::io::Result<()> {
        let session_dir = self.workspace.join(session_id);
        std::fs::create_dir_all(&session_dir)?;

        let index_file = session_dir.join("index.json");
        let mut index: HashMap<String, serde_json::Value> = if index_file.exists() {
            let content = std::fs::read_to_string(&index_file)?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            HashMap::new()
        };

        index.insert(
            key.to_string(),
            serde_json::json!({
                "updated": entry.updated_at.to_rfc3339(),
                "scope": format!("{:?}", entry.scope).to_lowercase(),
            }),
        );

        std::fs::write(&index_file, serde_json::to_string_pretty(&index)?)
    }

    // ─── Internal Helpers ─────────────────────────────────────

    fn find_key(
        &self,
        key: &str,
        session_id: Option<&str>,
        scope: MemoryScope,
        fallback_project: bool,
        fallback_global: bool,
    ) -> Option<&MemoryEntry> {
        match scope {
            MemoryScope::Global => self.global_store.get(key),
            MemoryScope::Project => self.stores.get("_project_memory").and_then(|s| s.get(key)),
            MemoryScope::Session => {
                if let Some(sid) = session_id {
                    if let Some(store) = self.stores.get(sid) {
                        if let Some(entry) = store.get(key) {
                            return Some(entry);
                        }
                    }
                }
                if fallback_project {
                    if let Some(entry) = self.stores.get("_project_memory").and_then(|s| s.get(key))
                    {
                        return Some(entry);
                    }
                }
                if fallback_global {
                    return self.global_store.get(key);
                }
                None
            }
        }
    }

    // ─── Vector Embedding & Similarity ────────────────────────

    fn tokenize(text: &str) -> Vec<String> {
        let re = Regex::new(r"[a-z_][a-z0-9_]{1,}").unwrap();
        re.find_iter(&text.to_lowercase())
            .map(|m| m.as_str().to_string())
            .filter(|w| !STOP_WORDS.contains(&w.as_str()) && w.len() > 2)
            .collect()
    }

    fn word_hash(word: &str, dims: usize) -> usize {
        let mut hasher = Md5::new();
        hasher.update(word.as_bytes());
        let result = hasher.finalize();
        let hex = format!("{:x}", result);
        u32::from_str_radix(&hex[..8], 16).unwrap_or(0) as usize % dims
    }

    fn text_to_vector(text: &str, dims: usize) -> Vec<f32> {
        let tokens = Self::tokenize(text);
        if tokens.is_empty() {
            return vec![0.0; dims];
        }

        // TF: term frequency
        let mut tf: HashMap<&str, f32> = HashMap::new();
        for token in &tokens {
            *tf.entry(token.as_str()).or_insert(0.0) += 1.0;
        }

        // Build vector using feature hashing
        let mut vector = vec![0.0f32; dims];
        for (word, count) in &tf {
            let bucket = Self::word_hash(word, dims);
            let total_tokens = tf.len() as f32;
            let idf = 1.0 + (total_tokens.max(1.0) / count.max(1.0)).ln();
            vector[bucket] += count * idf;
        }

        // L2 normalize
        let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut vector {
                *v /= norm;
            }
        }
        vector
    }

    fn keyword_score(query: &str, value: &serde_json::Value) -> f32 {
        let text = match value {
            serde_json::Value::String(s) => s.clone(),
            _ => value.to_string(),
        };
        let query_tokens = Self::tokenize(query);
        let text_lower = text.to_lowercase();
        let mut score = 0f32;
        for token in &query_tokens {
            if text_lower.contains(token.as_str()) {
                score += 1.0;
            }
        }
        if query_tokens.is_empty() {
            0.0
        } else {
            score / query_tokens.len() as f32
        }
    }

    fn vector_cosine(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() {
            let max_len = a.len().max(b.len());
            let mut a_padded = vec![0.0; max_len];
            let mut b_padded = vec![0.0; max_len];
            a_padded[..a.len()].copy_from_slice(a);
            b_padded[..b.len()].copy_from_slice(b);
            return Self::vector_cosine(&a_padded, &b_padded);
        }
        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        let denom = norm_a * norm_b;
        if denom > 0.0 {
            dot / denom
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_and_get() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut ws = MemoryWorkspace::new(temp_dir.path().to_path_buf());
        ws.store("s1", "key1", &serde_json::json!("hello world"), None);
        let val = ws.get("s1", "key1");
        assert!(val.is_some());
        assert_eq!(val.unwrap(), serde_json::json!("hello world"));
    }

    #[test]
    fn search_by_tags() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut ws = MemoryWorkspace::new(temp_dir.path().to_path_buf());
        ws.store(
            "s1",
            "k1",
            &serde_json::json!("research findings"),
            Some(vec!["research".to_string()]),
        );
        ws.store("s1", "k2", &serde_json::json!("other data"), None);

        let results = ws.search("s1", "research", Some(&["research".to_string()]), 10);
        assert!(!results.is_empty());
        assert_eq!(results[0]["key"], "k1");
    }

    #[test]
    fn vector_search_basic() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut ws = MemoryWorkspace::new(temp_dir.path().to_path_buf());
        ws.store(
            "s1",
            "k1",
            &serde_json::json!("rust async programming"),
            None,
        );
        ws.store(
            "s1",
            "k2",
            &serde_json::json!("python synchronous code"),
            None,
        );

        let results = ws.similarity_search("s1", "async rust code", 5, 0.0);
        assert!(!results.is_empty());
        // The rust/async entry should score highest
        assert_eq!(results[0]["key"], "k1");
    }

    #[test]
    fn delete_entry() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut ws = MemoryWorkspace::new(temp_dir.path().to_path_buf());
        ws.store("s1", "k1", &serde_json::json!("test"), None);
        assert!(ws.delete("s1", "k1"));
        assert!(ws.get("s1", "k1").is_none());
    }

    #[test]
    fn text_to_vector_nonempty() {
        let vec = MemoryWorkspace::text_to_vector("async rust tokio runtime", DIMS);
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01); // L2 normalized
    }

    #[test]
    fn vector_cosine_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((MemoryWorkspace::vector_cosine(&a, &b) - 1.0).abs() < 0.001);
    }
}
