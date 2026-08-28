//! Dates and durations in a fixed billing timezone.
//!
//! Billing days are anchored to one timezone regardless of the machine's clock,
//! so "today" is stable when travelling. The zone is `$JIMTIME_TZ`, an IANA
//! name like `Europe/Berlin`, defaulting to [`DEFAULT_TZ`].

use anyhow::{Result, anyhow, bail};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use chrono_tz::Tz;

/// The billing timezone used when `$JIMTIME_TZ` is unset.
pub const DEFAULT_TZ: Tz = chrono_tz::America::Los_Angeles;

/// The billing timezone: `$JIMTIME_TZ` if set, else [`DEFAULT_TZ`].
pub fn billing_tz() -> Result<Tz> {
    let raw = std::env::var_os("JIMTIME_TZ");
    resolve_tz(raw.as_ref().map(|v| v.to_string_lossy()).as_deref())
}

/// Resolve a raw `$JIMTIME_TZ` value. Split out from [`billing_tz`] so it can
/// be tested without touching process-wide environment state.
fn resolve_tz(raw: Option<&str>) -> Result<Tz> {
    let Some(name) = raw else {
        return Ok(DEFAULT_TZ);
    };
    let name = name.trim();
    if name.is_empty() {
        bail!(
            "JIMTIME_TZ is set but empty; unset it to use {}",
            DEFAULT_TZ
        );
    }
    name.parse::<Tz>().map_err(|_| {
        anyhow!(
            "unknown timezone {name:?} in JIMTIME_TZ; expected an IANA name \
             such as \"America/Los_Angeles\" or \"Europe/Berlin\""
        )
    })
}

/// Today's date in the billing timezone, as `YYYY-MM-DD`.
pub fn today() -> Result<String> {
    Ok(today_naive()?.format("%Y-%m-%d").to_string())
}

/// Today's date in the billing timezone.
pub fn today_naive() -> Result<NaiveDate> {
    Ok(Utc::now().with_timezone(&billing_tz()?).date_naive())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        parse_naive(s).unwrap()
    }

    #[test]
    fn tz_defaults_and_parses() {
        assert_eq!(resolve_tz(None).unwrap(), DEFAULT_TZ);
        assert_eq!(
            resolve_tz(Some("Europe/Berlin")).unwrap(),
            Tz::Europe__Berlin
        );
        assert_eq!(resolve_tz(Some("  UTC  ")).unwrap(), Tz::UTC);
    }

    #[test]
    fn tz_rejects_empty_and_unknown() {
        assert!(resolve_tz(Some("")).is_err());
        assert!(resolve_tz(Some("   ")).is_err());
        assert!(resolve_tz(Some("Mars/Olympus_Mons")).is_err());
        assert!(resolve_tz(Some("PST")).is_err());
    }

    #[test]
    fn hours_between_computes_decimal() {
        assert_eq!(hours_between("09:00", "10:15").unwrap(), 1.25);
        assert_eq!(hours_between("13:00", "13:30").unwrap(), 0.5);
    }

    #[test]
    fn hours_between_rejects_bad_order() {
        assert!(hours_between("10:00", "09:00").is_err());
        assert!(hours_between("09:00", "09:00").is_err());
        assert!(hours_between("nope", "10:00").is_err());
    }

    #[test]
    fn week_of_is_monday_to_sunday() {
        // 2026-07-28 is a Tuesday.
        assert_eq!(week_of(d("2026-07-28")), (d("2026-07-27"), d("2026-08-02")));
        // A Monday maps to itself.
        assert_eq!(week_of(d("2026-07-27")), (d("2026-07-27"), d("2026-08-02")));
        // A Sunday is the end of its week.
        assert_eq!(week_of(d("2026-08-02")), (d("2026-07-27"), d("2026-08-02")));
    }

    #[test]
    fn dates_between_is_inclusive() {
        assert_eq!(
            dates_between(d("2026-07-27"), d("2026-07-29")),
            vec!["2026-07-27", "2026-07-28", "2026-07-29"]
        );
        assert_eq!(dates_between(d("2026-07-27"), d("2026-07-27")).len(), 1);
    }
}
