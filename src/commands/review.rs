use super::Command;
use anyhow::Result;
use clap::Args;
use std::collections::BTreeMap;

use crate::daterange::RangeArgs;
use crate::store::Day;
use crate::view::fmt_hours;

/// Summarize entries over a date range or a single day
#[derive(Args)]
pub struct Review {
    #[command(flatten)]
    range: RangeArgs,
}

#[derive(Default)]
struct Agg {
    total: f64,
    billable: f64,
    nonbillable: f64,
    approved: usize,
    unapproved: usize,
    needs_review: usize,
    imported: usize,
    eligible: usize,
}

impl Agg {
    fn add(&mut self, other: &Agg) {
        self.total += other.total;
        self.billable += other.billable;
        self.nonbillable += other.nonbillable;
        self.approved += other.approved;
        self.unapproved += other.unapproved;
        self.needs_review += other.needs_review;
        self.imported += other.imported;
        self.eligible += other.eligible;
    }
}

#[async_trait::async_trait]
impl Command for Review {
    async fn run(&self) -> Result<()> {
        let dates = self.range.dates()?;
        let mut groups: BTreeMap<(String, String, String), Agg> = BTreeMap::new();

        for date in &dates {
            let Some(day) = Day::load(date)? else { continue };
            for s in &day.sections {
                let key = (
                    s.client_name.clone(),
                    s.project_name.clone(),
                    s.task_name.clone(),
                );
                let a = groups.entry(key).or_default();
                for e in &s.entries {
                    a.total += e.hours;
                    if e.billable {
                        a.billable += e.hours;
                    } else {
                        a.nonbillable += e.hours;
                    }
                    if s.approved {
                        a.approved += 1;
                    } else {
                        a.unapproved += 1;
                    }
                    if e.needs_review {
                        a.needs_review += 1;
                    }
                    if e.harvest_time_entry_id.is_some() {
                        a.imported += 1;
                    }
                    if s.approved && e.billable && e.harvest_time_entry_id.is_none() {
                        a.eligible += 1;
                    }
                }
            }
        }

        println!("Review: {}\n", self.range.label()?);
        if groups.is_empty() {
            println!("  (no entries)");
            return Ok(());
        }

        let mut grand = Agg::default();
        for ((client, project, task), a) in &groups {
            println!("{client} — {project} — {task}");
            println!("  Total: {}h", fmt_hours(a.total));
            println!(
                "  Billable: {}h   Non-billable: {}h",
                fmt_hours(a.billable),
                fmt_hours(a.nonbillable)
            );
            println!("  Approved: {}   Unapproved: {}", a.approved, a.unapproved);
            println!("  Needs review: {}", a.needs_review);
            println!("  Already imported: {}", a.imported);
            println!("  Eligible to push: {}", a.eligible);
            println!();
            grand.add(a);
        }

        println!(
            "Totals: {}h ({}h billable) · {} eligible to push",
            fmt_hours(grand.total),
            fmt_hours(grand.billable),
            grand.eligible
        );
        Ok(())
    }
}
