use super::Command;
use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::daterange::RangeArgs;
use crate::harvest::HarvestApi;
use crate::selection::FilterArgs;
use crate::store::Day;
use crate::view::fmt_hours;

/// Query Harvest, and dry-run or push approved time entries
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

/// Print the entries that a push would create, without calling the API.
fn dry_run(range: &RangeArgs, filter: &FilterArgs, include_non_billable: bool) -> Result<()> {
    let dates = range.dates()?;
    println!("Dry run: Harvest import — {}\n", range.label()?);

    let mut count = 0usize;
    let mut total = 0.0;
    for date in &dates {
        let Some(day) = Day::load(date)? else { continue };
        for s in &day.sections {
            if !s.approved || !filter.matches(s) {
                continue;
            }
            for e in &s.entries {
                if e.harvest_time_entry_id.is_some() {
                    continue;
                }
                if !e.billable && !include_non_billable {
                    continue;
                }
                println!(
                    "{date}  {}h  {} — {} — {}",
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
        println!("\nTotal eligible: {count} entries, {}h", fmt_hours(total));
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
    println!("Pushing approved Harvest entries — {}\n", range.label()?);

    let mut imported = 0usize;
    let mut total = 0.0;
    for date in &dates {
        let Some(mut day) = Day::load(date)? else {
            continue;
        };
        for si in 0..day.sections.len() {
            if !(day.sections[si].approved && filter.matches(&day.sections[si])) {
                continue;
            }
            let (project_id, task_id) =
                (day.sections[si].project_id, day.sections[si].task_id);
            for ei in 0..day.sections[si].entries.len() {
                let (id, hours, notes, billable, already) = {
                    let e = &day.sections[si].entries[ei];
                    (
                        e.id.clone(),
                        e.hours,
                        e.notes.clone(),
                        e.billable,
                        e.harvest_time_entry_id.is_some(),
                    )
                };
                if already || (!billable && !include_non_billable) {
                    continue;
                }
                match api
                    .create_time_entry(project_id, task_id, date, hours, &notes)
                    .await
                {
                    Ok(harvest_id) => {
                        day.sections[si].entries[ei].harvest_time_entry_id = Some(harvest_id);
                        day.save()?;
                        println!("  {id} → Harvest time entry {harvest_id}");
                        imported += 1;
                        total += hours;
                    }
                    Err(err) => {
                        eprintln!("\nFailed on {id}: {err:#}");
                        eprintln!(
                            "Imported {imported} entr{} ({}h) before this failure; state saved. \
                             Fix and re-run - already-imported entries are skipped.",
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
        println!("\nTotal imported: {imported} entries, {}h", fmt_hours(total));
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
