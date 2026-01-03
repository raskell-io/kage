//! Subscription pool for intelligent routing and load balancing
//!
//! Manages the selection of subscriptions for agent requests based on
//! availability, rate limits, routing rules, and load balancing strategy.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result};

use super::registry::SubscriptionRegistry;
use super::{LoadBalanceStrategy, RoutingConfig, Subscription, SubscriptionId};
use crate::agent::AgentId;

/// Request context for subscription selection
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    /// Agent making the request
    pub agent_id: Option<AgentId>,
    /// Namespace of the agent/task
    pub namespace: Option<String>,
    /// Priority level (e.g., "high", "normal", "batch")
    pub priority: Option<String>,
    /// Required tags
    pub required_tags: Vec<String>,
    /// Preferred subscription (for sticky sessions)
    pub preferred_subscription: Option<SubscriptionId>,
}

impl RequestContext {
    /// Create a new request context
    pub fn new() -> Self {
        Self::default()
    }

    /// Set agent ID
    pub fn with_agent(mut self, agent_id: AgentId) -> Self {
        self.agent_id = Some(agent_id);
        self
    }

    /// Set namespace
    pub fn with_namespace(mut self, namespace: &str) -> Self {
        self.namespace = Some(namespace.to_string());
        self
    }

    /// Set priority
    pub fn with_priority(mut self, priority: &str) -> Self {
        self.priority = Some(priority.to_string());
        self
    }

    /// Add required tag
    pub fn with_tag(mut self, tag: &str) -> Self {
        self.required_tags.push(tag.to_string());
        self
    }

    /// Set preferred subscription
    pub fn with_preferred(mut self, id: SubscriptionId) -> Self {
        self.preferred_subscription = Some(id);
        self
    }
}

/// Subscription pool for routing and load balancing
pub struct SubscriptionPool {
    /// Subscription registry
    registry: Arc<SubscriptionRegistry>,
    /// Routing configuration
    routing_config: RwLock<RoutingConfig>,
    /// Round-robin counter
    round_robin_counter: AtomicUsize,
    /// Sticky session mappings (agent -> subscription)
    sticky_sessions: RwLock<HashMap<AgentId, SubscriptionId>>,
}

impl SubscriptionPool {
    /// Create a new subscription pool
    pub fn new(registry: Arc<SubscriptionRegistry>) -> Self {
        Self {
            registry,
            routing_config: RwLock::new(RoutingConfig::default()),
            round_robin_counter: AtomicUsize::new(0),
            sticky_sessions: RwLock::new(HashMap::new()),
        }
    }

    /// Set routing configuration
    pub fn set_routing_config(&self, config: RoutingConfig) {
        let mut cfg = self.routing_config.write().unwrap();
        *cfg = config;
    }

    /// Get routing configuration
    pub fn routing_config(&self) -> RoutingConfig {
        self.routing_config.read().unwrap().clone()
    }

    /// Acquire a subscription for a request
    pub fn acquire(&self, context: &RequestContext) -> Result<SubscriptionLease> {
        // Get candidates based on routing rules
        let candidates = self.get_candidates(context)?;

        if candidates.is_empty() {
            anyhow::bail!("No available subscriptions for request");
        }

        // Select based on strategy
        let config = self.routing_config.read().unwrap();
        let selected = self.select_subscription(&candidates, &config.strategy, context)?;

        // Update sticky session if enabled
        if config.sticky_sessions {
            if let Some(agent_id) = context.agent_id {
                let mut sessions = self.sticky_sessions.write().unwrap();
                sessions.insert(agent_id, selected.id);
            }
        }

        Ok(SubscriptionLease {
            subscription: selected,
            pool: self,
        })
    }

    /// Get candidate subscriptions based on routing rules
    fn get_candidates(&self, context: &RequestContext) -> Result<Vec<Subscription>> {
        let config = self.routing_config.read().unwrap();
        let all_available = self.registry.list_available();

        if all_available.is_empty() {
            return Ok(vec![]);
        }

        // Check sticky session first
        if config.sticky_sessions {
            if let Some(agent_id) = context.agent_id {
                let sessions = self.sticky_sessions.read().unwrap();
                if let Some(&sub_id) = sessions.get(&agent_id) {
                    if let Some(sub) = self.registry.get(sub_id) {
                        if sub.is_available() {
                            return Ok(vec![sub]);
                        }
                    }
                }
            }
        }

        // Check preferred subscription
        if let Some(preferred_id) = context.preferred_subscription {
            if let Some(sub) = self.registry.get(preferred_id) {
                if sub.is_available() {
                    return Ok(vec![sub]);
                }
            }
        }

        // Apply namespace routing
        if let Some(ref namespace) = context.namespace {
            if let Some(sub_names) = config.namespace_routes.get(namespace) {
                let routed: Vec<_> = all_available
                    .iter()
                    .filter(|s| sub_names.contains(&s.name))
                    .cloned()
                    .collect();
                if !routed.is_empty() {
                    return Ok(routed);
                }
            }
        }

        // Apply priority routing
        if let Some(ref priority) = context.priority {
            if let Some(sub_names) = config.priority_routes.get(priority) {
                let routed: Vec<_> = all_available
                    .iter()
                    .filter(|s| sub_names.contains(&s.name))
                    .cloned()
                    .collect();
                if !routed.is_empty() {
                    return Ok(routed);
                }
            }
        }

        // Apply tag filtering
        if !context.required_tags.is_empty() {
            let tagged: Vec<_> = all_available
                .iter()
                .filter(|s| context.required_tags.iter().all(|tag| s.tags.contains(tag)))
                .cloned()
                .collect();
            if !tagged.is_empty() {
                return Ok(tagged);
            }
        }

        // Use default subscriptions if configured
        if !config.default_subscriptions.is_empty() {
            let defaults: Vec<_> = all_available
                .iter()
                .filter(|s| config.default_subscriptions.contains(&s.name))
                .cloned()
                .collect();
            if !defaults.is_empty() {
                return Ok(defaults);
            }
        }

        // Return all available
        Ok(all_available)
    }

