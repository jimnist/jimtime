use super::Command;
use anyhow::Result;
use clap::Args;
use std::collections::HashSet;

use crate::daterange::RangeArgs;
use crate::datarepo::Sync;
use crate::selection::FilterArgs;
use crate::store::Day;
use crate::view::fmt_hours;

/// Set matching entries back to unapproved
///
/// Unapproves every approved entry in scope, except any passed to `--except`.
/// Or take back just specific entries with `--only <id>`. Skips entries already
/// pushed to Harvest - those can't be unapproved.
#[derive(Args)]
pub struct Unapprove {
    #[command(flatten)]
    range: RangeArgs,
    #[command(flatten)]
    filter: FilterArgs,
    /// Unapprove only these entry IDs (repeatable)
    #[arg(long)]
    only: Vec<String>,
    /// Entry IDs to leave alone (repeatable)
    #[arg(long)]
    except: Vec<String>,
}

#[async_trait::async_trait]
impl Command for Unapprove {
    async fn run(&self) -> Result<()> {
        let mut changed_lines: Vec<(String, f64, String, String)> = Vec::new();
        let mut skipped_imported = 0usize;
        let mut skipped_invoiced = 0usize;
        let mut seen: HashSet<String> = HashSet::new();
        let only_mode = !self.only.is_empty();

        let sync = Sync::begin("unapprove", false)?;
        for date in &self.range.dates()? {
            let Some(mut day) = Day::load(date)? else {
                continue;
            };
            let mut day_changed = false;
            for s in &mut day.sections {
                if !self.filter.matches(s) {
                    continue;
                }
                let label = s.label();
                for e in &mut s.entries {
                    seen.insert(e.id.clone());
                    if !e.approved || self.except.contains(&e.id) {
                        continue;
                    }
                    // In --only mode, act on exactly those ids and nothing else.
                    if only_mode && !self.only.contains(&e.id) {
                        continue;
                    }
                    // Already pushed to Harvest - can't be unapproved.
                    if e.harvest_time_entry_id.is_some() {
                        skipped_imported += 1;
                        continue;
                    }
                    // Billed on an invoice - void the invoice first.
                    if e.invoice.is_some() {
                        skipped_invoiced += 1;
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

        sync.commit(&format!(
            "unapprove: {} {} entr{}",
            self.range.label()?,
            changed_lines.len(),
            if changed_lines.len() == 1 { "y" } else { "ies" }
        ))?;

        // A billing gate: an id that matched nothing is almost certainly a typo.
        for id in self.only.iter().chain(self.except.iter()) {
            if !seen.contains(id) {
                eprintln!("warning: id {id} matched no entry in scope");
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
        if skipped_invoiced > 0 {
            println!(
                "\nLeft {skipped_invoiced} invoiced entr{} approved (void the invoice first: jimtime invoice void <number>).",
                if skipped_invoiced == 1 { "y" } else { "ies" }
            );
        }
        Ok(())
    }
}
