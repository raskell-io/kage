//! User management command implementations (multi-user mode)

use anyhow::Result;

use crate::cli::UserCommands;

pub async fn run(cmd: UserCommands) -> Result<()> {
    match cmd {
        UserCommands::List => {
            tracing::info!("Listing users");
            todo!("User list not yet implemented")
        }
        UserCommands::Invite { email } => {
            tracing::info!("Inviting user: {}", email);
            todo!("User invite not yet implemented")
        }
        UserCommands::Remove { id } => {
            tracing::info!("Removing user: {}", id);
            todo!("User remove not yet implemented")
        }
    }
}
