//! On-demand terminal rendering of a `Day`. Never persisted. [ADR-0001]

use crate::store::{Day, Section};

/// Format hours to two decimals, e.g. `1.25`, `2.00`.
///
/// Adds `0.0` to normalize negative zero (which `f64::sum()` yields over an
/// empty iterator) so it never prints as `-0.00`.
pub fn fmt_hours(h: f64) -> String {
    format!("{:.2}", h + 0.0)
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
    out.push_str(&format!("Time Log — {}\n", day.date));

    if day.sections.is_empty() {
        out.push_str("\n  (no entries)\n");
        return out;
    }

    let mut day_total = 0.0;
    for s in &day.sections {
        let (total, billable) = section_hours(s);
        day_total += total;
        let status = if s.approved { "approved" } else { "unapproved" };
        out.push_str(&format!(
            "\n{} — {} — {}   [{}]\n",
            s.client_name, s.project_name, s.task_name, status
        ));
        for e in &s.entries {
            let bill = if e.billable { "billable" } else { "non-bill" };
            let mut flags = String::new();
            if e.needs_review {
                flags.push_str("  [needs review]");
            }
            if e.harvest_time_entry_id.is_some() {
                flags.push_str("  [imported]");
            }
            out.push_str(&format!(
                "  {:>6}h  {:<8}  {}{}\n",
                fmt_hours(e.hours),
                bill,
                e.notes,
                flags
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
