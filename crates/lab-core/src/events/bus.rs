//! Async pub/sub event bus with pattern matching.
//! Mirrors core/events/bus.py (200 lines)

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

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

    /// Get the event category (first part of dot-joined type).
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

// ─── Subscriber ─────────────────────────────────────────────────────

type EventHandler = Arc<dyn Fn(LabEvent) + Send + Sync>;

struct Subscriber {
    handler: EventHandler,
    match_once: bool,
    priority: i32,
}

// ─── EventBus ──────────────────────────────────────────────────────

pub struct EventBus {
    // pattern -> subscribers
    subscribers: HashMap<String, Vec<Subscriber>>,
    history: VecDeque<LabEvent>,
    max_history: usize,
}

impl EventBus {
    pub fn new(max_history: usize) -> Self {
        Self {
            subscribers: HashMap::new(),
            history: VecDeque::with_capacity(max_history),
            max_history,
        }
    }

    /// Subscribe to events matching a pattern.
    pub fn subscribe(
        &mut self,
        event_pattern: &str,
        handler: impl Fn(LabEvent) + Send + Sync + 'static,
    ) {
        self.subscribe_with_priority(event_pattern, handler, 0);
    }

    /// Subscribe with priority.
    pub fn subscribe_with_priority(
        &mut self,
        event_pattern: &str,
        handler: impl Fn(LabEvent) + Send + Sync + 'static,
        priority: i32,
    ) {
        let pat = if event_pattern.is_empty() {
            "*".to_string()
        } else {
            event_pattern.to_string()
        };
        self.subscribers.entry(pat).or_default().push(Subscriber {
            handler: Arc::new(handler),
            match_once: false,
            priority,
        });
    }

    /// Subscribe once (auto-unsubscribe after first match).
    pub fn subscribe_once(
        &mut self,
        event_pattern: &str,
        handler: impl Fn(LabEvent) + Send + Sync + 'static,
    ) {
        let pat = event_pattern.to_string();
        self.subscribers.entry(pat).or_default().push(Subscriber {
            handler: Arc::new(handler),
            match_once: true,
            priority: 0,
        });
    }

    /// Subscribe to ALL events.
    pub fn subscribe_all(&mut self, handler: impl Fn(LabEvent) + Send + Sync + 'static) {
        self.subscribers
            .entry("*".to_string())
            .or_default()
            .push(Subscriber {
                handler: Arc::new(handler),
                match_once: false,
                priority: 0,
            });
    }

    /// Emit an event to all matching subscribers.
    pub fn emit(&mut self, event: LabEvent) {
        // Record in history
        self.history.push_back(event.clone());
        while self.history.len() > self.max_history {
            self.history.pop_front();
        }
        // Dispatch to subscribers
        self.dispatch(&event);
    }

    /// Dispatch to matching subscribers.
    fn dispatch(&mut self, event: &LabEvent) {
        let mut patterns_to_remove: Vec<(String, usize)> = Vec::new();

        for (pattern, handlers) in self.subscribers.iter_mut() {
            if Self::matches(pattern, &event.event_type) {
                for (i, sub) in handlers.iter().enumerate() {
                    (sub.handler)(event.clone());
                    if sub.match_once {
                        patterns_to_remove.push((pattern.clone(), i));
                    }
                }
            }
        }

        // Remove one-shot subscribers (reverse order to avoid index shift)
        for (pattern, idx) in patterns_to_remove.iter().rev() {
            if let Some(handlers) = self.subscribers.get_mut(pattern) {
                if *idx < handlers.len() {
                    handlers.remove(*idx);
                }
            }
        }
    }

    /// Check if a pattern matches an event type.
    fn matches(pattern: &str, event_type: &str) -> bool {
        if pattern == "*" {
            return true;
        }
        let pat_parts: Vec<&str> = pattern.split('.').collect();
        let evt_parts: Vec<&str> = event_type.split('.').collect();

        if pat_parts.len() != evt_parts.len() {
            // Allow trailing wildcard: "agent.*" matches "agent.started"
            if pat_parts.len() == 2 && pat_parts[1] == "*" && pat_parts[0] == evt_parts.get(0).copied().unwrap_or("") {
                return true;
            }
            if pat_parts.len() == 1 && pat_parts[0] == "*" {
                return true;
            }
            return false;
        }

        for (pat, evt) in pat_parts.iter().zip(evt_parts.iter()) {
            if *pat != "*" && *pat != *evt {
                return false;
            }
        }
        true
    }

    /// Get event history, optionally filtered.
    pub fn get_history(&self, filter: Option<&str>, limit: usize) -> Vec<LabEvent> {
        if let Some(f) = filter {
            self.history
                .iter()
                .filter(|e| Self::matches(f, &e.event_type))
                .cloned()
                .rev()
                .take(limit)
                .collect()
        } else {
            self.history.iter().rev().take(limit).cloned().collect()
        }
    }

    /// Get history as JSON values.
    pub fn get_history_json(&self, filter: Option<&str>, limit: usize) -> serde_json::Value {
        let events: Vec<_> = self.get_history(filter, limit)
            .into_iter()
            .map(|e| e.to_dict())
            .collect();
        serde_json::Value::Array(events)
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_matching() {
        assert!(EventBus::matches("*", "anything.here"));
        assert!(EventBus::matches("agent.*", "agent.started"));
        assert!(EventBus::matches("tool.executed", "tool.executed"));
        assert!(!EventBus::matches("agent.*", "tool.executed"));
        assert!(!EventBus::matches("agent.started", "agent.paused"));
    }

    #[test]
    fn test_emit_and_history() {
        let mut bus = EventBus::new(100);
        let event = LabEvent::new("test.event", serde_json::json!({"key": "val"}));
        bus.emit(event);
        let history = bus.get_history(None, 10);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].event_type, "test.event");
    }

    #[test]
    fn test_category() {
        let event = LabEvent::new("tool.executed", serde_json::json!({}));
        assert_eq!(event.category(), "tool");
    }
}
