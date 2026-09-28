use super::Command;
use anyhow::Result;
use clap::Args;
use std::collections::BTreeMap;

use crate::config::Config;
use crate::daterange::RangeArgs;
use crate::selection::FilterArgs;
use crate::store::{Day, Entry};
use crate::view::{flags, fmt_hours, marker};

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

/// Per-group and overall counts.
#[derive(Default)]
struct Tally {
    total: f64,
    billable: f64,
    unapproved: usize,
    needs_review: usize,
    invoiceable: usize,
    pushable: usize,
}

impl Tally {
    fn add(&mut self, e: &Entry) {
        self.total += e.hours;
        if e.billable {
            self.billable += e.hours;
        }
        self.unapproved += usize::from(!e.approved);
        self.needs_review += usize::from(e.needs_review);
        self.invoiceable += usize::from(e.is_invoiceable());
        self.pushable += usize::from(e.is_pushable(false));
    }

    fn merge(&mut self, o: &Tally) {
        self.total += o.total;
        self.billable += o.billable;
        self.unapproved += o.unapproved;
        self.needs_review += o.needs_review;
        self.invoiceable += o.invoiceable;
        self.pushable += o.pushable;
    }

    /// The "ready for the next step" counts. Harvest's only shows when it is on.
    fn ready(&self, harvest: bool) -> String {
        let mut s = format!("{} ready to invoice", self.invoiceable);
        if harvest {
            s.push_str(&format!(" · {} ready to push", self.pushable));
        }
        s
    }
}

#[async_trait::async_trait]
impl Command for Review {
    async fn run(&self) -> Result<()> {
        let harvest = Config::load_optional()?.is_some_and(|c| c.harvest.enabled);
        let mut groups: BTreeMap<String, Vec<(String, Entry)>> = BTreeMap::new();

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
                        .entry(s.label())
                        .or_default()
                        .push((date.clone(), e.clone()));
                }
            }
        }

        let scope = if self.pending { " (pending only)" } else { "" };
        println!("Review: {}{}\n", self.range.label()?, scope);
        if groups.is_empty() {
            println!("  (no entries)");
            return Ok(());
        }

        let mut grand = Tally::default();
        for (label, rows) in &groups {
            println!("{label}");
            let mut t = Tally::default();
            for (_, e) in rows {
                println!("  {}", e.id);
                let bill = if e.billable { "billable" } else { "non-bill" };
                println!(
                    "    {} {:>6}h  {:<8}  {}{}",
                    marker(e),
                    fmt_hours(e.hours),
                    bill,
                    e.notes,
                    flags(e)
                );
                t.add(e);
            }
            println!(
                "  Total: {}h · {} unapproved · {} needs-review · {}\n",
                fmt_hours(t.total),
                t.unapproved,
                t.needs_review,
                t.ready(harvest)
            );
            grand.merge(&t);
        }

        println!(
            "Totals: {}h ({}h billable) · {}",
            fmt_hours(grand.total),
            fmt_hours(grand.billable),
            grand.ready(harvest)
        );
        Ok(())
    }
}
