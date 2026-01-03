//! OS keychain backend for secrets storage
//!
//! Uses the OS-native secrets storage:
//! - macOS: Keychain
//! - Linux: Secret Service (GNOME Keyring, KWallet)
//! - Windows: Credential Manager

use anyhow::Result;
use async_trait::async_trait;

use crate::secrets::backend::SecretsBackend;
use crate::secrets::SecretScope;

const SERVICE_NAME: &str = "kage";

/// OS keychain-based secrets backend
pub struct KeyringBackend;

impl KeyringBackend {
    /// Create a new keyring backend
    pub fn new() -> Self {
        Self
    }

    /// Convert scope to keychain key
    fn scope_to_key(scope: &SecretScope, name: &str) -> String {
        match scope {
            SecretScope::Global => format!("global:{}", name),
            SecretScope::Namespace(ns) => format!("namespace:{}:{}", ns, name),
            SecretScope::Repository(path) => format!("repo:{}:{}", path, name),
        }
    }
}

impl Default for KeyringBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SecretsBackend for KeyringBackend {
    async fn set(&self, name: &str, value: &str, scope: &SecretScope) -> Result<()> {
        let key = Self::scope_to_key(scope, name);
        let entry = keyring::Entry::new(SERVICE_NAME, &key)?;
        entry.set_password(value)?;
        tracing::debug!("Stored secret in keyring: {}", key);
        Ok(())
    }

    async fn get(&self, name: &str, scope: &SecretScope) -> Result<Option<String>> {
        let key = Self::scope_to_key(scope, name);
        let entry = keyring::Entry::new(SERVICE_NAME, &key)?;

        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn delete(&self, name: &str, scope: &SecretScope) -> Result<bool> {
        let key = Self::scope_to_key(scope, name);
        let entry = keyring::Entry::new(SERVICE_NAME, &key)?;

        match entry.delete_password() {
            Ok(()) => {
                tracing::debug!("Deleted secret from keyring: {}", key);
                Ok(true)
            }
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(anyhow::anyhow!("Keyring error: {}", e)),
        }
    }

    async fn list(&self, _scope: &SecretScope) -> Result<Vec<String>> {
        // Keyring crate doesn't support listing
        // Would need platform-specific code
        tracing::warn!("Secret listing not supported for keyring backend");
        Ok(vec![])
    }

    async fn health_check(&self) -> Result<()> {
        // Try to access keyring
        let test_key = "__kage_health_check__";
        let entry = keyring::Entry::new(SERVICE_NAME, test_key)?;

        // Just check we can create an entry (don't actually write)
        match entry.get_password() {
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("Keyring health check failed: {}", e)),
        }
    }

    fn name(&self) -> &'static str {
        "keyring"
    }
}
