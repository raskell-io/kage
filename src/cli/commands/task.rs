//! Task command implementations

use anyhow::Result;

use crate::cli::TaskCommands;

pub async fn run(cmd: TaskCommands) -> Result<()> {
    match cmd {
        TaskCommands::Add {
            goal,
            namespace,
            repo,
            max_iterations,
            checkpoint_every,
            approval,
        } => {
            tracing::info!("Adding task: {}", goal);
            // TODO: Implement task add
            todo!("Task add not yet implemented")
        }
        TaskCommands::List { status, json } => {
            tracing::info!("Listing tasks");
            // TODO: Implement task list
            todo!("Task list not yet implemented")
        }
        TaskCommands::Resume {
            id,
            extend_iterations,
            guidance,
        } => {
            tracing::info!("Resuming task {}", id);
            // TODO: Implement task resume
            todo!("Task resume not yet implemented")
        }
        TaskCommands::Show { id } => {
            tracing::info!("Showing task {}", id);
            // TODO: Implement task show
            todo!("Task show not yet implemented")
        }
        TaskCommands::Cancel { id } => {
            tracing::info!("Cancelling task {}", id);
            // TODO: Implement task cancel
            todo!("Task cancel not yet implemented")
        }
    }
}
