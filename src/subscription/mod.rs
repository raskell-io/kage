//! Multi-subscription management for Claude Code
//!
//! This module enables users to register and manage multiple Claude Code subscriptions
//! (e.g., "Claude Code 20x Max") for parallel productivity scaling.
//!
//! Key features:
//! - Subscription pool with intelligent routing
//! - Rate limit management and automatic cooldown
//! - Load balancing across subscriptions
//! - Usage tracking and health monitoring

pub mod health;
pub mod pool;
pub mod registry;
pub mod usage;

// Re-export main types
pub use pool::SubscriptionPool;
pub use registry::SubscriptionRegistry;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use ulid::Ulid;

/// Unique identifier for a subscription
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubscriptionId(Ulid);

impl SubscriptionId {
    /// Create a new unique subscription ID
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for SubscriptionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SubscriptionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for SubscriptionId {
    type Err = ulid::DecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Ulid::from_string(s)?))
    }
}

/// Provider type for the subscription
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    /// Claude Code CLI (primary)
    ClaudeCode,
    /// Direct Anthropic API
    AnthropicApi,
    /// OpenAI API (future)
    OpenAi,
    /// Google Gemini API (future)
    Gemini,
    /// Custom provider
    Custom(String),
}

impl Default for ProviderType {
    fn default() -> Self {
        Self::ClaudeCode
    }
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClaudeCode => write!(f, "claude-code"),
            Self::AnthropicApi => write!(f, "anthropic-api"),
            Self::OpenAi => write!(f, "openai"),
            Self::Gemini => write!(f, "gemini"),
            Self::Custom(name) => write!(f, "custom:{}", name),
        }
    }
}

/// Current status of a subscription
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
    /// Subscription is active and available
    Active,
    /// Subscription is temporarily rate limited (with cooldown end time)
    RateLimited {
        /// When the rate limit expires (unix timestamp)
        until: i64,
    },
    /// Subscription quota is exhausted for the period
    Exhausted {
        /// When quota resets (unix timestamp)
        resets_at: i64,
    },
    /// Subscription is disabled by user
    Disabled,
    /// Subscription failed health check
    Unhealthy {
        /// Reason for unhealthy status
        reason: String,
        /// When it was marked unhealthy (unix timestamp)
        since: i64,
    },
}

impl Default for SubscriptionStatus {
    fn default() -> Self {
        Self::Active
    }
}

