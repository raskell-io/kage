//! Daemon event bus for real-time UI streaming

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::broadcast;

use super::protocol::DaemonEvent;

/// Unique subscriber ID
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriberId(u64);

impl std::fmt::Display for SubscriberId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sub-{}", self.0)
    }
}

/// Real-time event bus for daemon events
///
/// Enables UI clients to subscribe to events and receive them as they happen,
/// instead of polling every few seconds.
#[derive(Clone)]
pub struct EventBus {
    /// Broadcast channel for events
    tx: broadcast::Sender<DaemonEvent>,

    /// Counter for subscriber IDs
    next_id: Arc<AtomicU64>,
}

impl EventBus {
    /// Create a new event bus
    pub fn new() -> Self {
        // Buffer up to 1000 events for slow receivers
        let (tx, _) = broadcast::channel(1000);
        Self {
            tx,
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Subscribe to events
    ///
    /// Returns a subscriber ID and a receiver channel.
    pub fn subscribe(&self) -> (SubscriberId, broadcast::Receiver<DaemonEvent>) {
        let id = SubscriberId(self.next_id.fetch_add(1, Ordering::SeqCst));
        let rx = self.tx.subscribe();
        (id, rx)
    }

    /// Publish an event to all subscribers
    pub fn publish(&self, event: DaemonEvent) {
        // Ignore send errors (no subscribers)
        let _ = self.tx.send(event);
    }

    /// Get the number of active subscribers
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentId;

    #[tokio::test]
    async fn test_event_bus_subscribe() {
        let bus = EventBus::new();

        let (id1, _rx1) = bus.subscribe();
        let (id2, _rx2) = bus.subscribe();

        assert_ne!(id1, id2);
        assert_eq!(bus.subscriber_count(), 2);
    }

    #[tokio::test]
    async fn test_event_bus_publish() {
        let bus = EventBus::new();
        let (_id, mut rx) = bus.subscribe();

        let agent_id = AgentId::new();
        bus.publish(DaemonEvent::AgentStatusChanged {
            id: agent_id,
            old_status: "running".into(),
            new_status: "paused".into(),
        });

        let event = rx.recv().await.unwrap();
        match event {
            DaemonEvent::AgentStatusChanged { id, new_status, .. } => {
                assert_eq!(id, agent_id);
                assert_eq!(new_status, "paused");
            }
            _ => panic!("Wrong event type"),
        }
    }
}
