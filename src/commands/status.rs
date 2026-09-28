use super::Command;
use anyhow::Result;
use clap::Args;

use crate::config::Config;
use crate::paths;
use crate::repo;
use crate::timeutil;

/// Show the current repo, its mapping, and today's store path
#[derive(Args)]
pub struct Status {}

#[async_trait::async_trait]
impl Command for Status {
    async fn run(&self) -> Result<()> {
        let repo = repo::current_repo()?;
        let config = Config::load()?;
        let m = config.for_repo(&repo)?;
        let date = timeutil::today()?;

        println!("Current repo:     {}", repo.display());
        println!("Mapped client:    {}", m.client.name);
        println!("Mapped project:   {}", m.project.name);
        println!(
            "Default task:     {}",
            config.task(&m.project.default_task)?.name
        );
        println!(
            "Billable default: {}",
            if m.project.billable { "yes" } else { "no" }
        );
        println!("Billing timezone: {}", timeutil::billing_tz()?);
        println!("Today's store:    {}", paths::day_file(&date)?.display());
        println!(
            "Harvest:          {}",
            if config.harvest.enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
        println!("Data repo:        {}", super::data::describe_state()?);
        Ok(())
    }
}