impl SubscriptionStatus {
    /// Check if the subscription is available for use
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// Check if the subscription is rate limited but will recover
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::RateLimited { .. })
    }

    /// Get the cooldown remaining if rate limited
    pub fn cooldown_remaining(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { until } => {
                let now = chrono::Utc::now().timestamp();
                if *until > now {
                    Some(Duration::from_secs((*until - now) as u64))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// Rate limit tracking state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitState {
    /// Requests made in current window
    pub requests_in_window: u32,
    /// Window start time (unix timestamp)
    pub window_start: i64,
    /// Window duration in seconds
    pub window_duration_secs: u64,
    /// Maximum requests per window
    pub max_requests_per_window: u32,
    /// Tokens used today
    pub tokens_today: u64,
    /// Daily token limit (0 = unlimited)
    pub daily_token_limit: u64,
    /// Consecutive errors
    pub consecutive_errors: u32,
    /// Last error time (unix timestamp)
    pub last_error_at: Option<i64>,
}

impl Default for RateLimitState {
    fn default() -> Self {
        Self {
            requests_in_window: 0,
            window_start: chrono::Utc::now().timestamp(),
            window_duration_secs: 60, // 1 minute window
            max_requests_per_window: 60, // 60 requests per minute default
            tokens_today: 0,
            daily_token_limit: 0, // Unlimited by default
            consecutive_errors: 0,
            last_error_at: None,
        }
    }
}

impl RateLimitState {
    /// Check if we're within rate limits
    pub fn is_within_limits(&self) -> bool {
        let now = chrono::Utc::now().timestamp();

        // Check if window has expired
        if now - self.window_start >= self.window_duration_secs as i64 {
            return true; // Window expired, limits reset
        }

        // Check request limit
        if self.requests_in_window >= self.max_requests_per_window {
            return false;
        }

        // Check daily token limit
        if self.daily_token_limit > 0 && self.tokens_today >= self.daily_token_limit {
            return false;
        }

        true
    }

    /// Record a request
    pub fn record_request(&mut self, tokens: u64) {
        let now = chrono::Utc::now().timestamp();

        // Reset window if expired
        if now - self.window_start >= self.window_duration_secs as i64 {
            self.requests_in_window = 0;
            self.window_start = now;
        }

        self.requests_in_window += 1;
        self.tokens_today += tokens;
        self.consecutive_errors = 0;
    }

    /// Record an error
    pub fn record_error(&mut self) {
        self.consecutive_errors += 1;
        self.last_error_at = Some(chrono::Utc::now().timestamp());
    }

    /// Reset daily counters (call at midnight)
    pub fn reset_daily(&mut self) {
        self.tokens_today = 0;
    }

    /// Calculate backoff duration based on consecutive errors
    pub fn error_backoff(&self) -> Duration {
        let base_secs = 5u64;
        let max_secs = 300u64; // 5 minutes max
        let backoff = base_secs * 2u64.pow(self.consecutive_errors.min(6));
        Duration::from_secs(backoff.min(max_secs))
    }
}

/// Usage metrics for a subscription
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageMetrics {
    /// Total requests made
    pub total_requests: u64,
    /// Total tokens used
    pub total_tokens: u64,
    /// Successful requests
    pub successful_requests: u64,
    /// Failed requests
    pub failed_requests: u64,
    /// Rate limit hits
    pub rate_limit_hits: u64,
    /// Last used timestamp
    pub last_used_at: Option<i64>,
    /// First used timestamp
    pub first_used_at: Option<i64>,
}

impl UsageMetrics {
    /// Record a successful request
    pub fn record_success(&mut self, tokens: u64) {
        let now = chrono::Utc::now().timestamp();
        self.total_requests += 1;
        self.total_tokens += tokens;
        self.successful_requests += 1;
        self.last_used_at = Some(now);
        if self.first_used_at.is_none() {
            self.first_used_at = Some(now);
        }
    }

    /// Record a failed request
    pub fn record_failure(&mut self) {
        self.total_requests += 1;
        self.failed_requests += 1;
        self.last_used_at = Some(chrono::Utc::now().timestamp());
    }

    /// Record a rate limit hit
    pub fn record_rate_limit(&mut self) {
        self.rate_limit_hits += 1;
    }

    /// Calculate success rate
    pub fn success_rate(&self) -> f64 {
        if self.total_requests == 0 {
            1.0
        } else {
            self.successful_requests as f64 / self.total_requests as f64
        }
    }
}

/// A registered subscription
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    /// Unique identifier
    pub id: SubscriptionId,
    /// User-friendly name (e.g., "work-account", "personal")
    pub name: String,
    /// Reference to keychain entry for API key (legacy, kept for compatibility)
    pub api_key_ref: String,
    /// Actual API key (stored directly in database for persistence)
    /// This avoids repeated keychain access on daemon restart
    #[serde(default)]
    pub api_key: Option<String>,
    /// Provider type
    pub provider: ProviderType,
    /// Current status
    pub status: SubscriptionStatus,
    /// Rate limit tracking
    pub rate_limit_state: RateLimitState,
    /// Usage metrics
    pub usage: UsageMetrics,
    /// Priority (higher = used first, 0-255)
    pub priority: u8,
    /// Tags for routing rules
    pub tags: Vec<String>,
    /// Created timestamp
    pub created_at: i64,
    /// Last modified timestamp
    pub updated_at: i64,
    /// Optional description
    pub description: Option<String>,
}

impl Subscription {
    /// Create a new subscription
    pub fn new(name: &str, api_key_ref: &str) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            id: SubscriptionId::new(),
            name: name.to_string(),
            api_key_ref: api_key_ref.to_string(),
            api_key: None,
            provider: ProviderType::default(),
            status: SubscriptionStatus::default(),
            rate_limit_state: RateLimitState::default(),
            usage: UsageMetrics::default(),
            priority: 100, // Default middle priority
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            description: None,
        }
    }

    /// Set the provider type
    pub fn with_provider(mut self, provider: ProviderType) -> Self {
        self.provider = provider;
        self
    }

    /// Set the priority
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    /// Add tags
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Set description
    pub fn with_description(mut self, description: &str) -> Self {
        self.description = Some(description.to_string());
        self
    }

    /// Set the API key directly (stored in database, no keychain needed)
    pub fn with_api_key(mut self, api_key: &str) -> Self {
        self.api_key = Some(api_key.to_string());
        self
    }

    /// Check if subscription is available for use
    pub fn is_available(&self) -> bool {
        self.status.is_available() && self.rate_limit_state.is_within_limits()
    }

    /// Enable the subscription
    pub fn enable(&mut self) {
        self.status = SubscriptionStatus::Active;
        self.updated_at = chrono::Utc::now().timestamp();
    }

    /// Disable the subscription
    pub fn disable(&mut self) {
        self.status = SubscriptionStatus::Disabled;
        self.updated_at = chrono::Utc::now().timestamp();
    }

    /// Mark as rate limited
    pub fn mark_rate_limited(&mut self, duration: Duration) {
        let until = chrono::Utc::now().timestamp() + duration.as_secs() as i64;
        self.status = SubscriptionStatus::RateLimited { until };
        self.usage.record_rate_limit();
        self.updated_at = chrono::Utc::now().timestamp();
    }

    /// Mark as unhealthy
    pub fn mark_unhealthy(&mut self, reason: &str) {
        let now = chrono::Utc::now().timestamp();
        self.status = SubscriptionStatus::Unhealthy {
            reason: reason.to_string(),
            since: now,
        };
        self.updated_at = now;
    }

    /// Record successful use
    pub fn record_success(&mut self, tokens: u64) {
        self.rate_limit_state.record_request(tokens);
        self.usage.record_success(tokens);
        self.updated_at = chrono::Utc::now().timestamp();

        // If was rate limited and now recovered, mark active
        if let SubscriptionStatus::RateLimited { until } = &self.status {
            if chrono::Utc::now().timestamp() >= *until {
                self.status = SubscriptionStatus::Active;
            }
        }
    }

    /// Record failed use
    pub fn record_failure(&mut self) {
        self.rate_limit_state.record_error();
        self.usage.record_failure();
        self.updated_at = chrono::Utc::now().timestamp();
    }
}

