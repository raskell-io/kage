//! Health monitoring for subscriptions
//!
//! Monitors subscription health, detects issues, and triggers alerts.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::mpsc;

use super::registry::SubscriptionRegistry;
use super::{Subscription, SubscriptionId, SubscriptionStatus};

/// Health check result
#[derive(Debug, Clone)]
pub enum HealthCheckResult {
    /// Subscription is healthy
    Healthy,
    /// Subscription has a warning
    Warning { reason: String },
    /// Subscription is unhealthy
    Unhealthy { reason: String },
}

/// Health alert type
#[derive(Debug, Clone)]
pub enum HealthAlert {
    /// Subscription became unhealthy
    SubscriptionUnhealthy {
        subscription_id: SubscriptionId,
        subscription_name: String,
        reason: String,
    },
    /// Subscription recovered
    SubscriptionRecovered {
        subscription_id: SubscriptionId,
        subscription_name: String,
    },
    /// Subscription quota exhausted
    QuotaExhausted {
        subscription_id: SubscriptionId,
        subscription_name: String,
        resets_at: i64,
    },
    /// High error rate detected
    HighErrorRate {
        subscription_id: SubscriptionId,
        subscription_name: String,
        error_rate: f64,
    },
    /// No available subscriptions
    NoAvailableSubscriptions,
    /// Pool health degraded
    PoolHealthDegraded {
        available: usize,
        total: usize,
    },
}

/// Configuration for health monitoring
#[derive(Debug, Clone)]
pub struct HealthConfig {
    /// Check interval
    pub check_interval: Duration,
    /// Error rate threshold for alerts (0.0 - 1.0)
    pub error_rate_threshold: f64,
    /// Consecutive failures before marking unhealthy
    pub max_consecutive_failures: u32,
    /// Enable automatic recovery attempts
    pub auto_recovery: bool,
    /// Recovery check interval
    pub recovery_interval: Duration,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(60),
            error_rate_threshold: 0.5,
            max_consecutive_failures: 3,
            auto_recovery: true,
            recovery_interval: Duration::from_secs(300),
        }
    }
}

/// Health monitor for subscription pool
pub struct HealthMonitor {
    /// Subscription registry
    registry: Arc<SubscriptionRegistry>,
    /// Health configuration
    config: HealthConfig,
    /// Alert sender
    alert_tx: mpsc::Sender<HealthAlert>,
    /// Previous health states for change detection
    previous_states: HashMap<SubscriptionId, SubscriptionStatus>,
}

impl HealthMonitor {
    /// Create a new health monitor
    pub fn new(
        registry: Arc<SubscriptionRegistry>,
        config: HealthConfig,
    ) -> (Self, mpsc::Receiver<HealthAlert>) {
        let (alert_tx, alert_rx) = mpsc::channel(100);

        let monitor = Self {
            registry,
            config,
            alert_tx,
            previous_states: HashMap::new(),
        };

        (monitor, alert_rx)
    }

    /// Run a single health check cycle
    pub async fn check(&mut self) -> Result<HealthSummary> {
        let subscriptions = self.registry.list();

        let mut summary = HealthSummary {
            total: subscriptions.len(),
            healthy: 0,
            warning: 0,
            unhealthy: 0,
            results: HashMap::new(),
        };

        for sub in &subscriptions {
            let result = self.check_subscription(sub).await;

            match &result {
                HealthCheckResult::Healthy => summary.healthy += 1,
                HealthCheckResult::Warning { .. } => summary.warning += 1,
                HealthCheckResult::Unhealthy { .. } => summary.unhealthy += 1,
            }

            summary.results.insert(sub.id, result.clone());

            // Detect state changes and send alerts
            self.detect_state_changes(sub, &result).await;
        }

        // Check pool-level health
        self.check_pool_health(&summary).await;

        Ok(summary)
    }

    /// Check health of a single subscription
    async fn check_subscription(&self, sub: &Subscription) -> HealthCheckResult {
        // Check if disabled
        if matches!(sub.status, SubscriptionStatus::Disabled) {
            return HealthCheckResult::Warning {
                reason: "Subscription is disabled".to_string(),
            };
        }

        // Check if unhealthy
        if let SubscriptionStatus::Unhealthy { reason, .. } = &sub.status {
            return HealthCheckResult::Unhealthy {
                reason: reason.clone(),
            };
        }

        // Check if exhausted
        if let SubscriptionStatus::Exhausted { resets_at } = &sub.status {
            return HealthCheckResult::Unhealthy {
                reason: format!("Quota exhausted, resets at {}", resets_at),
            };
        }

        // Check if rate limited
        if let SubscriptionStatus::RateLimited { until } = &sub.status {
            let now = chrono::Utc::now().timestamp();
            if *until > now {
                return HealthCheckResult::Warning {
                    reason: format!("Rate limited until {}", until),
                };
            }
        }

        // Check error rate
        let error_rate = 1.0 - sub.usage.success_rate();
        if error_rate > self.config.error_rate_threshold {
            return HealthCheckResult::Warning {
                reason: format!("High error rate: {:.1}%", error_rate * 100.0),
            };
        }

        // Check consecutive errors
        if sub.rate_limit_state.consecutive_errors >= self.config.max_consecutive_failures {
            return HealthCheckResult::Unhealthy {
                reason: format!(
                    "Too many consecutive errors: {}",
                    sub.rate_limit_state.consecutive_errors
                ),
            };
        }

        HealthCheckResult::Healthy
    }

