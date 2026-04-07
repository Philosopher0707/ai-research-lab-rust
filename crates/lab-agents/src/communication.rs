//! Inter-agent pub/sub messaging.
//! Mirrors agents/communication.py (247 lines)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::warn;

// ─── Message Types ──────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Task,
    Result,
    Query,
    Response,
    Notify,
    Error,
}

fn default_instant() -> Instant { Instant::now() }

// ─── AgentMessage ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub msg_id: String,
    pub sender_id: String,
    pub sender_role: String,
    pub recipient_id: String, // empty = broadcast
    #[serde(rename = "type")]
    pub msg_type: MessageType,
    pub payload: HashMap<String, serde_json::Value>,
    pub reply_to: String,
    #[serde(skip, default = "default_instant")]
    pub timestamp: Instant,
    pub ttl_secs: u64,
}

impl AgentMessage {
    pub fn new(
        sender_id: impl Into<String>,
        sender_role: impl Into<String>,
        recipient_id: impl Into<String>,
        msg_type: MessageType,
        payload: HashMap<String, serde_json::Value>,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        Self {
            msg_id: id,
            sender_id: sender_id.into(),
            sender_role: sender_role.into(),
            recipient_id: recipient_id.into(),
            msg_type,
            payload,
            reply_to: String::new(),
            timestamp: Instant::now(),
            ttl_secs: 60,
        }
    }

    /// Create a reply to this message.
    pub fn reply(&self, payload: HashMap<String, serde_json::Value>) -> Self {
        let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        Self {
            msg_id: id,
            sender_id: self.recipient_id.clone(),
            recipient_id: self.sender_id.clone(),
            msg_type: MessageType::Response,
            payload,
            reply_to: self.msg_id.clone(),
            sender_role: String::new(),
            timestamp: Instant::now(),
            ttl_secs: 60,
        }
    }

    pub fn is_expired(&self) -> bool {
        self.timestamp.elapsed().as_secs() > self.ttl_secs
    }
}

impl Default for AgentMessage {
    fn default() -> Self {
        Self::new("", "", "", MessageType::Notify, HashMap::new())
    }
}

// ─── AgentMailbox ───────────────────────────────────────────

/// Per-agent async message inbox.
#[derive(Clone)]
pub struct AgentMailbox {
    pub agent_id: String,
    tx: mpsc::Sender<AgentMessage>,
    rx: std::sync::Arc<tokio::sync::Mutex<mpsc::Receiver<AgentMessage>>>,
    history: std::sync::Arc<tokio::sync::Mutex<Vec<AgentMessage>>>,
    max_history: usize,
}

impl AgentMailbox {
    pub fn new(agent_id: impl Into<String>, capacity: usize, max_history: usize) -> Self {
        let (tx, rx) = mpsc::channel(capacity);
        Self {
            agent_id: agent_id.into(),
            tx,
            rx: std::sync::Arc::new(tokio::sync::Mutex::new(rx)),
            history: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
            max_history,
        }
    }

    pub async fn send(&self, msg: AgentMessage) -> Result<(), mpsc::error::SendError<AgentMessage>> {
        self.tx.send(msg.clone()).await?;
        let mut hist = self.history.lock().await;
        hist.push(msg);
        if hist.len() > self.max_history {
            let remove = hist.len() - self.max_history;
            hist.drain(..remove);
        }
        Ok(())
    }

    pub async fn receive(&self, timeout_secs: u64) -> Option<AgentMessage> {
        let mut rx = self.rx.lock().await;
        match tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            rx.recv(),
        )
        .await
        {
            Ok(Some(msg)) if !msg.is_expired() => Some(msg),
            Ok(_) => None,
            Err(_) => None, // timeout
        }
    }

    pub async fn history(&self, limit: usize) -> Vec<serde_json::Value> {
        let hist = self.history.lock().await;
        hist.iter()
            .rev()
            .take(limit)
            .map(|m| serde_json::json!({
                "msg_id": m.msg_id,
                "sender_id": m.sender_id,
                "sender_role": m.sender_role,
                "type": format!("{:?}", m.msg_type).to_lowercase(),
                "reply_to": m.reply_to,
            }))
            .collect()
    }
}

// ─── AgentCommunicator ─────────────────────────────────────

/// Session-scoped inter-agent communication bus.
pub struct AgentCommunicator {
    session_id: String,
    mailboxes: HashMap<String, AgentMailbox>,
}

impl AgentCommunicator {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            mailboxes: HashMap::new(),
        }
    }

    /// Register an agent's mailbox.
    pub fn register(&mut self, mailbox: AgentMailbox) {
        self.mailboxes.insert(mailbox.agent_id.clone(), mailbox);
    }

    /// Create and register a mailbox for an agent.
    pub fn create_mailbox(
        &mut self,
        agent_id: String,
        capacity: usize,
        max_history: usize,
    ) {
        let mailbox = AgentMailbox::new(&agent_id, capacity, max_history);
        self.mailboxes.insert(agent_id, mailbox);
    }

    /// Send a message to a specific agent.
    pub async fn send_to(
        &self,
        recipient_id: &str,
        msg: AgentMessage,
    ) -> Result<(), String> {
        if let Some(mailbox) = self.mailboxes.get(recipient_id) {
            mailbox.send(msg).await.map_err(|e| e.to_string())
        } else {
            Err(format!("No mailbox for agent: {recipient_id}"))
        }
    }

    /// Broadcast a message to all agents in the session.
    pub async fn broadcast(&self, msg: AgentMessage) {
        for (id, mailbox) in &self.mailboxes {
            if *id != msg.sender_id {
                // Clone for each recipient
                let mut copy = msg.clone();
                copy.recipient_id = id.clone();
                if let Err(e) = mailbox.send(copy).await {
                    warn!("Failed to send broadcast to {id}: {e}");
                }
            }
        }
    }

    pub fn mailbox(&self, agent_id: &str) -> Option<&AgentMailbox> {
        self.mailboxes.get(agent_id)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn agent_count(&self) -> usize {
        self.mailboxes.len()
    }
}
