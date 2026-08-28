//! On-demand terminal rendering. Never persisted. [ADR-0001]

use crate::store::{Day, Entry, Section};

/// Format hours to two decimals, e.g. `1.25`, `2.00`.
///
/// Adds `0.0` to normalize negative zero (which `f64::sum()` yields over an
/// empty iterator) so it never prints as `-0.00`.
pub fn fmt_hours(h: f64) -> String {
    format!("{:.2}", h + 0.0)
}

/// Format an amount with two decimals and thousands separators, e.g.
/// `1,234.50`. Currency-agnostic: the caller supplies the symbol or code.
pub fn fmt_amount(v: f64) -> String {
    let s = format!("{:.2}", v.abs() + 0.0);
    let (whole, frac) = s.split_once('.').expect("{:.2} always has a decimal point");
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let sign = if v < 0.0 { "-" } else { "" };
    format!("{sign}{grouped}.{frac}")
}

/// `●` unapproved, `○` approved.
pub fn marker(e: &Entry) -> char {
    if e.approved { '○' } else { '●' }
}

/// Trailing status flags for an entry, e.g. `  [needs review]  [imported]`.
pub fn flags(e: &Entry) -> String {
    let mut s = String::new();
    if e.needs_review {
        s.push_str("  [needs review]");
    }
    if e.harvest_time_entry_id.is_some() {
        s.push_str("  [imported]");
    }
    s
}

fn section_hours(s: &Section) -> (f64, f64) {
    let total: f64 = s.entries.iter().map(|e| e.hours).sum();
    let billable: f64 = s
        .entries
        .iter()
        .filter(|e| e.billable)
        .map(|e| e.hours)
        .sum();
    (total, billable)
}

/// Render a full day to a human-readable string.
pub fn render_day(day: &Day) -> String {
    let mut out = String::new();
    out.push_str(&format!("Time Log - {}\n", day.date));

    if day.sections.is_empty() {
        out.push_str("\n  (no entries)\n");
        return out;
    }

    let mut day_total = 0.0;
    for s in &day.sections {
        let (total, billable) = section_hours(s);
        day_total += total;
        out.push_str(&format!(
            "\n{} - {} - {}\n",
            s.client_name, s.project_name, s.task_name
        ));
        for e in &s.entries {
            let bill = if e.billable { "billable" } else { "non-bill" };
            out.push_str(&format!(
                "  {} {:>6}h  {:<8}  {}{}\n",
                marker(e),
                fmt_hours(e.hours),
                bill,
                e.notes,
                flags(e)
            ));
        }
        out.push_str(&format!(
            "  ── {}h total, {}h billable\n",
            fmt_hours(total),
            fmt_hours(billable)
        ));
    }

    out.push_str(&format!("\nDay total: {}h\n", fmt_hours(day_total)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_get_thousands_separators() {
        assert_eq!(fmt_amount(0.0), "0.00");
        assert_eq!(fmt_amount(12.5), "12.50");
        assert_eq!(fmt_amount(999.999), "1,000.00");
        assert_eq!(fmt_amount(1234.5), "1,234.50");
        assert_eq!(fmt_amount(1234567.891), "1,234,567.89");
    }

    #[test]
    fn negative_amounts_keep_one_leading_sign() {
        assert_eq!(fmt_amount(-1234.5), "-1,234.50");
        assert_eq!(fmt_amount(-0.0), "0.00");
    }
}