    /// Detect state changes and send alerts
    async fn detect_state_changes(&mut self, sub: &Subscription, result: &HealthCheckResult) {
        let previous = self.previous_states.get(&sub.id);

        // Check for recovery
        if let Some(prev) = previous {
            if matches!(prev, SubscriptionStatus::Unhealthy { .. })
                && matches!(result, HealthCheckResult::Healthy)
            {
                let _ = self
                    .alert_tx
                    .send(HealthAlert::SubscriptionRecovered {
                        subscription_id: sub.id,
                        subscription_name: sub.name.clone(),
                    })
                    .await;
            }
        }

        // Check for new unhealthy state
        if let HealthCheckResult::Unhealthy { reason } = result {
            if !matches!(
                previous,
                Some(SubscriptionStatus::Unhealthy { .. })
            ) {
                let _ = self
                    .alert_tx
                    .send(HealthAlert::SubscriptionUnhealthy {
                        subscription_id: sub.id,
                        subscription_name: sub.name.clone(),
                        reason: reason.clone(),
                    })
                    .await;
            }
        }

        // Check for quota exhaustion
        if let SubscriptionStatus::Exhausted { resets_at } = &sub.status {
            if !matches!(previous, Some(SubscriptionStatus::Exhausted { .. })) {
                let _ = self
                    .alert_tx
                    .send(HealthAlert::QuotaExhausted {
                        subscription_id: sub.id,
                        subscription_name: sub.name.clone(),
                        resets_at: *resets_at,
                    })
                    .await;
            }
        }

        // Check for high error rate
        let error_rate = 1.0 - sub.usage.success_rate();
        if error_rate > self.config.error_rate_threshold {
            let _ = self
                .alert_tx
                .send(HealthAlert::HighErrorRate {
                    subscription_id: sub.id,
                    subscription_name: sub.name.clone(),
                    error_rate,
                })
                .await;
        }

        // Update previous state
        self.previous_states.insert(sub.id, sub.status.clone());
    }

    /// Check pool-level health
    async fn check_pool_health(&self, summary: &HealthSummary) {
        // No available subscriptions
        if summary.healthy == 0 && summary.total > 0 {
            let _ = self
                .alert_tx
                .send(HealthAlert::NoAvailableSubscriptions)
                .await;
        }

        // Pool health degraded (less than half healthy)
        if summary.healthy < summary.total / 2 && summary.total > 1 {
            let _ = self
                .alert_tx
                .send(HealthAlert::PoolHealthDegraded {
                    available: summary.healthy,
                    total: summary.total,
                })
                .await;
        }
    }

    /// Attempt to recover unhealthy subscriptions
    pub async fn attempt_recovery(&self) -> Result<Vec<SubscriptionId>> {
        let mut recovered = Vec::new();

        for sub in self.registry.list() {
            if let SubscriptionStatus::Unhealthy { since, .. } = &sub.status {
                let now = chrono::Utc::now().timestamp();
                let unhealthy_duration = now - since;

                // Only attempt recovery after recovery interval
                if unhealthy_duration >= self.config.recovery_interval.as_secs() as i64 {
                    // For now, just re-enable the subscription
                    // In a real implementation, we'd do an actual health check
                    self.registry.enable(sub.id)?;
                    recovered.push(sub.id);
                    tracing::info!("Attempted recovery for subscription {}", sub.name);
                }
            }

            // Recover from rate limiting if cooldown expired
            if let SubscriptionStatus::RateLimited { until } = &sub.status {
                let now = chrono::Utc::now().timestamp();
                if now >= *until {
                    self.registry.enable(sub.id)?;
                    recovered.push(sub.id);
                    tracing::info!(
                        "Subscription {} recovered from rate limiting",
                        sub.name
                    );
                }
            }
        }

        Ok(recovered)
    }

    /// Get current health configuration
    pub fn config(&self) -> &HealthConfig {
        &self.config
    }

    /// Update health configuration
    pub fn set_config(&mut self, config: HealthConfig) {
        self.config = config;
    }
}

/// Summary of a health check
#[derive(Debug, Clone)]
pub struct HealthSummary {
    /// Total subscriptions
    pub total: usize,
    /// Healthy subscriptions
    pub healthy: usize,
    /// Subscriptions with warnings
    pub warning: usize,
    /// Unhealthy subscriptions
    pub unhealthy: usize,
    /// Per-subscription results
    pub results: HashMap<SubscriptionId, HealthCheckResult>,
}

impl HealthSummary {
    /// Get overall health status
    pub fn status(&self) -> OverallHealth {
        if self.total == 0 {
            OverallHealth::Unknown
        } else if self.healthy == self.total {
            OverallHealth::Healthy
        } else if self.unhealthy > 0 {
            OverallHealth::Degraded
        } else if self.warning > 0 {
            OverallHealth::Warning
        } else {
            OverallHealth::Unknown
        }
    }

