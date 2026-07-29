use super::Command;
use anyhow::Result;
use clap::Args;

use crate::mapping::Mappings;
use crate::repo;

/// Show the Harvest mapping for the current repo
#[derive(Args)]
pub struct Map {}

#[async_trait::async_trait]
impl Command for Map {
    async fn run(&self) -> Result<()> {
        let repo = repo::current_repo()?;
        let mappings = Mappings::load()?;
        let m = mappings.for_repo(&repo)?;

        println!("Repo:             {}", repo.display());
        println!(
            "Client:           {} (id {})",
            m.client_name, m.client_id
        );
        println!(
            "Project:          {} (id {})",
            m.project_name, m.project_id
        );
        println!(
            "Default task:     {} (id {})",
            m.default_task_name, m.default_task_id
        );
        println!(
            "Billable default: {}",
            if m.billable { "yes" } else { "no" }
        );
        Ok(())
    }
}
