use super::Command;
use anyhow::Result;
use clap::Args;
use std::collections::HashSet;

use crate::harvest::HarvestApi;
use crate::daterange::RangeArgs;
use crate::selection::FilterArgs;
use crate::store::Day;
use crate::view::fmt_hours;

/// Approve unapproved entries (the human gate before pushing)
///
/// Approves every unapproved entry in scope, except ones flagged `needs-review`
/// (held until you look) and any passed to `--except`. Or approve just specific
/// entries with `--only <id>`. Review first with `jimtime review --pending`.
///
/// Add `--push` to send them to Harvest in the same step. That is an
/// irreversible external write, which is why it is opt-in and not the default;
/// `jimtime harvest unpush` is the way back.
#[derive(Args)]
pub struct Approve {
    #[command(flatten)]
    range: RangeArgs,
    #[command(flatten)]
    filter: FilterArgs,
    /// Approve only these entry IDs (repeatable); naming one approves it even if
    /// it is flagged needs-review
    #[arg(long)]
    only: Vec<String>,
    /// Entry IDs to hold back (repeatable)
    #[arg(long)]
    except: Vec<String>,
    /// Also approve entries flagged needs-review
    #[arg(long)]
    include_needs_review: bool,
    /// Push what was just approved to Harvest, in the same run
    ///
    /// Off by default: approving is local and reversible, pushing is a write to
    /// a billing system and is not. Non-billable entries are approved but not
    /// pushed. Undo with `jimtime harvest unpush`.
    #[arg(long)]
    push: bool,
}

struct Line {
    date: String,
    hours: f64,
    label: String,
    id: String,
}

impl Line {
    fn print(&self) {
        println!(
            "  {}  {}h  {}  ({})",
            self.date,
            fmt_hours(self.hours),
            self.label,
            self.id
        );
    }
}

#[async_trait::async_trait]
impl Command for Approve {
    async fn run(&self) -> Result<()> {
        let mut approved: Vec<Line> = Vec::new();
        let mut held_review: Vec<Line> = Vec::new();
        let mut held_except: Vec<Line> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let only_mode = !self.only.is_empty();

        for date in &self.range.dates()? {
            let Some(mut day) = Day::load(date)? else {
                continue;
            };
            let mut changed = false;
            for s in &mut day.sections {
                if !self.filter.matches(s) {
                    continue;
                }
                let label = format!("{} - {} - {}", s.client_name, s.project_name, s.task_name);
                for e in &mut s.entries {
                    seen.insert(e.id.clone());
                    if e.approved {
                        continue;
                    }
                    // In --only mode, act on exactly those ids and nothing else.
                    if only_mode && !self.only.contains(&e.id) {
                        continue;
                    }
                    let line = Line {
                        date: date.clone(),
                        hours: e.hours,
                        label: label.clone(),
                        id: e.id.clone(),
                    };
                    if self.except.contains(&e.id) {
                        held_except.push(line);
                        continue;
                    }
                    // Naming an id via --only is explicit intent, so it bypasses
                    // the needs-review hold.
                    if !only_mode && e.needs_review && !self.include_needs_review {
                        held_review.push(line);
                        continue;
                    }
                    e.approved = true;
                    e.needs_review = false;
                    approved.push(line);
                    changed = true;
                }
            }
            if changed {
                day.save()?;
            }
        }

        // A billing gate: an id that matched nothing is almost certainly a typo.
        for id in self.only.iter().chain(self.except.iter()) {
            if !seen.contains(id) {
                eprintln!("warning: id {id} matched no entry in scope");
            }
        }

        if approved.is_empty() && held_review.is_empty() && held_except.is_empty() {
            println!("Nothing to approve for {}.", self.range.label()?);
            return Ok(());
        }

        if !approved.is_empty() {
            println!("Approved {}:", plural(approved.len()));
            approved.iter().for_each(Line::print);
        }
        if !held_review.is_empty() {
            println!(
                "\nHeld {} flagged needs-review (approve with --include-needs-review, or --only <id>):",
                plural(held_review.len())
            );
            held_review.iter().for_each(Line::print);
        }
        if !held_except.is_empty() {
            println!("\nHeld {} via --except:", plural(held_except.len()));
            held_except.iter().for_each(Line::print);
        }

        // Push only what this run actually approved. Nothing approved means
        // nothing to push, and reaching for credentials would just be noise.
        if self.push && !approved.is_empty() {
            println!();
            let api = HarvestApi::from_env()?;
            super::push(&api, &self.range, &self.filter, false).await?;
        }
        Ok(())
    }
}

fn plural(n: usize) -> String {
    format!("{n} entr{}", if n == 1 { "y" } else { "ies" })
}
