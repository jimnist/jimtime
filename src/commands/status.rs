use super::Command;
use anyhow::Result;
use clap::Args;

use crate::mapping::Mappings;
use crate::paths;
use crate::repo;
use crate::timeutil;

/// Show the current repo, its Harvest mapping, and today's store path
#[derive(Args)]
pub struct Status {}

#[async_trait::async_trait]
impl Command for Status {
    async fn run(&self) -> Result<()> {
        let repo = repo::current_repo()?;
        let mappings = Mappings::load()?;
        let m = mappings.for_repo(&repo)?;
        let date = timeutil::today()?;

        println!("Current repo:     {}", repo.display());
        println!("Mapped client:    {}", m.client_name);
        println!("Mapped project:   {}", m.project_name);
        println!("Default task:     {}", m.default_task_name);
        println!(
            "Billable default: {}",
            if m.billable { "yes" } else { "no" }
        );
        println!("Billing timezone: {}", timeutil::billing_tz()?);
        println!("Today's store:    {}", paths::day_file(&date)?.display());
        Ok(())
    }
}
