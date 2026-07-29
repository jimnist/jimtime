use super::Command;
use anyhow::Result;
use clap::Args;

use crate::daterange::RangeArgs;
use crate::selection::FilterArgs;
use crate::store::Day;
use crate::view::fmt_hours;

/// Approve matching sections (the human gate before pushing)
#[derive(Args)]
pub struct Approve {
    #[command(flatten)]
    range: RangeArgs,
    #[command(flatten)]
    filter: FilterArgs,
    /// Apply the change. Without it, prints what would be approved.
    #[arg(long)]
    yes: bool,
}

#[async_trait::async_trait]
impl Command for Approve {
    async fn run(&self) -> Result<()> {
        let dates = self.range.dates()?;
        let mut planned: Vec<(String, String, String, String, usize, f64)> = Vec::new();
        let mut saved_days = 0;

        for date in &dates {
            let Some(mut day) = Day::load(date)? else {
                continue;
            };
            let mut changed = false;
            for s in &mut day.sections {
                if !s.approved && self.filter.matches(s) {
                    let hours: f64 = s.entries.iter().map(|e| e.hours).sum();
                    planned.push((
                        date.clone(),
                        s.client_name.clone(),
                        s.project_name.clone(),
                        s.task_name.clone(),
                        s.entries.len(),
                        hours,
                    ));
                    if self.yes {
                        s.approved = true;
                        // Approval is the review gate; clear the needs-review flags.
                        for e in &mut s.entries {
                            e.needs_review = false;
                        }
                        changed = true;
                    }
                }
            }
            if self.yes && changed {
                day.save()?;
                saved_days += 1;
            }
        }

        if planned.is_empty() {
            println!("Nothing to approve for {}.", self.range.label()?);
            return Ok(());
        }

        let verb = if self.yes { "Approved" } else { "Would approve" };
        println!("{verb} {} section(s):\n", planned.len());
        for (date, client, project, task, count, hours) in &planned {
            println!(
                "  {date}  {client} — {project} — {task}  ({count} entr{}, {}h)",
                if *count == 1 { "y" } else { "ies" },
                fmt_hours(*hours)
            );
        }
        if self.yes {
            println!("\nUpdated {saved_days} day(s).");
        } else {
            println!("\nRe-run with --yes to approve.");
        }
        Ok(())
    }
}
