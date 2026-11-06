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

impl Command for Status {
    async fn run(&self) -> Result<()> {
        let repo = repo::current_repo()?;
        let config = Config::load()?;
        let m = config.for_repo(&repo)?;
        let date = timeutil::today()?;

        outln!("Current repo:     {}", repo.display());
        outln!("Mapped client:    {}", m.client.name);
        outln!("Mapped project:   {}", m.project.name);
        outln!(
            "Default task:     {}",
            config.task(&m.project.default_task)?.name
        );
        outln!(
            "Billable default: {}",
            if m.project.billable { "yes" } else { "no" }
        );
        outln!("Billing timezone: {}", timeutil::billing_tz()?);
        outln!("Today's store:    {}", paths::day_file(&date)?.display());
        outln!(
            "Harvest:          {}",
            if config.harvest.enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
        outln!("Data repo:        {}", super::data::describe_state()?);
        Ok(())
    }
}
