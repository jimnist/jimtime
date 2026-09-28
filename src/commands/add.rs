use super::Command;
use anyhow::{Result, bail};
use clap::{Args, ValueEnum};

use crate::config::Config;
use crate::datarepo::Sync;
use crate::repo;
use crate::store::{Day, Section};
use crate::timeutil;
use crate::view::fmt_hours;

#[derive(Copy, Clone, ValueEnum)]
pub enum Billable {
    Yes,
    No,
}

/// Add a time entry for the current repo
#[derive(Args)]
pub struct Add {
    /// Decimal hours, e.g. 1.25
    #[arg(long)]
    hours: Option<f64>,

    /// Start time HH:MM (24-hour); use with --to instead of --hours
    #[arg(long)]
    from: Option<String>,

    /// End time HH:MM (24-hour); use with --from instead of --hours
    #[arg(long)]
    to: Option<String>,

    /// Date YYYY-MM-DD (default: today in the billing timezone)
    #[arg(long)]
    date: Option<String>,

    /// Task key from [tasks] in the config (default: the project's default_task)
    #[arg(long)]
    task: Option<String>,

    /// Override the project's billable default
    #[arg(long, value_enum)]
    billable: Option<Billable>,

    /// Flag the entry for review before it can be approved
    #[arg(long)]
    needs_review: bool,

    /// Invoice-friendly description of the work
    #[arg(long)]
    notes: String,
}

#[async_trait::async_trait]
impl Command for Add {
    async fn run(&self) -> Result<()> {
        let hours = self.resolve_hours()?;

        let date = match &self.date {
            Some(d) => timeutil::parse_date(d)?,
            None => timeutil::today()?,
        };

        let repo = repo::current_repo()?;
        let config = Config::load()?;
        let m = config.for_repo(&repo)?;

        let task_key = self.task.as_deref().unwrap_or(&m.project.default_task);
        let task = config.task(task_key)?;

        let billable = match self.billable {
            Some(Billable::Yes) => true,
            Some(Billable::No) => false,
            None => m.project.billable,
        };

        let proto = Section {
            repo_path: repo.display().to_string(),
            client: m.client_key.to_string(),
            client_name: m.client.name.clone(),
            project: m.project_key.to_string(),
            project_name: m.project.name.clone(),
            task: task_key.to_string(),
            task_name: task.name.clone(),
            harvest_client_id: config.harvest_client_id(m.client_key),
            harvest_project_id: config.harvest_project_id(m.client_key, m.project_key),
            harvest_task_id: config.harvest_task_id(task_key),
            ..Section::default()
        };

        let sync = Sync::begin("add", false)?;
        let mut day = Day::load_or_new(&date)?;
        let id = day.add_entry(
            proto.clone(),
            hours,
            billable,
            self.needs_review,
            self.notes.clone(),
        );
        day.save()?;
        sync.commit(&format!(
            "add: {date} {}h {}",
            fmt_hours(hours),
            proto.label()
        ))?;

        println!(
            "Added {}h to {} on {}{}",
            fmt_hours(hours),
            proto.label(),
            date,
            if self.needs_review {
                "  [needs review]"
            } else {
                ""
            }
        );
        println!("  {}  ({})", self.notes, id);
        Ok(())
    }
}

impl Add {
    fn resolve_hours(&self) -> Result<f64> {
        match (self.hours, &self.from, &self.to) {
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                bail!("use either --hours or --from/--to, not both")
            }
            (Some(h), None, None) => {
                if h <= 0.0 {
                    bail!("--hours must be positive");
                }
                Ok(h)
            }
            (None, Some(from), Some(to)) => timeutil::hours_between(from, to),
            (None, Some(_), None) | (None, None, Some(_)) => {
                bail!("--from and --to must be given together")
            }
            (None, None, None) => bail!("provide --hours, or both --from and --to"),
        }
    }
}
