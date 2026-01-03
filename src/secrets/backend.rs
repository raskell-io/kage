//! Secrets backend trait for pluggable secrets management
//!
//! Supports OS keychain (default), AWS Secrets Manager, and Azure Key Vault.

use anyhow::Result;
use async_trait::async_trait;

use super::SecretScope;

/// Backend for secrets storage
///
/// Implementations must be thread-safe and async-compatible.
#[async_trait]
pub trait SecretsBackend: Send + Sync {
    /// Store a secret
    async fn set(&self, name: &str, value: &str, scope: &SecretScope) -> Result<()>;

    /// Retrieve a secret
    async fn get(&self, name: &str, scope: &SecretScope) -> Result<Option<String>>;

    /// Delete a secret
    async fn delete(&self, name: &str, scope: &SecretScope) -> Result<bool>;

    /// List secret names (not values) for a scope
    async fn list(&self, scope: &SecretScope) -> Result<Vec<String>>;

    /// Check if the backend is available and healthy
    async fn health_check(&self) -> Result<()>;

    /// Get the backend name for logging
    fn name(&self) -> &'static str;
}

/// Configuration for secrets backend selection
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SecretsBackendConfig {
    /// OS keychain (default)
    Keyring,

    /// AWS Secrets Manager
    #[cfg(feature = "secrets-aws")]
    AwsSecretsManager {
        /// AWS region (default: from AWS_REGION env var)
        #[serde(default)]
        region: Option<String>,
        /// Secret name prefix (default: "kage/")
        #[serde(default = "default_aws_prefix")]
        prefix: String,
        /// KMS key ID for encryption (optional, uses AWS managed key if not set)
        #[serde(default)]
        kms_key_id: Option<String>,
        /// Fall back to local keyring if AWS fails
        #[serde(default)]
        fallback_to_keyring: bool,
    },

    /// Azure Key Vault
    #[cfg(feature = "secrets-azure")]
    AzureKeyVault {
        /// Key Vault URL (e.g., "https://my-vault.vault.azure.net/")
        vault_url: String,
        /// Secret name prefix (default: "kage-")
        #[serde(default = "default_azure_prefix")]
        prefix: String,
        /// Fall back to local keyring if Azure fails
        #[serde(default)]
        fallback_to_keyring: bool,
    },
}

impl Default for SecretsBackendConfig {
    fn default() -> Self {
        Self::Keyring
    }
}

#[cfg(feature = "secrets-aws")]
fn default_aws_prefix() -> String {
    "kage/".to_string()
}

#[cfg(feature = "secrets-azure")]
fn default_azure_prefix() -> String {
    "kage-".to_string()
}

/// Create a secrets backend from configuration
pub async fn create_backend(
    config: &SecretsBackendConfig,
) -> Result<Box<dyn SecretsBackend>> {
    match config {
        SecretsBackendConfig::Keyring => {
            let backend = super::backends::KeyringBackend::new();
            Ok(Box::new(backend))
        }

        #[cfg(feature = "secrets-aws")]
        SecretsBackendConfig::AwsSecretsManager {
            region,
            prefix,
            kms_key_id,
            fallback_to_keyring,
        } => {
            let backend = super::backends::AwsSecretsManagerBackend::new(
                region.clone(),
                prefix.clone(),
                kms_key_id.clone(),
                *fallback_to_keyring,
            ).await?;
            Ok(Box::new(backend))
        }

        #[cfg(feature = "secrets-azure")]
        SecretsBackendConfig::AzureKeyVault {
            vault_url,
            prefix,
            fallback_to_keyring,
        } => {
            let backend = super::backends::AzureKeyVaultBackend::new(
                vault_url.clone(),
                prefix.clone(),
                *fallback_to_keyring,
            ).await?;
            Ok(Box::new(backend))
        }
    }
}
