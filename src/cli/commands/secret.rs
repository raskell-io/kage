//! Secret command implementations

use anyhow::Result;

use crate::cli::SecretCommands;

pub async fn run(cmd: SecretCommands) -> Result<()> {
    match cmd {
        SecretCommands::Set { name, scope } => {
            tracing::info!("Setting secret {} in scope {}", name, scope);
            // TODO: Implement secret set (prompt for value, store in keychain)
            todo!("Secret set not yet implemented")
        }
        SecretCommands::List { scope } => {
            tracing::info!("Listing secrets");
            // TODO: Implement secret list
            todo!("Secret list not yet implemented")
        }
        SecretCommands::Delete { name, scope } => {
            tracing::info!("Deleting secret {} from scope {}", name, scope);
            // TODO: Implement secret delete
            todo!("Secret delete not yet implemented")
        }
    }
}
