//! Subscription registry for CRUD operations
//!
//! Manages the persistent storage of subscriptions using redb.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use anyhow::{Context, Result};
use redb::{Database, ReadableTable, TableDefinition};

use super::{ProviderType, Subscription, SubscriptionId, SubscriptionStatus};

const SUBSCRIPTIONS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("subscriptions");

/// Registry for managing subscriptions
pub struct SubscriptionRegistry {
    /// In-memory cache of subscriptions
    subscriptions: RwLock<HashMap<SubscriptionId, Subscription>>,
    /// Database for persistence
    db: Database,
}

impl SubscriptionRegistry {
    /// Open or create a subscription registry
    pub fn open(path: PathBuf) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let db = Database::create(&path)
            .with_context(|| format!("Failed to open subscription database at {:?}", path))?;

        // Initialize table
        {
            let write_txn = db.begin_write()?;
            {
                let _ = write_txn.open_table(SUBSCRIPTIONS_TABLE)?;
            }
            write_txn.commit()?;
        }

        let registry = Self {
            subscriptions: RwLock::new(HashMap::new()),
            db,
        };

        // Load existing subscriptions
        registry.load_all()?;

        Ok(registry)
    }

    /// Load all subscriptions from database into memory
    fn load_all(&self) -> Result<()> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(SUBSCRIPTIONS_TABLE)?;

        let mut subs = self.subscriptions.write().unwrap();
        subs.clear();

        for entry in table.iter()? {
            let (_, value) = entry?;
            let sub: Subscription = rmp_serde::from_slice(value.value())?;
            subs.insert(sub.id, sub);
        }

        Ok(())
    }

    /// Add a new subscription
    pub fn add(&self, subscription: Subscription) -> Result<SubscriptionId> {
        let id = subscription.id;

        // Check for duplicate name
        {
            let subs = self.subscriptions.read().unwrap();
            if subs.values().any(|s| s.name == subscription.name) {
                anyhow::bail!("Subscription with name '{}' already exists", subscription.name);
            }
        }

        // Persist to database
        self.persist(&subscription)?;

        // Add to cache
        {
            let mut subs = self.subscriptions.write().unwrap();
            subs.insert(id, subscription);
        }

        tracing::info!("Added subscription: {} ({})", id, id);
        Ok(id)
    }

    /// Create and add a new subscription
    pub fn create(
        &self,
        name: &str,
        api_key_ref: &str,
        provider: ProviderType,
    ) -> Result<SubscriptionId> {
        let subscription = Subscription::new(name, api_key_ref).with_provider(provider);
        self.add(subscription)
    }

    /// Get a subscription by ID
    pub fn get(&self, id: SubscriptionId) -> Option<Subscription> {
        let subs = self.subscriptions.read().unwrap();
        subs.get(&id).cloned()
    }

    /// Get a subscription by name
    pub fn get_by_name(&self, name: &str) -> Option<Subscription> {
        let subs = self.subscriptions.read().unwrap();
        subs.values().find(|s| s.name == name).cloned()
    }

    /// Update a subscription
    pub fn update(&self, subscription: Subscription) -> Result<()> {
        let id = subscription.id;

        // Check if exists
        {
            let subs = self.subscriptions.read().unwrap();
            if !subs.contains_key(&id) {
                anyhow::bail!("Subscription {} not found", id);
            }
        }

        // Persist to database
        self.persist(&subscription)?;

        // Update cache
        {
            let mut subs = self.subscriptions.write().unwrap();
            subs.insert(id, subscription);
        }

        Ok(())
    }

    /// Remove a subscription
    pub fn remove(&self, id: SubscriptionId) -> Result<Option<Subscription>> {
        // Remove from cache first
        let subscription = {
            let mut subs = self.subscriptions.write().unwrap();
            subs.remove(&id)
        };

        if subscription.is_some() {
            // Remove from database
            let write_txn = self.db.begin_write()?;
            {
                let mut table = write_txn.open_table(SUBSCRIPTIONS_TABLE)?;
                table.remove(id.to_string().as_str())?;
            }
            write_txn.commit()?;

            tracing::info!("Removed subscription: {}", id);
        }

        Ok(subscription)
    }

    /// Remove a subscription by name
    pub fn remove_by_name(&self, name: &str) -> Result<Option<Subscription>> {
        let id = {
            let subs = self.subscriptions.read().unwrap();
            subs.values().find(|s| s.name == name).map(|s| s.id)
        };

        match id {
            Some(id) => self.remove(id),
            None => Ok(None),
        }
    }

    /// List all subscriptions
    pub fn list(&self) -> Vec<Subscription> {
        let subs = self.subscriptions.read().unwrap();
        let mut list: Vec<_> = subs.values().cloned().collect();
        // Sort by priority (highest first), then by name
        list.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.name.cmp(&b.name)));
        list
    }

    /// List available subscriptions (active and within rate limits)
    pub fn list_available(&self) -> Vec<Subscription> {
        let subs = self.subscriptions.read().unwrap();
        let mut list: Vec<_> = subs.values().filter(|s| s.is_available()).cloned().collect();
        list.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.name.cmp(&b.name)));
        list
    }

    /// List subscriptions by status
    pub fn list_by_status(&self, status_filter: &str) -> Vec<Subscription> {
        let subs = self.subscriptions.read().unwrap();
        subs.values()
            .filter(|s| match status_filter {
                "active" => matches!(s.status, SubscriptionStatus::Active),
                "disabled" => matches!(s.status, SubscriptionStatus::Disabled),
                "rate_limited" => matches!(s.status, SubscriptionStatus::RateLimited { .. }),
                "exhausted" => matches!(s.status, SubscriptionStatus::Exhausted { .. }),
                "unhealthy" => matches!(s.status, SubscriptionStatus::Unhealthy { .. }),
                _ => true,
            })
            .cloned()
            .collect()
    }

    /// List subscriptions with specific tag
    pub fn list_by_tag(&self, tag: &str) -> Vec<Subscription> {
        let subs = self.subscriptions.read().unwrap();
        subs.values()
            .filter(|s| s.tags.contains(&tag.to_string()))
            .cloned()
            .collect()
    }

    /// Enable a subscription
    pub fn enable(&self, id: SubscriptionId) -> Result<()> {
        let mut subscription = self.get(id).ok_or_else(|| anyhow::anyhow!("Subscription {} not found", id))?;
        subscription.enable();
        self.update(subscription)
    }

    /// Disable a subscription
    pub fn disable(&self, id: SubscriptionId) -> Result<()> {
        let mut subscription = self.get(id).ok_or_else(|| anyhow::anyhow!("Subscription {} not found", id))?;
        subscription.disable();
        self.update(subscription)
    }

    /// Get count of subscriptions
    pub fn count(&self) -> usize {
        let subs = self.subscriptions.read().unwrap();
        subs.len()
    }

    /// Get count of available subscriptions
    pub fn count_available(&self) -> usize {
        let subs = self.subscriptions.read().unwrap();
        subs.values().filter(|s| s.is_available()).count()
    }

    /// Check if registry has any subscriptions
    pub fn is_empty(&self) -> bool {
        let subs = self.subscriptions.read().unwrap();
        subs.is_empty()
    }

    /// Persist a subscription to database
    fn persist(&self, subscription: &Subscription) -> Result<()> {
        let data = rmp_serde::to_vec(subscription)?;

        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(SUBSCRIPTIONS_TABLE)?;
            table.insert(subscription.id.to_string().as_str(), data.as_slice())?;
        }
        write_txn.commit()?;

        Ok(())
    }

    /// Record successful usage for a subscription
    pub fn record_success(&self, id: SubscriptionId, tokens: u64) -> Result<()> {
        let mut subscription = self.get(id).ok_or_else(|| anyhow::anyhow!("Subscription {} not found", id))?;
        subscription.record_success(tokens);
        self.update(subscription)
    }

    /// Record failed usage for a subscription
    pub fn record_failure(&self, id: SubscriptionId) -> Result<()> {
        let mut subscription = self.get(id).ok_or_else(|| anyhow::anyhow!("Subscription {} not found", id))?;
        subscription.record_failure();
        self.update(subscription)
    }

    /// Mark subscription as rate limited
    pub fn mark_rate_limited(&self, id: SubscriptionId, duration: std::time::Duration) -> Result<()> {
        let mut subscription = self.get(id).ok_or_else(|| anyhow::anyhow!("Subscription {} not found", id))?;
        subscription.mark_rate_limited(duration);
        self.update(subscription)
    }

    /// Mark subscription as unhealthy
    pub fn mark_unhealthy(&self, id: SubscriptionId, reason: &str) -> Result<()> {
        let mut subscription = self.get(id).ok_or_else(|| anyhow::anyhow!("Subscription {} not found", id))?;
        subscription.mark_unhealthy(reason);
        self.update(subscription)
    }

    /// Update the API key for a subscription (stores directly in database)
    pub fn update_api_key(&self, id: SubscriptionId, api_key: &str) -> Result<()> {
        let mut subscription = self.get(id).ok_or_else(|| anyhow::anyhow!("Subscription {} not found", id))?;
        subscription.api_key = Some(api_key.to_string());
        subscription.updated_at = chrono::Utc::now().timestamp();
        self.update(subscription)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_registry_crud() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("subscriptions.redb");
        let registry = SubscriptionRegistry::open(path).unwrap();

        // Create
        let id = registry
            .create("test-sub", "keychain:test-key", ProviderType::ClaudeCode)
            .unwrap();

        // Read
        let sub = registry.get(id).unwrap();
        assert_eq!(sub.name, "test-sub");

        // Get by name
        let sub = registry.get_by_name("test-sub").unwrap();
        assert_eq!(sub.id, id);

        // List
        let list = registry.list();
        assert_eq!(list.len(), 1);

        // Update
        let mut sub = registry.get(id).unwrap();
        sub.priority = 200;
        registry.update(sub).unwrap();

        let sub = registry.get(id).unwrap();
        assert_eq!(sub.priority, 200);

        // Delete
        let removed = registry.remove(id).unwrap();
        assert!(removed.is_some());
        assert!(registry.get(id).is_none());
    }

    #[test]
    fn test_duplicate_name_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("subscriptions.redb");
        let registry = SubscriptionRegistry::open(path).unwrap();

        registry
            .create("test-sub", "key1", ProviderType::ClaudeCode)
            .unwrap();

        let result = registry.create("test-sub", "key2", ProviderType::ClaudeCode);
        assert!(result.is_err());
    }

    #[test]
    fn test_list_available() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("subscriptions.redb");
        let registry = SubscriptionRegistry::open(path).unwrap();

        let id1 = registry
            .create("sub1", "key1", ProviderType::ClaudeCode)
            .unwrap();
        let id2 = registry
            .create("sub2", "key2", ProviderType::ClaudeCode)
            .unwrap();

        // Disable one
        registry.disable(id2).unwrap();

        let available = registry.list_available();
        assert_eq!(available.len(), 1);
        assert_eq!(available[0].id, id1);
    }
}