    /// Select a subscription based on load balancing strategy
    fn select_subscription(
        &self,
        candidates: &[Subscription],
        strategy: &LoadBalanceStrategy,
        _context: &RequestContext,
    ) -> Result<Subscription> {
        if candidates.is_empty() {
            anyhow::bail!("No candidate subscriptions available");
        }

        let selected = match strategy {
            LoadBalanceStrategy::RoundRobin => {
                let idx = self.round_robin_counter.fetch_add(1, Ordering::SeqCst);
                &candidates[idx % candidates.len()]
            }

            LoadBalanceStrategy::Priority => {
                // Candidates are already sorted by priority
                &candidates[0]
            }

            LoadBalanceStrategy::LeastRecentlyUsed => {
                candidates
                    .iter()
                    .min_by_key(|s| s.usage.last_used_at.unwrap_or(0))
                    .unwrap()
            }

            LoadBalanceStrategy::Random => {
                use std::collections::hash_map::RandomState;
                use std::hash::{BuildHasher, Hasher};
                let hasher = RandomState::new().build_hasher();
                let idx = hasher.finish() as usize % candidates.len();
                &candidates[idx]
            }

            LoadBalanceStrategy::WeightedQuota => {
                // Weight by remaining rate limit capacity
                candidates
                    .iter()
                    .max_by_key(|s| {
                        let state = &s.rate_limit_state;
                        state.max_requests_per_window.saturating_sub(state.requests_in_window)
                    })
                    .unwrap()
            }
        };

        Ok(selected.clone())
    }

    /// Release a subscription (called when done with request)
    pub fn release(&self, id: SubscriptionId, success: bool, tokens_used: u64) -> Result<()> {
        if success {
            self.registry.record_success(id, tokens_used)?;
        } else {
            self.registry.record_failure(id)?;
        }
        Ok(())
    }

    /// Mark subscription as rate limited
    pub fn mark_rate_limited(&self, id: SubscriptionId, duration: Duration) -> Result<()> {
        self.registry.mark_rate_limited(id, duration)?;
        tracing::warn!(
            "Subscription {} rate limited for {:?}",
            id,
            duration
        );
        Ok(())
    }

    /// Clear sticky session for an agent
    pub fn clear_sticky_session(&self, agent_id: AgentId) {
        let mut sessions = self.sticky_sessions.write().unwrap();
        sessions.remove(&agent_id);
    }

    /// Clear all sticky sessions
    pub fn clear_all_sticky_sessions(&self) {
        let mut sessions = self.sticky_sessions.write().unwrap();
        sessions.clear();
    }

    /// Get pool status summary
    pub fn status(&self) -> PoolStatus {
        let all = self.registry.list();
        let available = self.registry.list_available();
        let sticky = self.sticky_sessions.read().unwrap();

        PoolStatus {
            total_subscriptions: all.len(),
            available_subscriptions: available.len(),
            active_sticky_sessions: sticky.len(),
            subscriptions: all,
        }
    }

    /// Get the underlying registry
    pub fn registry(&self) -> &Arc<SubscriptionRegistry> {
        &self.registry
    }
}

/// Status summary of the subscription pool
#[derive(Debug, Clone)]
pub struct PoolStatus {
    /// Total number of registered subscriptions
    pub total_subscriptions: usize,
    /// Number of currently available subscriptions
    pub available_subscriptions: usize,
    /// Number of active sticky sessions
    pub active_sticky_sessions: usize,
    /// All subscriptions with their current state
    pub subscriptions: Vec<Subscription>,
}

/// A lease on a subscription for a single request
pub struct SubscriptionLease<'a> {
    /// The subscription
    pub subscription: Subscription,
    /// Reference to the pool for release
    pool: &'a SubscriptionPool,
}

