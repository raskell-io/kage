//! Memory storage backends
//!
//! - `filesystem` - Local append-only logs (default)
//! - `s3` - AWS S3 (requires `storage-s3` feature)
//! - `azure` - Azure Blob Storage (requires `storage-azure` feature)

mod filesystem;

#[cfg(feature = "storage-s3")]
mod s3;

#[cfg(feature = "storage-azure")]
mod azure;

pub use filesystem::FilesystemBackend;

#[cfg(feature = "storage-s3")]
pub use s3::S3Backend;

#[cfg(feature = "storage-azure")]
pub use azure::AzureBlobBackend;
