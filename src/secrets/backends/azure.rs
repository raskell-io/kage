//! Azure Key Vault backend for secrets storage
//!
//! Stores secrets in Azure Key Vault.
//! Secret name format: {prefix}{scope}-{name}
//! (Azure Key Vault names can only contain alphanumeric and dashes)

use anyhow::{Context, Result};
use async_trait::async_trait;

use azure_identity::DefaultAzureCredential;
use azure_security_keyvault_secrets::SecretClient;

use crate::secrets::backend::SecretsBackend;
use crate::secrets::SecretScope;

/// Azure Key Vault backend
pub struct AzureKeyVaultBackend {
    client: SecretClient,
    prefix: String,
    fallback: Option<super::KeyringBackend>,
}

impl AzureKeyVaultBackend {
    /// Create a new Azure Key Vault backend
    ///
    /// Uses DefaultAzureCredential (env vars, managed identity, Azure CLI, etc.)
    pub async fn new(
        vault_url: String,
        prefix: String,
        fallback_to_keyring: bool,
    ) -> Result<Self> {
        let credential = DefaultAzureCredential::default();
        let client = SecretClient::new(&vault_url, std::sync::Arc::new(credential))?;

        let fallback = if fallback_to_keyring {
            Some(super::KeyringBackend::new())
        } else {
            None
        };

        Ok(Self {
            client,
            prefix,
            fallback,
        })
    }

    /// Get the Azure secret name for a scope and name
    ///
    /// Azure Key Vault names can only contain alphanumeric characters and dashes.
    fn secret_name(&self, scope: &SecretScope, name: &str) -> String {
        let scope_str = match scope {
            SecretScope::Global => "global".to_string(),
            SecretScope::Namespace(ns) => format!("ns-{}", sanitize_for_azure(ns)),
            SecretScope::Repository(path) => format!("repo-{}", sanitize_for_azure(path)),
        };
        format!("{}{}-{}", self.prefix, scope_str, sanitize_for_azure(name))
    }
}

/// Sanitize a string for Azure Key Vault (only alphanumeric and dashes allowed)
fn sanitize_for_azure(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[async_trait]
impl SecretsBackend for AzureKeyVaultBackend {
    async fn set(&self, name: &str, value: &str, scope: &SecretScope) -> Result<()> {
        let secret_name = self.secret_name(scope, name);

        match self.client.set(&secret_name, value).await {
            Ok(_) => {
                tracing::debug!("Stored secret in Azure Key Vault: {}", secret_name);
                Ok(())
            }
            Err(e) => {
                if let Some(ref fallback) = self.fallback {
                    tracing::warn!("Azure Key Vault failed, falling back to keyring: {}", e);
                    return fallback.set(name, value, scope).await;
                }
                Err(e.into())
            }
        }
    }

    async fn get(&self, name: &str, scope: &SecretScope) -> Result<Option<String>> {
        let secret_name = self.secret_name(scope, name);

        match self.client.get(&secret_name).await {
            Ok(response) => {
                Ok(Some(response.value.to_string()))
            }
            Err(e) if e.to_string().contains("SecretNotFound") || e.to_string().contains("404") => {
                // Try fallback if configured
                if let Some(ref fallback) = self.fallback {
                    return fallback.get(name, scope).await;
                }
                Ok(None)
            }
            Err(e) => {
                if let Some(ref fallback) = self.fallback {
                    tracing::warn!("Azure Key Vault failed, falling back to keyring: {}", e);
                    return fallback.get(name, scope).await;
                }
                Err(e.into())
            }
        }
    }

    async fn delete(&self, name: &str, scope: &SecretScope) -> Result<bool> {
        let secret_name = self.secret_name(scope, name);

        match self.client.delete(&secret_name).await {
            Ok(_) => {
                // Azure Key Vault uses soft-delete by default
                // We should also purge to permanently delete
                let _ = self.client.purge_deleted(&secret_name).await;
                tracing::debug!("Deleted secret from Azure Key Vault: {}", secret_name);
                Ok(true)
            }
            Err(e) if e.to_string().contains("SecretNotFound") || e.to_string().contains("404") => {
                // Also try to delete from fallback
                if let Some(ref fallback) = self.fallback {
                    return fallback.delete(name, scope).await;
                }
                Ok(false)
            }
            Err(e) => {
                if let Some(ref fallback) = self.fallback {
                    tracing::warn!("Azure Key Vault failed, trying keyring: {}", e);
                    return fallback.delete(name, scope).await;
                }
                Err(e.into())
            }
        }
    }

    async fn list(&self, scope: &SecretScope) -> Result<Vec<String>> {
        let prefix = self.secret_name(scope, "");
        let mut names = Vec::new();

        // List all secrets and filter by prefix
        let secrets = self.client.list_secrets().await
            .context("Failed to list secrets from Azure Key Vault")?;

        for secret in secrets {
            if let Some(id) = secret.id {
                // Extract secret name from the ID URL
                if let Some(name) = id.rsplit('/').next() {
                    if name.starts_with(&prefix) {
                        // Remove prefix to get short name
                        if let Some(short_name) = name.strip_prefix(&prefix) {
                            names.push(short_name.to_string());
                        }
                    }
                }
            }
        }

        Ok(names)
    }

    async fn health_check(&self) -> Result<()> {
        // Try to list secrets (will fail if no access)
        self.client.list_secrets().await
            .context("Azure Key Vault health check failed")?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        "azure_key_vault"
    }
}