impl<'a> SubscriptionLease<'a> {
    /// Get the subscription ID
    pub fn id(&self) -> SubscriptionId {
        self.subscription.id
    }

    /// Get the subscription name
    pub fn name(&self) -> &str {
        &self.subscription.name
    }

    /// Get the API key reference
    pub fn api_key_ref(&self) -> &str {
        &self.subscription.api_key_ref
    }

    /// Mark the request as successful
    pub fn success(self, tokens_used: u64) -> Result<()> {
        self.pool.release(self.subscription.id, true, tokens_used)
    }

    /// Mark the request as failed
    pub fn failure(self) -> Result<()> {
        self.pool.release(self.subscription.id, false, 0)
    }

    /// Mark as rate limited (and release)
    pub fn rate_limited(self, duration: Duration) -> Result<()> {
        self.pool.mark_rate_limited(self.subscription.id, duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_pool() -> (SubscriptionPool, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("subscriptions.redb");
        let registry = Arc::new(SubscriptionRegistry::open(path).unwrap());
        let pool = SubscriptionPool::new(registry);
        (pool, dir)
    }

    #[test]
    fn test_acquire_release() {
        let (pool, _dir) = create_test_pool();

        // Add a subscription
        pool.registry()
            .create("test-sub", "key", super::super::ProviderType::ClaudeCode)
            .unwrap();

        // Acquire
        let context = RequestContext::new();
        let lease = pool.acquire(&context).unwrap();
        assert_eq!(lease.name(), "test-sub");

        // Release with success
        lease.success(100).unwrap();

        // Verify metrics updated
        let sub = pool.registry().get_by_name("test-sub").unwrap();
        assert_eq!(sub.usage.total_tokens, 100);
    }

    #[test]
    fn test_no_available_subscriptions() {
        let (pool, _dir) = create_test_pool();

        let context = RequestContext::new();
        let result = pool.acquire(&context);
        assert!(result.is_err());
    }

    #[test]
    fn test_round_robin() {
        let (pool, _dir) = create_test_pool();

        pool.registry()
            .create("sub1", "key1", super::super::ProviderType::ClaudeCode)
            .unwrap();
        pool.registry()
            .create("sub2", "key2", super::super::ProviderType::ClaudeCode)
            .unwrap();

        let context = RequestContext::new();

        let lease1 = pool.acquire(&context).unwrap();
        let name1 = lease1.name().to_string();
        lease1.success(0).unwrap();

        let lease2 = pool.acquire(&context).unwrap();
        let name2 = lease2.name().to_string();
        lease2.success(0).unwrap();

        // Should alternate (though order depends on priority sorting)
        assert!(name1 != name2 || pool.registry().list().len() == 1);
    }

    #[test]
    fn test_namespace_routing() {
        let (pool, _dir) = create_test_pool();

        pool.registry()
            .create("backend-sub", "key1", super::super::ProviderType::ClaudeCode)
            .unwrap();
        pool.registry()
            .create("frontend-sub", "key2", super::super::ProviderType::ClaudeCode)
            .unwrap();

        let mut config = RoutingConfig::default();
        config
            .namespace_routes
            .insert("backend".to_string(), vec!["backend-sub".to_string()]);
        config
            .namespace_routes
            .insert("frontend".to_string(), vec!["frontend-sub".to_string()]);
        pool.set_routing_config(config);

        // Request for backend namespace
        let context = RequestContext::new().with_namespace("backend");
        let lease = pool.acquire(&context).unwrap();
        assert_eq!(lease.name(), "backend-sub");
        lease.success(0).unwrap();

        // Request for frontend namespace
        let context = RequestContext::new().with_namespace("frontend");
        let lease = pool.acquire(&context).unwrap();
        assert_eq!(lease.name(), "frontend-sub");
        lease.success(0).unwrap();
    }

    #[test]
    fn test_sticky_sessions() {
        let (pool, _dir) = create_test_pool();

        pool.registry()
            .create("sub1", "key1", super::super::ProviderType::ClaudeCode)
            .unwrap();
        pool.registry()
            .create("sub2", "key2", super::super::ProviderType::ClaudeCode)
            .unwrap();

        let mut config = RoutingConfig::default();
        config.sticky_sessions = true;
        pool.set_routing_config(config);

        let agent_id = AgentId::new();
        let context = RequestContext::new().with_agent(agent_id);

        // First request assigns a subscription
        let lease1 = pool.acquire(&context).unwrap();
        let assigned_name = lease1.name().to_string();
        lease1.success(0).unwrap();

        // Subsequent requests should use the same subscription
        for _ in 0..5 {
            let lease = pool.acquire(&context).unwrap();
            assert_eq!(lease.name(), assigned_name);
            lease.success(0).unwrap();
        }

        // Clear sticky session
        pool.clear_sticky_session(agent_id);
    }
}