/// Routing configuration for subscriptions
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Map namespace to preferred subscriptions
    pub namespace_routes: HashMap<String, Vec<String>>,
    /// Map priority level to subscriptions
    pub priority_routes: HashMap<String, Vec<String>>,
    /// Default subscriptions when no specific route matches
    pub default_subscriptions: Vec<String>,
    /// Enable sticky sessions (keep agent on same subscription)
    pub sticky_sessions: bool,
    /// Load balancing strategy
    pub strategy: LoadBalanceStrategy,
}

/// Load balancing strategy
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalanceStrategy {
    /// Round-robin across available subscriptions
    #[default]
    RoundRobin,
    /// Use highest priority first
    Priority,
    /// Use least recently used
    LeastRecentlyUsed,
    /// Random selection
    Random,
    /// Weighted by remaining quota
    WeightedQuota,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subscription_creation() {
        let sub = Subscription::new("test-sub", "keychain:test-key");
        assert_eq!(sub.name, "test-sub");
        assert_eq!(sub.api_key_ref, "keychain:test-key");
        assert!(sub.is_available());
    }

    #[test]
    fn test_subscription_status() {
        let mut sub = Subscription::new("test", "key");
        assert!(sub.status.is_available());

        sub.disable();
        assert!(!sub.status.is_available());

        sub.enable();
        assert!(sub.status.is_available());
    }

    #[test]
    fn test_rate_limit_state() {
        let mut state = RateLimitState::default();
        assert!(state.is_within_limits());

        // Record requests up to limit
        for _ in 0..60 {
            state.record_request(100);
        }
        assert!(!state.is_within_limits());
    }

    #[test]
    fn test_usage_metrics() {
        let mut metrics = UsageMetrics::default();
        assert_eq!(metrics.success_rate(), 1.0);

        metrics.record_success(1000);
        metrics.record_success(500);
        metrics.record_failure();

        assert_eq!(metrics.total_requests, 3);
        assert_eq!(metrics.total_tokens, 1500);
        assert_eq!(metrics.successful_requests, 2);
        assert_eq!(metrics.failed_requests, 1);
    }

    #[test]
    fn test_error_backoff() {
        let mut state = RateLimitState::default();
        assert_eq!(state.error_backoff(), Duration::from_secs(5));

        state.record_error();
        assert_eq!(state.error_backoff(), Duration::from_secs(10));

        state.record_error();
        assert_eq!(state.error_backoff(), Duration::from_secs(20));
    }
}
