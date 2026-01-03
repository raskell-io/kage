//! Secrets storage backends
//!
//! - `keyring` - OS keychain (default)
//! - `aws` - AWS Secrets Manager (requires `secrets-aws` feature)
//! - `azure` - Azure Key Vault (requires `secrets-azure` feature)

mod keyring;

#[cfg(feature = "secrets-aws")]
mod aws;

#[cfg(feature = "secrets-azure")]
mod azure;

pub use keyring::KeyringBackend;

#[cfg(feature = "secrets-aws")]
pub use aws::AwsSecretsManagerBackend;

#[cfg(feature = "secrets-azure")]
pub use azure::AzureKeyVaultBackend;
