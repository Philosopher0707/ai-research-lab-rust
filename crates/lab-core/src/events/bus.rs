//! Async pub/sub event bus backed by `tokio::sync::broadcast`.
//!
//! Key improvements over the previous callback-based design:
//! - `emit(&self)` — no exclusive lock on the lab needed to publish events.
//! - `subscribe()` returns a `broadcast::Receiver<LabEvent>` — callers drive
//!   their own async loop rather than registering synchronous closures.
//! - `EventBus` is `Clone + Send + Sync` so it can be wrapped in `Arc` and
//!   shared between the lab engine and the API state without a bridge channel.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

// ─── LabEvent ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: serde_json::Value,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub source: String,
    #[serde(default)]
    pub priority: EventPriority,
    #[serde(skip, default = "default_instant")]
    pub timestamp: std::time::Instant,
}

fn default_instant() -> std::time::Instant {
    std::time::Instant::now()
}

impl Default for LabEvent {
    fn default() -> Self {
        Self {
            event_type: String::new(),
            data: serde_json::Value::Null,
            source: String::new(),
            priority: EventPriority::Normal,
            timestamp: std::time::Instant::now(),
        }
    }
}

impl LabEvent {
    pub fn new(event_type: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            event_type: event_type.into(),
            data,
            source: String::new(),
            priority: EventPriority::Normal,
            timestamp: std::time::Instant::now(),
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    pub fn with_priority(mut self, priority: EventPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Returns the event category — the part before the first dot (e.g. `"tool"`).
    pub fn category(&self) -> &str {
        self.event_type.split('.').next().unwrap_or("")
    }

    pub fn to_dict(&self) -> serde_json::Value {
        serde_json::json!({
            "type": self.event_type,
            "data": self.data,
            "source": self.source,
            "priority": format!("{:?}", self.priority).to_lowercase(),
        })
    }
}

// ─── EventPriority ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventPriority {
    Low,
    #[default]
    Normal,
    High,
    Critical,
}

// ─── EventBus ──────────────────────────────────────────────────────

/// Broadcast-based event bus.
///
/// Wrapping in `Arc<EventBus>` lets the lab engine and the API server share
/// the same bus without any bridge tasks or extra channels.
pub struct EventBus {
    tx: broadcast::Sender<LabEvent>,
    history: Arc<RwLock<VecDeque<LabEvent>>>,
    max_history: usize,
}

impl EventBus {
    /// Create a new bus.
    ///
    /// - `capacity`: broadcast channel buffer depth (lagged receivers get an error).
    /// - `max_history`: how many events to keep in the in-memory ring buffer.
    pub fn new(capacity: usize, max_history: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            tx,
            history: Arc::new(RwLock::new(VecDeque::with_capacity(max_history.min(4096)))),
            max_history,
        }
    }

    /// Publish an event. Never blocks; `&self` only — no exclusive lock needed.
    ///
    /// If no subscribers are active the event is still recorded in history.
    pub fn emit(&self, event: LabEvent) {
        if let Ok(mut hist) = self.history.write() {
            hist.push_back(event.clone());
            while hist.len() > self.max_history {
                hist.pop_front();
            }
        }
        // Sending to zero receivers is not an error; the result is just dropped.
        let _ = self.tx.send(event);
    }

    /// Subscribe to all events. Returns a `broadcast::Receiver<LabEvent>`.
    ///
    /// The receiver will get every event emitted after this call. Lagged
    /// receivers get `RecvError::Lagged` — callers should handle that and continue.
    pub fn subscribe(&self) -> broadcast::Receiver<LabEvent> {
        self.tx.subscribe()
    }

