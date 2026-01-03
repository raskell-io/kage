//! Secrets management with pluggable backends
//!
//! Supports multiple backends:
//! - OS Keychain (default): macOS Keychain, Linux Secret Service, Windows Credential Manager
//! - AWS Secrets Manager (requires `secrets-aws` feature)
//! - Azure Key Vault (requires `secrets-azure` feature)

pub mod backend;
pub mod backends;

use std::sync::Arc;

use anyhow::Result;

pub use backend::{SecretsBackend, SecretsBackendConfig};

const SERVICE_NAME: &str = "kage";

/// Secret scope
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretScope {
    /// Global secret (available everywhere)
    Global,

    /// Namespace-scoped secret
    Namespace(String),

    /// Repository-scoped secret
    Repository(String),
}

impl SecretScope {
    /// Parse scope from string (e.g., "global", "namespace:backend", "repo:/path/to/repo")
    pub fn parse(s: &str) -> Result<Self> {
        if s == "global" {
            Ok(Self::Global)
        } else if let Some(name) = s.strip_prefix("namespace:") {
            Ok(Self::Namespace(name.to_string()))
        } else if let Some(path) = s.strip_prefix("repo:") {
            Ok(Self::Repository(path.to_string()))
        } else {
            anyhow::bail!("Invalid scope format: {}", s)
        }
    }

    /// Convert to keychain key format
    fn to_key(&self, name: &str) -> String {
        match self {
            Self::Global => format!("global:{}", name),
            Self::Namespace(ns) => format!("namespace:{}:{}", ns, name),
            Self::Repository(path) => format!("repo:{}:{}", path, name),
        }
    }
}

/// Secrets manager with pluggable backend
pub struct SecretsManager {
    backend: Arc<dyn SecretsBackend>,
}

impl SecretsManager {
    /// Create a new secrets manager with the default keyring backend
    pub fn new() -> Self {
        Self {
            backend: Arc::new(backends::KeyringBackend::new()),
        }
    }

    /// Create a new secrets manager with a configurable backend
    pub async fn with_config(config: &SecretsBackendConfig) -> Result<Self> {
        let backend = backend::create_backend(config).await?;
        Ok(Self {
            backend: Arc::from(backend),
        })
    }

    /// Create a new secrets manager with a specific backend
    pub fn with_backend(backend: Box<dyn SecretsBackend>) -> Self {
        Self {
            backend: Arc::from(backend),
        }
    }

    /// Get the backend name
    pub fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    /// Store a secret
    pub async fn set(&self, name: &str, value: &str, scope: &SecretScope) -> Result<()> {
        self.backend.set(name, value, scope).await
    }

    /// Retrieve a secret
    pub async fn get(&self, name: &str, scope: &SecretScope) -> Result<Option<String>> {
        self.backend.get(name, scope).await
    }

    /// Delete a secret
    pub async fn delete(&self, name: &str, scope: &SecretScope) -> Result<bool> {
        self.backend.delete(name, scope).await
    }

    /// List secret names for a scope
    pub async fn list(&self, scope: &SecretScope) -> Result<Vec<String>> {
        self.backend.list(scope).await
    }

    /// Get a secret, falling back through scopes
    ///
    /// Checks in order: Repository -> Namespace -> Global
    pub async fn get_with_fallback(
        &self,
        name: &str,
        repo: Option<&str>,
        namespace: Option<&str>,
    ) -> Result<Option<String>> {
        // Try repository scope first
        if let Some(repo_path) = repo {
            if let Some(value) = self.get(name, &SecretScope::Repository(repo_path.to_string())).await? {
                return Ok(Some(value));
            }
        }

        // Try namespace scope
        if let Some(ns) = namespace {
            if let Some(value) = self.get(name, &SecretScope::Namespace(ns.to_string())).await? {
                return Ok(Some(value));
            }
        }

        // Fall back to global
        self.get(name, &SecretScope::Global).await
    }

    /// Check backend health
    pub async fn health_check(&self) -> Result<()> {
        self.backend.health_check().await
    }
}

impl Default for SecretsManager {
    fn default() -> Self {
        Self::new()
    }
}

// Legacy module-level functions for backwards compatibility
// These use the default keyring backend

/// Store a secret in the OS keychain
pub fn set(name: &str, value: &str, scope: &SecretScope) -> Result<()> {
    let key = scope.to_key(name);
    let entry = keyring::Entry::new(SERVICE_NAME, &key)?;
    entry.set_password(value)?;
    tracing::debug!("Stored secret: {}", key);
    Ok(())
}

/// Retrieve a secret from the OS keychain
pub fn get(name: &str, scope: &SecretScope) -> Result<Option<String>> {
    let key = scope.to_key(name);
    let entry = keyring::Entry::new(SERVICE_NAME, &key)?;

    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Delete a secret from the OS keychain
pub fn delete(name: &str, scope: &SecretScope) -> Result<bool> {
    let key = scope.to_key(name);
    let entry = keyring::Entry::new(SERVICE_NAME, &key)?;

    match entry.delete_password() {
        Ok(()) => {
            tracing::debug!("Deleted secret: {}", key);
            Ok(true)
        }
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(anyhow::anyhow!("Keyring error: {}", e)),
    }
}

/// List all secret names (not values) for a scope
///
/// Note: This is a best-effort list based on our naming convention.
/// The OS keychain APIs don't always support enumeration.
pub fn list(_scope: &SecretScope) -> Result<Vec<String>> {
    // Unfortunately, keyring crate doesn't support listing
    // We would need platform-specific code to enumerate secrets
    // For now, we'll track names in a separate storage

    tracing::warn!("Secret listing not yet implemented");
    Ok(vec![])
}

/// Get a secret, falling back through scopes
///
/// Checks in order: Repository -> Namespace -> Global
pub fn get_with_fallback(name: &str, repo: Option<&str>, namespace: Option<&str>) -> Result<Option<String>> {
    // Try repository scope first
    if let Some(repo_path) = repo {
        if let Some(value) = get(name, &SecretScope::Repository(repo_path.to_string()))? {
            return Ok(Some(value));
        }
    }

    // Try namespace scope
    if let Some(ns) = namespace {
        if let Some(value) = get(name, &SecretScope::Namespace(ns.to_string()))? {
            return Ok(Some(value));
        }
    }

    // Fall back to global
    get(name, &SecretScope::Global)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_parse() {
        assert_eq!(SecretScope::parse("global").unwrap(), SecretScope::Global);
        assert_eq!(
            SecretScope::parse("namespace:backend").unwrap(),
            SecretScope::Namespace("backend".to_string())
        );
        assert_eq!(
            SecretScope::parse("repo:/path/to/repo").unwrap(),
            SecretScope::Repository("/path/to/repo".to_string())
        );
    }

    #[test]
    fn test_scope_to_key() {
        assert_eq!(
            SecretScope::Global.to_key("api_key"),
            "global:api_key"
        );
        assert_eq!(
            SecretScope::Namespace("backend".to_string()).to_key("api_key"),
            "namespace:backend:api_key"
        );
    }
}
