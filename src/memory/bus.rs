//! Context bus - real-time pub/sub for memory events

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};

use crate::agent::AgentId;

use super::entry::{MemoryEntry, MemoryScope};

/// Real-time context bus for memory events
pub struct ContextBus {
    /// Broadcast channel for all events
    tx: broadcast::Sender<(MemoryEntry, MemoryScope)>,

    /// Per-agent subscriptions (scope filters)
    subscriptions: Arc<RwLock<HashMap<AgentId, Vec<ScopeFilter>>>>,
}

/// Filter for subscriptions
#[derive(Debug, Clone)]
pub enum ScopeFilter {
    /// Subscribe to all events in a namespace
    Namespace(String),

    /// Subscribe to global events
    Global,

    /// Subscribe to specific agent's events
    Agent(AgentId),
}

impl ContextBus {
    /// Create a new context bus
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1000);
        Self {
            tx,
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Subscribe an agent to receive events
    pub async fn subscribe(&self, agent: AgentId, filters: Vec<ScopeFilter>) {
        let mut subs = self.subscriptions.write().await;
        subs.insert(agent, filters);
    }

    /// Unsubscribe an agent
    pub async fn unsubscribe(&self, agent: AgentId) {
        let mut subs = self.subscriptions.write().await;
        subs.remove(&agent);
    }

    /// Publish a memory event
    pub async fn publish(&self, entry: MemoryEntry, scope: MemoryScope) {
        // Broadcast to all receivers
        let _ = self.tx.send((entry, scope));
    }

    /// Get a receiver for events
    pub fn receiver(&self) -> broadcast::Receiver<(MemoryEntry, MemoryScope)> {
        self.tx.subscribe()
    }

    /// Check if an agent should receive an event based on their filters
    pub async fn should_receive(&self, agent: AgentId, scope: &MemoryScope) -> bool {
        let subs = self.subscriptions.read().await;

        if let Some(filters) = subs.get(&agent) {
            for filter in filters {
                match (filter, scope) {
                    (ScopeFilter::Global, MemoryScope::Global) => return true,
                    (ScopeFilter::Namespace(f), MemoryScope::Namespace(s)) if f == s => {
                        return true
                    }
                    (ScopeFilter::Agent(f), MemoryScope::Agent(s)) if f == s => return true,
                    _ => {}
                }
            }
        }

        false
    }
}

impl Default for ContextBus {
    fn default() -> Self {
        Self::new()
    }
}
