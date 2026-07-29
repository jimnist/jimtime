use super::Command;
use anyhow::Result;
use clap::Args;

use crate::daterange::RangeArgs;
use crate::selection::FilterArgs;
use crate::store::Day;
use crate::view::fmt_hours;

/// Set matching sections back to unapproved
#[derive(Args)]
pub struct Unapprove {
    #[command(flatten)]
    range: RangeArgs,
    #[command(flatten)]
    filter: FilterArgs,
}

#[async_trait::async_trait]
impl Command for Unapprove {
    async fn run(&self) -> Result<()> {
        let dates = self.range.dates()?;
        let mut changed: Vec<(String, String, String, String, f64)> = Vec::new();

        for date in &dates {
            let Some(mut day) = Day::load(date)? else {
                continue;
            };
            let mut day_changed = false;
            for s in &mut day.sections {
                if s.approved && self.filter.matches(s) {
                    // Don't unapprove entries already pushed to Harvest.
                    if s.entries.iter().any(|e| e.harvest_time_entry_id.is_some()) {
                        continue;
                    }
                    s.approved = false;
                    let hours: f64 = s.entries.iter().map(|e| e.hours).sum();
                    changed.push((
                        date.clone(),
                        s.client_name.clone(),
                        s.project_name.clone(),
                        s.task_name.clone(),
                        hours,
                    ));
                    day_changed = true;
                }
            }
            if day_changed {
                day.save()?;
            }
        }

        if changed.is_empty() {
            println!("Nothing to unapprove for {}.", self.range.label()?);
            return Ok(());
        }
        println!("Unapproved {} section(s):\n", changed.len());
        for (date, client, project, task, hours) in &changed {
            println!("  {date}  {client} — {project} — {task}  ({}h)", fmt_hours(*hours));
        }
        Ok(())
    }
}