    /// Get cached event history, optionally filtered by pattern, newest-first.
    ///
    /// Pattern syntax mirrors the original bus:
    /// - `"*"` — all events
    /// - `"agent.*"` — all events in the `agent` category
    /// - `"tool.executed"` — exact type match
    pub fn get_history(&self, filter: Option<&str>, limit: usize) -> Vec<LabEvent> {
        let hist = match self.history.read() {
            Ok(h) => h,
            Err(e) => e.into_inner(),
        };
        let effective_limit = if limit == 0 { self.max_history } else { limit };
        if let Some(f) = filter {
            hist.iter()
                .rev()
                .filter(|e| Self::matches(f, &e.event_type))
                .take(effective_limit)
                .cloned()
                .collect()
        } else {
            hist.iter().rev().take(effective_limit).cloned().collect()
        }
    }

    /// Convenience: return history as a JSON array suitable for API responses.
    pub fn get_history_json(&self, filter: Option<&str>, limit: usize) -> serde_json::Value {
        let events: Vec<_> = self
            .get_history(filter, limit)
            .into_iter()
            .map(|e| e.to_dict())
            .collect();
        serde_json::Value::Array(events)
    }

    // ─── Pattern matching ──────────────────────────────────────────

    /// Returns true if `pattern` matches `event_type`.
    ///
    /// Rules:
    /// - `"*"` matches everything.
    /// - `"agent.*"` matches any event whose first segment is `"agent"`.
    /// - Exact strings match only themselves.
    pub fn matches(pattern: &str, event_type: &str) -> bool {
        if pattern == "*" {
            return true;
        }
        let pat: Vec<&str> = pattern.split('.').collect();
        let evt: Vec<&str> = event_type.split('.').collect();

        // Trailing wildcard: "agent.*" matches "agent.started"
        if pat.len() == 2 && pat[1] == "*" {
            return pat[0] == evt.first().copied().unwrap_or("");
        }

        if pat.len() != evt.len() {
            return false;
        }

        pat.iter().zip(evt.iter()).all(|(p, e)| *p == "*" || p == e)
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(512, 1000)
    }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_matching() {
        assert!(EventBus::matches("*", "anything.here"));
        assert!(EventBus::matches("agent.*", "agent.started"));
        assert!(EventBus::matches("tool.executed", "tool.executed"));
        assert!(!EventBus::matches("agent.*", "tool.executed"));
        assert!(!EventBus::matches("agent.started", "agent.paused"));
    }

    #[test]
    fn emit_records_history() {
        let bus = EventBus::new(16, 100);
        bus.emit(LabEvent::new("test.event", serde_json::json!({"key": "val"})));
        let history = bus.get_history(None, 10);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].event_type, "test.event");
    }

    #[test]
    fn history_capped_at_max() {
        let bus = EventBus::new(16, 3);
        for i in 0..5u32 {
            bus.emit(LabEvent::new(format!("ev.{}", i), serde_json::json!({})));
        }
        assert_eq!(bus.get_history(None, 100).len(), 3);
    }

    #[test]
    fn history_filtered() {
        let bus = EventBus::new(16, 100);
        bus.emit(LabEvent::new("tool.executed", serde_json::json!({})));
        bus.emit(LabEvent::new("agent.started", serde_json::json!({})));
        let filtered = bus.get_history(Some("tool.*"), 10);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].event_type, "tool.executed");
    }

    #[tokio::test]
    async fn subscribe_receives_emitted_events() {
        let bus = EventBus::new(16, 100);
        let mut rx = bus.subscribe();
        bus.emit(LabEvent::new("session.created", serde_json::json!({})));
        let event = rx.recv().await.expect("should receive event");
        assert_eq!(event.event_type, "session.created");
    }

    #[test]
    fn emit_no_receiver_does_not_panic() {
        let bus = EventBus::new(16, 100);
        // No subscriber — should not panic
        bus.emit(LabEvent::new("orphan.event", serde_json::json!({})));
    }

    #[test]
    fn category() {
        let event = LabEvent::new("tool.executed", serde_json::json!({}));
        assert_eq!(event.category(), "tool");
    }
}
