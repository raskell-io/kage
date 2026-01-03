//! AWS Secrets Manager backend for secrets storage
//!
//! Stores secrets in AWS Secrets Manager with optional KMS encryption.
//! Secret name format: {prefix}{scope}/{name}

use anyhow::{Context, Result};
use async_trait::async_trait;

use aws_sdk_secretsmanager::Client;

use crate::secrets::backend::SecretsBackend;
use crate::secrets::SecretScope;

/// AWS Secrets Manager backend
pub struct AwsSecretsManagerBackend {
    client: Client,
    prefix: String,
    kms_key_id: Option<String>,
    fallback: Option<super::KeyringBackend>,
}

impl AwsSecretsManagerBackend {
    /// Create a new AWS Secrets Manager backend
    ///
    /// Uses the default AWS credential chain (env vars, IAM role, etc.)
    pub async fn new(
        region: Option<String>,
        prefix: String,
        kms_key_id: Option<String>,
        fallback_to_keyring: bool,
    ) -> Result<Self> {
        let mut config_loader = aws_config::from_env();

        if let Some(region) = region {
            config_loader = config_loader.region(aws_sdk_secretsmanager::config::Region::new(region));
        }

        let config = config_loader.load().await;
        let client = Client::new(&config);

        let fallback = if fallback_to_keyring {
            Some(super::KeyringBackend::new())
        } else {
            None
        };

        Ok(Self {
            client,
            prefix,
            kms_key_id,
            fallback,
        })
    }

    /// Get the AWS secret name for a scope and name
    fn secret_name(&self, scope: &SecretScope, name: &str) -> String {
        let scope_str = match scope {
            SecretScope::Global => "global".to_string(),
            SecretScope::Namespace(ns) => format!("namespace/{}", ns),
            SecretScope::Repository(path) => {
                // Sanitize path for AWS (replace / with -)
                format!("repo/{}", path.replace('/', "-"))
            }
        };
        format!("{}{}/{}", self.prefix, scope_str, name)
    }
}

#[async_trait]
impl SecretsBackend for AwsSecretsManagerBackend {
    async fn set(&self, name: &str, value: &str, scope: &SecretScope) -> Result<()> {
        let secret_name = self.secret_name(scope, name);

        // Try to create or update the secret
        let result = self.client
            .put_secret_value()
            .secret_id(&secret_name)
            .secret_string(value)
            .send()
            .await;

        match result {
            Ok(_) => {
                tracing::debug!("Updated secret in AWS: {}", secret_name);
                Ok(())
            }
            Err(e) if e.to_string().contains("ResourceNotFoundException") => {
                // Secret doesn't exist, create it
                let mut request = self.client
                    .create_secret()
                    .name(&secret_name)
                    .secret_string(value);

                if let Some(ref kms_key) = self.kms_key_id {
                    request = request.kms_key_id(kms_key);
                }

                request.send().await
                    .context("Failed to create secret in AWS Secrets Manager")?;

                tracing::debug!("Created secret in AWS: {}", secret_name);
                Ok(())
            }
            Err(e) => {
                if let Some(ref fallback) = self.fallback {
                    tracing::warn!("AWS Secrets Manager failed, falling back to keyring: {}", e);
                    return fallback.set(name, value, scope).await;
                }
                Err(e.into())
            }
        }
    }

    async fn get(&self, name: &str, scope: &SecretScope) -> Result<Option<String>> {
        let secret_name = self.secret_name(scope, name);

        match self.client
            .get_secret_value()
            .secret_id(&secret_name)
            .send()
            .await
        {
            Ok(response) => {
                Ok(response.secret_string().map(|s| s.to_string()))
            }
            Err(e) if e.to_string().contains("ResourceNotFoundException") => {
                // Try fallback if configured
                if let Some(ref fallback) = self.fallback {
                    return fallback.get(name, scope).await;
                }
                Ok(None)
            }
            Err(e) => {
                if let Some(ref fallback) = self.fallback {
                    tracing::warn!("AWS Secrets Manager failed, falling back to keyring: {}", e);
                    return fallback.get(name, scope).await;
                }
                Err(e.into())
            }
        }
    }

    async fn delete(&self, name: &str, scope: &SecretScope) -> Result<bool> {
        let secret_name = self.secret_name(scope, name);

        match self.client
            .delete_secret()
            .secret_id(&secret_name)
            .force_delete_without_recovery(true)
            .send()
            .await
        {
            Ok(_) => {
                tracing::debug!("Deleted secret from AWS: {}", secret_name);
                Ok(true)
            }
            Err(e) if e.to_string().contains("ResourceNotFoundException") => {
                // Also try to delete from fallback
                if let Some(ref fallback) = self.fallback {
                    return fallback.delete(name, scope).await;
                }
                Ok(false)
            }
            Err(e) => {
                if let Some(ref fallback) = self.fallback {
                    tracing::warn!("AWS Secrets Manager failed, trying keyring: {}", e);
                    return fallback.delete(name, scope).await;
                }
                Err(e.into())
            }
        }
    }

    async fn list(&self, scope: &SecretScope) -> Result<Vec<String>> {
        let prefix = self.secret_name(scope, "");
        let mut names = Vec::new();
        let mut next_token = None;

        loop {
            let mut request = self.client
                .list_secrets()
                .filters(
                    aws_sdk_secretsmanager::types::Filter::builder()
                        .key(aws_sdk_secretsmanager::types::FilterNameStringType::Name)
                        .values(&prefix)
                        .build(),
                );

            if let Some(token) = next_token {
                request = request.next_token(token);
            }

            let response = request.send().await
                .context("Failed to list secrets from AWS")?;

            if let Some(secrets) = response.secret_list {
                for secret in secrets {
                    if let Some(name) = secret.name {
                        // Extract the secret name after the prefix
                        if let Some(short_name) = name.strip_prefix(&prefix) {
                            names.push(short_name.to_string());
                        }
                    }
                }
            }

            next_token = response.next_token;
            if next_token.is_none() {
                break;
            }
        }

        Ok(names)
    }

    async fn health_check(&self) -> Result<()> {
        // Try to list secrets (will fail if no access)
        self.client
            .list_secrets()
            .max_results(1)
            .send()
            .await
            .context("AWS Secrets Manager health check failed")?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        "aws_secrets_manager"
    }
}
