use super::Command;
use anyhow::Result;
use clap::Args;
use std::collections::BTreeMap;

use crate::daterange::RangeArgs;
use crate::selection::FilterArgs;
use crate::store::Day;
use crate::view::fmt_hours;

/// Export a markdown time report over a date range or a single day
#[derive(Args)]
pub struct Report {
    #[command(flatten)]
    range: RangeArgs,
    #[command(flatten)]
    filter: FilterArgs,
    /// Only include billable entries
    #[arg(long)]
    billable_only: bool,
}

struct Row {
    date: String,
    hours: f64,
    billable: bool,
    notes: String,
}

#[async_trait::async_trait]
impl Command for Report {
    async fn run(&self) -> Result<()> {
        let mut groups: BTreeMap<(String, String, String), Vec<Row>> = BTreeMap::new();

        for date in &self.range.dates()? {
            let Some(day) = Day::load(date)? else { continue };
            for s in &day.sections {
                if !self.filter.matches(s) {
                    continue;
                }
                for e in &s.entries {
                    if self.billable_only && !e.billable {
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
                            date: date.clone(),
                            hours: e.hours,
                            billable: e.billable,
                            notes: e.notes.clone(),
                        });
                }
            }
        }

        println!("# Time Report - {}\n", self.range.label()?);
        if groups.is_empty() {
            println!("_No entries._");
            return Ok(());
        }

        let (mut grand_total, mut grand_billable, mut grand_count) = (0.0, 0.0, 0usize);
        for ((client, project, task), rows) in &groups {
            println!("## {client} - {project} - {task}\n");
            println!("| Date | Hours | Billable | Notes |");
            println!("|---|---:|:---:|---|");
            let mut total = 0.0;
            let mut billable = 0.0;
            for r in rows {
                println!(
                    "| {} | {} | {} | {} |",
                    r.date,
                    fmt_hours(r.hours),
                    if r.billable { "yes" } else { "no" },
                    escape_cell(&r.notes)
                );
                total += r.hours;
                if r.billable {
                    billable += r.hours;
                }
            }
            println!(
                "\n**Subtotal: {}h ({}h billable)**\n",
                fmt_hours(total),
                fmt_hours(billable)
            );
            grand_total += total;
            grand_billable += billable;
            grand_count += rows.len();
        }

        println!("---\n");
        println!(
            "**Total: {}h across {} entr{} ({}h billable)**",
            fmt_hours(grand_total),
            grand_count,
            if grand_count == 1 { "y" } else { "ies" },
            fmt_hours(grand_billable)
        );
        Ok(())
    }
}

/// Make a note safe for one markdown table cell: escape `|` and flatten any
/// newlines, either of which would break the row.
fn escape_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}
