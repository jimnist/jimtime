use super::Command;
use anyhow::Result;
use clap::Args;
use std::collections::HashSet;

use crate::daterange::RangeArgs;
use crate::selection::FilterArgs;
use crate::store::Day;
use crate::view::fmt_hours;

/// Set matching entries back to unapproved
///
/// Skips entries already pushed to Harvest - those can't be unapproved.
#[derive(Args)]
pub struct Unapprove {
    #[command(flatten)]
    range: RangeArgs,
    #[command(flatten)]
    filter: FilterArgs,
    /// Entry IDs to leave alone (repeatable)
    #[arg(long)]
    except: Vec<String>,
}

#[async_trait::async_trait]
impl Command for Unapprove {
    async fn run(&self) -> Result<()> {
        let mut changed_lines: Vec<(String, f64, String, String)> = Vec::new();
        let mut skipped_imported = 0usize;
        let mut seen: HashSet<String> = HashSet::new();

        for date in &self.range.dates()? {
            let Some(mut day) = Day::load(date)? else {
                continue;
            };
            let mut day_changed = false;
            for s in &mut day.sections {
                if !self.filter.matches(s) {
                    continue;
                }
                let label = format!("{} — {} — {}", s.client_name, s.project_name, s.task_name);
                for e in &mut s.entries {
                    seen.insert(e.id.clone());
                    if !e.approved || self.except.contains(&e.id) {
                        continue;
                    }
                    // Already pushed to Harvest - can't be unapproved.
                    if e.harvest_time_entry_id.is_some() {
                        skipped_imported += 1;
                        continue;
                    }
                    e.approved = false;
                    changed_lines.push((date.clone(), e.hours, label.clone(), e.id.clone()));
                    day_changed = true;
                }
            }
            if day_changed {
                day.save()?;
            }
        }

        for id in &self.except {
            if !seen.contains(id) {
                eprintln!("warning: --except {id} matched no entry in scope");
            }
        }

        if changed_lines.is_empty() {
            println!("Nothing to unapprove for {}.", self.range.label()?);
        } else {
            println!(
                "Unapproved {} entr{}:",
                changed_lines.len(),
                if changed_lines.len() == 1 { "y" } else { "ies" }
            );
            for (date, hours, label, id) in &changed_lines {
                println!("  {date}  {}h  {label}  ({id})", fmt_hours(*hours));
            }
        }
        if skipped_imported > 0 {
            println!(
                "\nLeft {skipped_imported} already-pushed entr{} approved (can't unapprove imported entries).",
                if skipped_imported == 1 { "y" } else { "ies" }
            );
        }
        Ok(())
    }
}
