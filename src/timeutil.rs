//! Dates and durations in a fixed billing timezone.
//!
//! Billing days are anchored to `America/Los_Angeles` regardless of the
//! machine's clock, so "today" is stable when travelling.

use anyhow::{Result, bail};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use chrono_tz::America::Los_Angeles;

/// Today's date in the billing timezone, as `YYYY-MM-DD`.
pub fn today() -> String {
    today_naive().format("%Y-%m-%d").to_string()
}

/// Today's date in the billing timezone.
pub fn today_naive() -> NaiveDate {
    Utc::now().with_timezone(&Los_Angeles).date_naive()
}

/// Parse a `YYYY-MM-DD` string into a `NaiveDate`.
pub fn parse_naive(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("invalid date {s:?}, expected YYYY-MM-DD"))
}

/// Every `YYYY-MM-DD` date from `from` to `to`, inclusive.
pub fn dates_between(from: NaiveDate, to: NaiveDate) -> Vec<String> {
    let mut out = Vec::new();
    let mut d = from;
    while d <= to {
        out.push(d.format("%Y-%m-%d").to_string());
        d += Duration::days(1);
    }
    out
}

/// The Monday–Sunday week containing `d`.
pub fn week_of(d: NaiveDate) -> (NaiveDate, NaiveDate) {
    let offset = d.weekday().num_days_from_monday() as i64;
    let monday = d - Duration::days(offset);
    (monday, monday + Duration::days(6))
}

/// Validate a `YYYY-MM-DD` string, returning it normalized.
pub fn parse_date(s: &str) -> Result<String> {
    let d = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("invalid date {s:?}, expected YYYY-MM-DD"))?;
    Ok(d.format("%Y-%m-%d").to_string())
}

/// Minutes since midnight for an `HH:MM` string.
fn minutes_of_day(s: &str) -> Result<u32> {
    let t = chrono::NaiveTime::parse_from_str(s, "%H:%M")
        .map_err(|_| anyhow::anyhow!("invalid time {s:?}, expected HH:MM (24-hour)"))?;
    Ok(t.signed_duration_since(chrono::NaiveTime::MIN)
        .num_minutes() as u32)
}

/// Decimal hours between two `HH:MM` times on the same day.
pub fn hours_between(from: &str, to: &str) -> Result<f64> {
    let f = minutes_of_day(from)?;
    let t = minutes_of_day(to)?;
    if t <= f {
        bail!("--to ({to}) must be after --from ({from})");
    }
    Ok((t - f) as f64 / 60.0)
}
