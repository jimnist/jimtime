use super::Command;
use anyhow::{Result, bail};
use chrono::{Duration, Months, NaiveDate};
use clap::{Args, Subcommand};

use std::collections::BTreeMap;

use crate::daterange::RangeArgs;
use crate::harvest::HarvestApi;
use crate::selection::FilterArgs;
use crate::store::Day;
use crate::timeutil;
use crate::view::{fmt_amount, fmt_hours};

/// Query Harvest, show uninvoiced balances, and dry-run or push approved time
/// entries
#[derive(Args)]
pub struct Harvest {
    #[command(subcommand)]
    cmd: HarvestCmd,
}

#[derive(Subcommand)]
enum HarvestCmd {
    /// List your Harvest projects with their client and ids
    Projects {
        /// Include archived projects
        #[arg(long)]
        all: bool,
    },
    /// List your Harvest clients with ids
    Clients {
        /// Include archived clients
        #[arg(long)]
        all: bool,
    },
    /// List the tasks assigned to a project (candidates for default_task_id)
    Tasks {
        /// Harvest project id
        #[arg(long)]
        project: u64,
    },
    /// Total uninvoiced billable time by client, in money
    Uninvoiced {
        /// Start date YYYY-MM-DD (default: two months back)
        #[arg(long)]
        from: Option<String>,
        /// End date YYYY-MM-DD (default: today)
        #[arg(long)]
        to: Option<String>,
        /// Add uninvoiced expenses into each client's total
        #[arg(long)]
        with_expenses: bool,
    },
    /// Show what would be pushed to Harvest (no API calls, no credentials)
    DryRun {
        #[command(flatten)]
        range: RangeArgs,
        #[command(flatten)]
        filter: FilterArgs,
        /// Include non-billable entries
        #[arg(long)]
        include_non_billable: bool,
    },
    /// Push approved billable entries to Harvest
    Push {
        #[command(flatten)]
        range: RangeArgs,
        #[command(flatten)]
        filter: FilterArgs,
        /// Include non-billable entries
        #[arg(long)]
        include_non_billable: bool,
    },
}

