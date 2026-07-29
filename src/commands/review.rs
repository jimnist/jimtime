use super::Command;
use anyhow::Result;
use clap::Args;
use std::collections::BTreeMap;

use crate::daterange::RangeArgs;
use crate::selection::FilterArgs;
use crate::store::Day;
use crate::view::fmt_hours;

/// List entries over a date range or a single day
#[derive(Args)]
pub struct Review {
    #[command(flatten)]
    range: RangeArgs,
    #[command(flatten)]
    filter: FilterArgs,
    /// Only show unapproved entries
    #[arg(long)]
    pending: bool,
}

struct Row {
    id: String,
    hours: f64,
    billable: bool,
    approved: bool,
    needs_review: bool,
    imported: bool,
    notes: String,
}

impl Row {
    fn marker(&self) -> char {
        if self.approved { '○' } else { '●' }
    }
    fn eligible(&self) -> bool {
        self.approved && self.billable && !self.imported
    }
    fn flags(&self) -> String {
        let mut s = String::new();
        if self.needs_review {
            s.push_str("  [needs review]");
        }
        if self.imported {
            s.push_str("  [imported]");
        }
        s
    }
}

#[async_trait::async_trait]
impl Command for Review {
    async fn run(&self) -> Result<()> {
        let mut groups: BTreeMap<(String, String, String), Vec<Row>> = BTreeMap::new();

        for date in &self.range.dates()? {
            let Some(day) = Day::load(date)? else { continue };
            for s in &day.sections {
                if !self.filter.matches(s) {
                    continue;
                }
                for e in &s.entries {
                    if self.pending && e.approved {
                        continue;
                    }
                    groups
                        .entry((
                            s.client_name.clone(),
                            s.project_name.clone(),
                            s.task_name.clone(),
                        ))
                        .or_default()
                        .push(Row {
                            id: e.id.clone(),
                            hours: e.hours,
                            billable: e.billable,
                            approved: e.approved,
                            needs_review: e.needs_review,
                            imported: e.harvest_time_entry_id.is_some(),
                            notes: e.notes.clone(),
                        });
                }
            }
        }

        let scope = if self.pending { " (pending only)" } else { "" };
        println!("Review: {}{}\n", self.range.label()?, scope);
        if groups.is_empty() {
            println!("  (no entries)");
            return Ok(());
        }

        let (mut g_total, mut g_billable, mut g_eligible) = (0.0, 0.0, 0usize);
        for ((client, project, task), rows) in &groups {
            println!("{client} — {project} — {task}");
            let mut total = 0.0;
            let mut billable = 0.0;
            let (mut unapproved, mut needs_review, mut eligible) = (0usize, 0usize, 0usize);
            for r in rows {
                println!("  {}", r.id);
                let bill = if r.billable { "billable" } else { "non-bill" };
                println!(
                    "    {} {:>6}h  {:<8}  {}{}",
                    r.marker(),
                    fmt_hours(r.hours),
                    bill,
                    r.notes,
                    r.flags()
                );
                total += r.hours;
                if r.billable {
                    billable += r.hours;
                }
                if !r.approved {
                    unapproved += 1;
                }
                if r.needs_review {
                    needs_review += 1;
                }
                if r.eligible() {
                    eligible += 1;
                }
            }
            println!(
                "  Total: {}h · {} unapproved · {} needs-review · {} eligible to push\n",
                fmt_hours(total),
                unapproved,
                needs_review,
                eligible
            );
            g_total += total;
            g_billable += billable;
            g_eligible += eligible;
        }

        println!(
            "Totals: {}h ({}h billable) · {} eligible to push",
            fmt_hours(g_total),
            fmt_hours(g_billable),
            g_eligible
        );
        Ok(())
    }
}
