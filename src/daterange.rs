//! A reusable clap argument group for selecting a single day or a date range.

use anyhow::{Result, bail};
use chrono::{Duration, NaiveDate};
use clap::Args;

use crate::timeutil;

#[derive(Args)]
pub struct RangeArgs {
    /// Start date YYYY-MM-DD (use with --to)
    #[arg(long)]
    from: Option<String>,
    /// End date YYYY-MM-DD (use with --from)
    #[arg(long)]
    to: Option<String>,
    /// A single day YYYY-MM-DD
    #[arg(long)]
    date: Option<String>,
    /// Just today
    #[arg(long)]
    today: bool,
    /// The current week (Monday–Sunday)
    #[arg(long)]
    week: bool,
    /// Last week (Monday–Sunday)
    #[arg(long)]
    last_week: bool,
}

impl RangeArgs {
    /// Resolve to an inclusive `(from, to)` pair. Defaults to today when nothing
    /// is specified; errors if more than one selector is given.
    pub fn resolve(&self) -> Result<(NaiveDate, NaiveDate)> {
        let mut selectors = 0;
        if self.date.is_some() {
            selectors += 1;
        }
        if self.today {
            selectors += 1;
        }
        if self.week {
            selectors += 1;
        }
        if self.last_week {
            selectors += 1;
        }
        if self.from.is_some() || self.to.is_some() {
            selectors += 1;
        }
        if selectors > 1 {
            bail!("choose only one of --date / --today / --week / --last-week / (--from & --to)");
        }

        if let Some(d) = &self.date {
            let d = timeutil::parse_naive(d)?;
            return Ok((d, d));
        }
        if self.today {
            let t = timeutil::today_naive();
            return Ok((t, t));
        }
        if self.week {
            return Ok(timeutil::week_of(timeutil::today_naive()));
        }
        if self.last_week {
            return Ok(timeutil::week_of(
                timeutil::today_naive() - Duration::days(7),
            ));
        }
        if self.from.is_some() || self.to.is_some() {
            let (f, t) = match (&self.from, &self.to) {
                (Some(f), Some(t)) => (timeutil::parse_naive(f)?, timeutil::parse_naive(t)?),
                _ => bail!("--from and --to must be given together"),
            };
            if t < f {
                bail!("--to must not be before --from");
            }
            return Ok((f, t));
        }
        let t = timeutil::today_naive();
        Ok((t, t))
    }

    /// The list of `YYYY-MM-DD` dates in the resolved range.
    pub fn dates(&self) -> Result<Vec<String>> {
        let (f, t) = self.resolve()?;
        Ok(timeutil::dates_between(f, t))
    }

    /// A human label for the resolved range.
    pub fn label(&self) -> Result<String> {
        let (f, t) = self.resolve()?;
        Ok(if f == t {
            f.format("%Y-%m-%d").to_string()
        } else {
            format!("{} through {}", f.format("%Y-%m-%d"), t.format("%Y-%m-%d"))
        })
    }
}
