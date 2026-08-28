use super::Command;
use anyhow::{Result, bail};
use clap::{Args, ValueEnum};

use crate::mapping::Mappings;
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

    /// Task alias defined in the mapping config (e.g. development, meetings)
    #[arg(long)]
    task: Option<String>,

    /// Explicit Harvest task id (overrides --task and the mapping default)
    #[arg(long)]
    task_id: Option<u64>,

    /// Display name for --task-id (defaults to "Task <id>")
    #[arg(long)]
    task_name: Option<String>,

    /// Override the mapping's billable default
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
        let mappings = Mappings::load()?;
        let m = mappings.for_repo(&repo)?.clone();

        let (task_id, task_name) = self.resolve_task(&mappings, &m)?;

        let billable = match self.billable {
            Some(Billable::Yes) => true,
            Some(Billable::No) => false,
            None => m.billable,
        };

        let proto = Section {
            repo_path: repo.display().to_string(),
            client_id: m.client_id,
            client_name: m.client_name.clone(),
            project_id: m.project_id,
            project_name: m.project_name.clone(),
            task_id,
            task_name,
            approved: false,
            entries: Vec::new(),
        };

        let mut day = Day::load_or_new(&date)?;
        let id = day.add_entry(
            proto.clone(),
            hours,
            billable,
            self.needs_review,
            self.notes.clone(),
        );
        day.save()?;

        println!(
            "Added {}h to {} - {} - {} on {}{}",
            fmt_hours(hours),
            proto.client_name,
            proto.project_name,
            proto.task_name,
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

    fn resolve_task(
        &self,
        mappings: &Mappings,
        m: &crate::mapping::RepoMapping,
    ) -> Result<(u64, String)> {
        if let Some(id) = self.task_id {
            let name = self
                .task_name
                .clone()
                .unwrap_or_else(|| format!("Task {id}"));
            return Ok((id, name));
        }
        if let Some(alias) = &self.task {
            let a = mappings.alias(alias)?;
            return Ok((a.task_id, a.task_name.clone()));
        }
        Ok((m.default_task_id, m.default_task_name.clone()))
    }
}