    /// Get health score (0.0 - 1.0)
    pub fn score(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.healthy as f64 / self.total as f64
        }
    }
}

/// Overall pool health status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverallHealth {
    /// All subscriptions healthy
    Healthy,
    /// Some warnings but functional
    Warning,
    /// Some unhealthy, degraded operation
    Degraded,
    /// Unknown (no subscriptions)
    Unknown,
}

impl std::fmt::Display for OverallHealth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy => write!(f, "healthy"),
            Self::Warning => write!(f, "warning"),
            Self::Degraded => write!(f, "degraded"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Run the health monitor as a background task
pub async fn run_health_monitor(
    registry: Arc<SubscriptionRegistry>,
    config: HealthConfig,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) -> Result<()> {
    let check_interval = config.check_interval;
    let recovery_interval = config.recovery_interval;
    let auto_recovery = config.auto_recovery;

    let (mut monitor, mut alert_rx) = HealthMonitor::new(registry, config);

    let mut check_interval = tokio::time::interval(check_interval);
    let mut recovery_interval = tokio::time::interval(recovery_interval);

    loop {
        tokio::select! {
            _ = check_interval.tick() => {
                match monitor.check().await {
                    Ok(summary) => {
                        tracing::debug!(
                            "Health check complete: {}/{} healthy",
                            summary.healthy,
                            summary.total
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Health check failed: {}", e);
                    }
                }
            }

            _ = recovery_interval.tick() => {
                if auto_recovery {
                    match monitor.attempt_recovery().await {
                        Ok(recovered) if !recovered.is_empty() => {
                            tracing::info!("Recovered {} subscriptions", recovered.len());
                        }
                        Err(e) => {
                            tracing::warn!("Recovery attempt failed: {}", e);
                        }
                        _ => {}
                    }
                }
            }

            Some(alert) = alert_rx.recv() => {
                // Log alerts (in real implementation, would also send notifications)
                match &alert {
                    HealthAlert::SubscriptionUnhealthy { subscription_name, reason, .. } => {
                        tracing::warn!("Subscription {} unhealthy: {}", subscription_name, reason);
                    }
                    HealthAlert::SubscriptionRecovered { subscription_name, .. } => {
                        tracing::info!("Subscription {} recovered", subscription_name);
                    }
                    HealthAlert::QuotaExhausted { subscription_name, .. } => {
                        tracing::warn!("Subscription {} quota exhausted", subscription_name);
                    }
                    HealthAlert::HighErrorRate { subscription_name, error_rate, .. } => {
                        tracing::warn!(
                            "Subscription {} has high error rate: {:.1}%",
                            subscription_name,
                            error_rate * 100.0
                        );
                    }
                    HealthAlert::NoAvailableSubscriptions => {
                        tracing::error!("No available subscriptions!");
                    }
                    HealthAlert::PoolHealthDegraded { available, total } => {
                        tracing::warn!(
                            "Pool health degraded: {}/{} available",
                            available,
                            total
                        );
                    }
                }
            }

            _ = shutdown_rx.recv() => {
                tracing::info!("Health monitor shutting down");
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::ProviderType;
    use tempfile::tempdir;

    fn create_test_registry() -> (Arc<SubscriptionRegistry>, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("subscriptions.redb");
        let registry = Arc::new(SubscriptionRegistry::open(path).unwrap());
        (registry, dir)
    }

    #[tokio::test]
    async fn test_health_check_healthy() {
        let (registry, _dir) = create_test_registry();
        registry
            .create("test-sub", "key", ProviderType::ClaudeCode)
            .unwrap();

        let (mut monitor, _rx) = HealthMonitor::new(registry, HealthConfig::default());
        let summary = monitor.check().await.unwrap();

        assert_eq!(summary.total, 1);
        assert_eq!(summary.healthy, 1);
        assert_eq!(summary.status(), OverallHealth::Healthy);
    }

    #[tokio::test]
    async fn test_health_check_disabled() {
        let (registry, _dir) = create_test_registry();
        let id = registry
            .create("test-sub", "key", ProviderType::ClaudeCode)
            .unwrap();
        registry.disable(id).unwrap();

        let (mut monitor, _rx) = HealthMonitor::new(registry, HealthConfig::default());
        let summary = monitor.check().await.unwrap();

        assert_eq!(summary.warning, 1);
        assert_eq!(summary.healthy, 0);
    }

    #[tokio::test]
    async fn test_health_alert_on_unhealthy() {
        let (registry, _dir) = create_test_registry();
        let id = registry
            .create("test-sub", "key", ProviderType::ClaudeCode)
            .unwrap();
        registry.mark_unhealthy(id, "Test failure").unwrap();

        let (mut monitor, mut rx) = HealthMonitor::new(registry, HealthConfig::default());
        monitor.check().await.unwrap();

        // Should receive an alert
        let alert = rx.try_recv();
        assert!(matches!(
            alert,
            Ok(HealthAlert::SubscriptionUnhealthy { .. })
        ));
    }
}
