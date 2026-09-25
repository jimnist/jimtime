use super::Command;
use anyhow::Result;
use clap::Args;

use crate::config::Config;
use crate::repo;

/// Show the client/project mapping for the current repo
#[derive(Args)]
pub struct Map {}

#[async_trait::async_trait]
impl Command for Map {
    async fn run(&self) -> Result<()> {
        let repo = repo::current_repo()?;
        let config = Config::load()?;
        let m = config.for_repo(&repo)?;
        let task_key = &m.project.default_task;
        let task = config.task(task_key)?;

        println!("Repo:             {}", repo.display());
        println!(
            "Client:           {} ({}){}",
            m.client.name,
            m.client_key,
            harvest(m.client.harvest_id)
        );
        println!(
            "Project:          {} ({}){}",
            m.project.name,
            m.project_key,
            harvest(m.project.harvest_id)
        );
        println!(
            "Default task:     {} ({}){}",
            task.name,
            task_key,
            harvest(task.harvest_id)
        );
        println!(
            "Billable default: {}",
            if m.project.billable { "yes" } else { "no" }
        );
        match config.rate(m.client_key, m.project_key, task_key) {
            Ok(r) => println!("Rate:             {r} {}/h", m.client.currency),
            Err(_) => println!("Rate:             (not set)"),
        }
        Ok(())
    }
}

fn harvest(id: Option<u64>) -> String {
    id.map(|i| format!(", Harvest id {i}")).unwrap_or_default()
}