#[async_trait::async_trait]
impl Command for Harvest {
    async fn run(&self) -> Result<()> {
        match &self.cmd {
            HarvestCmd::Projects { all } => {
                let api = HarvestApi::from_env()?;
                let mut projects = api.list_projects(!all).await?;
                projects.sort_by(|a, b| {
                    a.client
                        .name
                        .to_lowercase()
                        .cmp(&b.client.name.to_lowercase())
                        .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
                println!(
                    "{:<28} {:>10}   {:<32} {:>10}  CODE",
                    "CLIENT", "CLIENT_ID", "PROJECT", "PROJECT_ID"
                );
                for p in &projects {
                    let flag = if p.is_active { "" } else { "  (archived)" };
                    println!(
                        "{:<28} {:>10}   {:<32} {:>10}  {}{}",
                        truncate(&p.client.name, 28),
                        p.client.id,
                        truncate(&p.name, 32),
                        p.id,
                        p.code.clone().unwrap_or_default(),
                        flag
                    );
                }
            }
            HarvestCmd::Clients { all } => {
                let api = HarvestApi::from_env()?;
                let mut clients = api.list_clients(!all).await?;
                clients.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                println!("{:<40} {:>10}", "CLIENT", "CLIENT_ID");
                for c in &clients {
                    let flag = if c.is_active { "" } else { "  (archived)" };
                    println!("{:<40} {:>10}{}", truncate(&c.name, 40), c.id, flag);
                }
            }
            HarvestCmd::Tasks { project } => {
                let api = HarvestApi::from_env()?;
                let tasks = api.task_assignments(*project).await?;
                if tasks.is_empty() {
                    println!("No task assignments found for project {project}.");
                    return Ok(());
                }
                println!("{:<36} {:>10}  BILLABLE", "TASK", "TASK_ID");
                for t in &tasks {
                    let flag = if t.is_active { "" } else { "  (archived)" };
                    println!(
                        "{:<36} {:>10}  {}{}",
                        truncate(&t.task.name, 36),
                        t.task.id,
                        if t.billable { "yes" } else { "no" },
                        flag
                    );
                }
            }
            HarvestCmd::Uninvoiced {
                from,
                to,
                with_expenses,
            } => {
                let api = HarvestApi::from_env()?;
                uninvoiced(&api, from.as_deref(), to.as_deref(), *with_expenses).await?;
            }
            HarvestCmd::DryRun {
                range,
                filter,
                include_non_billable,
            } => {
                dry_run(range, filter, *include_non_billable)?;
            }
            HarvestCmd::Push {
                range,
                filter,
                include_non_billable,
            } => {
                let api = HarvestApi::from_env()?;
                push(&api, range, filter, *include_non_billable).await?;
            }
        }
        Ok(())
    }
}

/// Print each client's uninvoiced billable time and what it is worth, largest
/// first. Harvest computes the money, so the rates stay where they are managed.
async fn uninvoiced(
    api: &HarvestApi,
    from: Option<&str>,
    to: Option<&str>,
    with_expenses: bool,
) -> Result<()> {
    let to = match to {
        Some(t) => timeutil::parse_naive(t)?,
        None => timeutil::today_naive()?,
    };
    let from = match from {
        Some(f) => timeutil::parse_naive(f)?,
        None => default_from(to),
    };
    if to < from {
        bail!("--to must not be before --from");
    }

    // Rates - and so amounts - are per currency, and only totals within one
    // currency mean anything, so each gets its own table.
    let mut by_currency: BTreeMap<String, BTreeMap<String, Totals>> = BTreeMap::new();
    for (w_from, w_to) in windows(from, to) {
        let rows = api
            .uninvoiced_report(&fmt_date(w_from), &fmt_date(w_to))
            .await?;
        for r in &rows {
            let t = by_currency
                .entry(r.currency.clone())
                .or_default()
                .entry(r.client_name.clone())
                .or_default();
            t.hours += r.uninvoiced_hours;
            t.amount += r.uninvoiced_amount;
            t.expenses += r.uninvoiced_expenses;
        }
    }

    println!("Uninvoiced - {} through {}\n", fmt_date(from), fmt_date(to));

    let mut printed = 0usize;
    for (currency, clients) in &by_currency {
        let mut clients: Vec<(&String, &Totals)> = clients
            .iter()
            .filter(|(_, t)| t.hours != 0.0 || t.amount != 0.0 || t.expenses != 0.0)
            .collect();
        if clients.is_empty() {
            continue;
        }
        // Biggest outstanding balance first: that is the thing to act on.
        clients.sort_by(|(a_name, a), (b_name, b)| {
            b.total(with_expenses)
                .partial_cmp(&a.total(with_expenses))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a_name.to_lowercase().cmp(&b_name.to_lowercase()))
        });

        if printed > 0 {
            println!();
        }
        printed += 1;

        println!("{:<NAME_W$} {:>9}  {:>13}", "CLIENT", "HOURS", currency);
        let mut grand = Totals::default();
        for (name, t) in &clients {
            println!(
                "{:<NAME_W$} {:>9}  {:>13}",
                truncate(name, NAME_W),
                fmt_hours(t.hours),
                fmt_amount(t.total(with_expenses))
            );
            grand.hours += t.hours;
            grand.amount += t.amount;
            grand.expenses += t.expenses;
        }
        println!("{}", "-".repeat(NAME_W + 25));
        println!(
            "{:<NAME_W$} {:>9}  {:>13}",
            format!(
                "Total ({} client{})",
                clients.len(),
                if clients.len() == 1 { "" } else { "s" }
            ),
            fmt_hours(grand.hours),
            fmt_amount(grand.total(with_expenses))
        );
        if !with_expenses && grand.expenses != 0.0 {
            println!(
                "Plus {} {currency} in uninvoiced expenses, not counted above (--with-expenses).",
                fmt_amount(grand.expenses)
            );
        }
    }

    if printed == 0 {
        println!("Nothing uninvoiced.");
    }
    Ok(())
}

/// Width of the client-name column in the uninvoiced table.
const NAME_W: usize = 36;

fn fmt_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// The widest span Harvest accepts for the uninvoiced report: the endpoints may
/// be at most 365 days apart, leap years included.
const MAX_SPAN_DAYS: i64 = 365;

/// Where an unspecified range starts: two months back. Anything uninvoiced is
/// normally days or weeks old, so this covers the current and previous billing
/// month with room to spare, in one request. Older balances need an explicit
/// `--from`.
fn default_from(to: NaiveDate) -> NaiveDate {
    to.checked_sub_months(Months::new(2))
        .unwrap_or(NaiveDate::MIN)
}

/// Split an inclusive range into windows the uninvoiced report will accept. The
/// windows are disjoint, so the per-project hours and amounts from each simply
/// add up.
fn windows(from: NaiveDate, to: NaiveDate) -> Vec<(NaiveDate, NaiveDate)> {
    let mut out = Vec::new();
    let mut start = from;
    while start <= to {
        let end = start
            .checked_add_signed(Duration::days(MAX_SPAN_DAYS))
            .unwrap_or(NaiveDate::MAX)
            .min(to);
        out.push((start, end));
        let Some(next) = end.succ_opt() else { break };
        start = next;
    }
    out
}

/// A client's uninvoiced totals, summed over their projects.
#[derive(Default)]
struct Totals {
    hours: f64,
    amount: f64,
    expenses: f64,
}

impl Totals {
    fn total(&self, with_expenses: bool) -> f64 {
        if with_expenses {
            self.amount + self.expenses
        } else {
            self.amount
        }
    }
}

/// Print the entries that a push would create, without calling the API.
fn dry_run(range: &RangeArgs, filter: &FilterArgs, include_non_billable: bool) -> Result<()> {
    let dates = range.dates()?;
    println!("Dry run: Harvest import - {}\n", range.label()?);

    let mut count = 0usize;
    let mut total = 0.0;
    for date in &dates {
        let Some(day) = Day::load(date)? else { continue };
        for s in &day.sections {
            if !filter.matches(s) {
                continue;
            }
            for e in &s.entries {
                if !e.is_pushable(include_non_billable) {
                    continue;
                }
                println!(
                    "{date}  {}h  {} - {} - {}",
                    fmt_hours(e.hours),
                    s.client_name,
                    s.project_name,
                    s.task_name
                );
                println!("  id: {}", e.id);
                println!("  notes: {}", e.notes);
                count += 1;
                total += e.hours;
            }
        }
    }

    if count == 0 {
        println!("Nothing eligible to push.");
    } else {
        println!(
            "\nTotal eligible: {count} entr{}, {}h",
            if count == 1 { "y" } else { "ies" },
            fmt_hours(total)
        );
        println!("No entries were pushed.");
    }
    Ok(())
}

/// Create Harvest time entries for eligible entries, saving the returned id back
/// to each entry immediately so a mid-run failure never re-pushes.
async fn push(
    api: &HarvestApi,
    range: &RangeArgs,
    filter: &FilterArgs,
    include_non_billable: bool,
) -> Result<()> {
    if !api.tracks_by_duration().await? {
        bail!(
            "Your Harvest account tracks time via start/end timestamps, not duration.\n\
             The hours-based create used here won't work - switch the account to duration \
             tracking in Harvest settings, or the push needs to send timestamps instead."
        );
    }

    let dates = range.dates()?;
    println!("Pushing approved Harvest entries - {}\n", range.label()?);

    let mut imported = 0usize;
    let mut total = 0.0;
    for date in &dates {
        let Some(mut day) = Day::load(date)? else {
            continue;
        };
        for si in 0..day.sections.len() {
            if !filter.matches(&day.sections[si]) {
                continue;
            }
            let (project_id, task_id) = (day.sections[si].project_id, day.sections[si].task_id);
            for ei in 0..day.sections[si].entries.len() {
                if !day.sections[si].entries[ei].is_pushable(include_non_billable) {
                    continue;
                }
                let (id, hours, notes) = {
                    let e = &day.sections[si].entries[ei];
                    (e.id.clone(), e.hours, e.notes.clone())
                };
                match api
                    .create_time_entry(project_id, task_id, date, hours, &notes)
                    .await
                {
                    Ok(harvest_id) => {
                        day.sections[si].entries[ei].harvest_time_entry_id = Some(harvest_id);
                        // The create already happened server-side; if we can't
                        // persist the id, surface it loudly so the user can
                        // record it and avoid a double-push on re-run.
                        if let Err(save_err) = day.save() {
                            eprintln!(
                                "\nHarvest time entry {harvest_id} WAS created for {id}, but saving \
                                 the store failed:\n  {save_err:#}\n\
                                 Record  \"harvest_time_entry_id\": {harvest_id}  on entry {id} in \
                                 the JSON BEFORE re-running, or it will be pushed again (double-billed)."
                            );
                            return Err(save_err);
                        }
                        println!("  {id} → Harvest time entry {harvest_id}");
                        imported += 1;
                        total += hours;
                    }
                    Err(err) => {
                        eprintln!("\nFailed on {id}: {err:#}");
                        eprintln!(
                            "Imported {imported} entr{} ({}h) before this failure; state saved.\n\
                             If this was a timeout the entry may still have been created in Harvest - \
                             check there before re-running to avoid a duplicate.",
                            if imported == 1 { "y" } else { "ies" },
                            fmt_hours(total)
                        );
                        return Err(err);
                    }
                }
            }
        }
    }

    if imported == 0 {
        println!("Nothing eligible to push.");
    } else {
        println!(
            "\nTotal imported: {imported} entr{}, {}h",
            if imported == 1 { "y" } else { "ies" },
            fmt_hours(total)
        );
    }
    Ok(())
}

fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn a_short_range_is_one_window() {
        assert_eq!(
            windows(d("2026-01-01"), d("2026-03-01")),
            vec![(d("2026-01-01"), d("2026-03-01"))]
        );
    }

    #[test]
    fn windows_never_exceed_the_api_limit_and_stay_disjoint() {
        let (from, to) = (d("2023-01-01"), d("2026-08-28"));
        let ws = windows(from, to);
        assert_eq!(ws.first().unwrap().0, from);
        assert_eq!(ws.last().unwrap().1, to);
        for (i, (s, e)) in ws.iter().enumerate() {
            assert!(s <= e);
            assert!(
                (*e - *s).num_days() <= MAX_SPAN_DAYS,
                "window {i} spans {} days",
                (*e - *s).num_days()
            );
            if i > 0 {
                assert_eq!(ws[i - 1].1.succ_opt().unwrap(), *s, "gap or overlap at {i}");
            }
        }
    }

    #[test]
    fn the_default_range_is_two_months_in_one_window() {
        for to in ["2024-02-29", "2026-08-28", "2026-01-01"] {
            let to = d(to);
            assert_eq!(windows(default_from(to), to).len(), 1, "for {to}");
        }
    }
}
