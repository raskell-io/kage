//! Namespace command implementations

use anyhow::Result;

use crate::cli::NamespaceCommands;

pub async fn run(cmd: NamespaceCommands) -> Result<()> {
    match cmd {
        NamespaceCommands::Create { name } => {
            tracing::info!("Creating namespace: {}", name);
            // TODO: Implement namespace create
            todo!("Namespace create not yet implemented")
        }
        NamespaceCommands::List => {
            tracing::info!("Listing namespaces");
            // TODO: Implement namespace list
            todo!("Namespace list not yet implemented")
        }
        NamespaceCommands::Show { name } => {
            tracing::info!("Showing namespace: {}", name);
            // TODO: Implement namespace show
            todo!("Namespace show not yet implemented")
        }
        NamespaceCommands::AddRepo {
            namespace,
            path,
            name,
        } => {
            tracing::info!("Adding repo to namespace {}", namespace);
            // TODO: Implement namespace add-repo
            todo!("Namespace add-repo not yet implemented")
        }
        NamespaceCommands::RemoveRepo { namespace, repo } => {
            tracing::info!("Removing repo {} from namespace {}", repo, namespace);
            // TODO: Implement namespace remove-repo
            todo!("Namespace remove-repo not yet implemented")
        }
        NamespaceCommands::Delete { name, force } => {
            tracing::info!("Deleting namespace: {}", name);
            // TODO: Implement namespace delete
            todo!("Namespace delete not yet implemented")
        }
    }
}
