//! Invoices: selecting invoiceable time, pricing it, numbering, and the
//! invoice record that is the durable truth of what was billed. [ADR-0007,
//! ADR-0008]

pub mod harvest_import;
pub mod mail;
pub mod render;

use anyhow::{Context, Result, bail};
use chrono::{Duration, NaiveDate};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

use crate::config::{Business, Config};
use crate::datarepo;
use crate::paths;
use crate::store::{Day, write_atomic};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Draft,
    Finalized,
    Void,
}

/// An invoice, as rendered and as recorded in `invoices/YYYY/<number>.json`.
/// Everything the PDF shows is snapshotted here, so later config edits never
/// change what an issued invoice said.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Invoice {
    pub number: String,
    pub year: i32,
    pub seq: u32,
    pub status: Status,
    pub client_key: String,
    pub client: Party,
    pub business: Business,
    pub currency: String,
    pub issue_date: String,
    pub due_date: String,
    pub period_from: String,
    pub period_to: String,
    pub lines: Vec<Line>,
    pub total_hours: f64,
    pub total: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sent: Vec<SendEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uploads: Vec<Upload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voided_at: Option<String>,
    /// The day the client paid, `YYYY-MM-DD`. [ADR-0011]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paid_date: Option<String>,
    /// Where the invoice was issued.
    #[serde(default, skip_serializing_if = "Source::is_jimtime")]
    pub source: Source,
    /// For an invoice imported from Harvest: what Harvest billed, exactly as
    /// the client saw it. Authoritative over `lines`, which only record the
    /// time it covered (Harvest lines can be edited, and discounted). [ADR-0011]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harvest: Option<HarvestSnapshot>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    #[default]
    Jimtime,
    /// Issued in Harvest, imported by `invoice import-harvest`.
    Harvest,
}

impl Source {
    fn is_jimtime(&self) -> bool {
        *self == Source::Jimtime
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HarvestSnapshot {
    pub id: u64,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub line_items: Vec<crate::harvest::InvoiceLineItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discount_percent: Option<f64>,
    pub discount_amount: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tax_percent: Option<f64>,
    pub tax_amount: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tax2_percent: Option<f64>,
    pub tax2_amount: f64,
}

/// The client as billed.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Party {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default)]
    pub email_to: Vec<String>,
    #[serde(default)]
    pub email_cc: Vec<String>,
}

/// One invoiced Entry.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Line {
    pub entry_id: String,
    pub date: String,
    pub project: String,
    pub project_name: String,
    pub task: String,
    pub task_name: String,
    pub notes: String,
    pub hours: f64,
    pub rate: f64,
    pub amount: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SendEvent {
    pub at: String,
    pub to: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cc: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bcc: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Upload {
    pub provider: String,
    pub at: String,
    /// Where it landed: a Dropbox path or a Google Drive file id.
    pub location: String,
}

/// Round to cents. Lines are rounded, and the total is the sum of the rounded
/// lines, so the invoice always adds up on paper. Hours are never rounded.
pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// What an invoice for a client over some dates would contain.
pub struct Selection {
    pub lines: Vec<Line>,
    /// Billable, un-invoiced entries left out because they are unapproved.
    pub held_unapproved: usize,
    /// ...of which flagged needs-review.
    pub held_needs_review: usize,
    /// Selected entries that were also pushed to Harvest, which may mean the
    /// client is billed from there too.
    pub harvest_linked: usize,
}

/// Collect the invoiceable entries for a client (optionally one project) over
/// the given dates, priced from config.
pub fn select(
    config: &Config,
    client_key: &str,
    project: Option<&str>,
    dates: &[String],
) -> Result<Selection> {
    config.client(client_key)?;
    let mut sel = Selection {
        lines: Vec::new(),
        held_unapproved: 0,
        held_needs_review: 0,
        harvest_linked: 0,
    };
    for date in dates {
        let Some(day) = Day::load(date)? else {
            continue;
        };
        for s in &day.sections {
            if s.client != client_key || project.is_some_and(|p| p != s.project) {
                continue;
            }
            for e in &s.entries {
                if e.billable && e.invoice.is_none() && !e.approved {
                    sel.held_unapproved += 1;
                    sel.held_needs_review += usize::from(e.needs_review);
                }
                if !e.is_invoiceable() {
                    continue;
                }
                sel.harvest_linked += usize::from(e.harvest_time_entry_id.is_some());
                let rate = config.rate(&s.client, &s.project, &s.task)?;
                sel.lines.push(Line {
                    entry_id: e.id.clone(),
                    date: date.clone(),
                    project: s.project.clone(),
                    project_name: s.project_name.clone(),
                    task: s.task.clone(),
                    task_name: s.task_name.clone(),
                    notes: e.notes.clone(),
                    hours: e.hours,
                    rate,
                    amount: round2(e.hours * rate),
                });
            }
        }
    }
    Ok(sel)
}

impl Invoice {
    /// Build an invoice (draft or not yet numbered) from a selection.
    pub fn build(
        config: &Config,
        client_key: &str,
        lines: Vec<Line>,
        issue: NaiveDate,
        period: (NaiveDate, NaiveDate),
    ) -> Result<Invoice> {
        let client = config.client(client_key)?;
        let total = round2(lines.iter().map(|l| l.amount).sum());
        let total_hours = lines.iter().map(|l| l.hours).sum::<f64>() + 0.0;
        let due = issue + Duration::days(i64::from(config.invoice.due_days));
        Ok(Invoice {
            number: "DRAFT".into(),
            year: chrono::Datelike::year(&issue),
            seq: 0,
            status: Status::Draft,
            client_key: client_key.to_string(),
            client: Party {
                name: client.name.clone(),
                address: client.address.clone(),
                email_to: client.email_to.clone(),
                email_cc: client.email_cc.clone(),
            },
            business: config.business.clone(),
            currency: client.currency.clone(),
            issue_date: fmt_date(issue),
            due_date: fmt_date(due),
            period_from: fmt_date(period.0),
            period_to: fmt_date(period.1),
            lines,
            total_hours,
            total,
            notes: config.invoice.notes.clone(),
            sent: Vec::new(),
            uploads: Vec::new(),
            voided_at: None,
            paid_date: None,
            source: Source::Jimtime,
            harvest: None,
        })
    }

    /// A short hash of what is being billed and to whom. A draft prints it and
    /// `finalize --confirm` checks it, so what was approved is what is sent.
    /// Deliberately excludes the number and dates, which a draft does not have.
    pub fn fingerprint(&self, config: &Config) -> String {
        let mut h = Sha256::new();
        let mut feed = |s: &str| {
            h.update(s.as_bytes());
            h.update([0u8]);
        };
        feed(&self.client_key);
        feed(&self.currency);
        for l in &self.lines {
            feed(&l.entry_id);
            feed(&format!("{}|{}|{}", l.hours, l.rate, l.amount));
            feed(&l.notes);
        }
        feed(&format!("{}", self.total));
        for r in self.recipients(config).all() {
            feed(&r);
        }
        hex::encode(h.finalize())[..12].to_string()
    }

    /// Who an email of this invoice goes to: the client's `email_to`, then Cc
    /// from the client (including any one-off `--cc`) and from `[email]`, then
    /// Bcc from `[email]`. Each address appears once, in the most visible
    /// place it is listed (To over Cc over Bcc), so nobody gets it twice.
    pub fn recipients(&self, config: &Config) -> Recipients {
        let (global_cc, global_bcc) = match &config.email {
            Some(e) => (e.cc.as_slice(), e.bcc.as_slice()),
            None => (&[][..], &[][..]),
        };
        let mut seen: Vec<String> = Vec::new();
        let mut take = |list: &mut dyn Iterator<Item = &String>| -> Vec<String> {
            let mut out = Vec::new();
            for a in list {
                let key = address_key(a);
                if !seen.contains(&key) {
                    seen.push(key);
                    out.push(a.clone());
                }
            }
            out
        };
        let to = take(&mut self.client.email_to.iter());
        let cc = take(&mut self.client.email_cc.iter().chain(global_cc));
        let bcc = take(&mut global_bcc.iter());
        Recipients { to, cc, bcc }
    }

    /// Issued and not yet paid (nor void).
    pub fn is_outstanding(&self) -> bool {
        self.status == Status::Finalized && self.paid_date.is_none()
    }

    /// `paid 2026-08-01`, `overdue since 2026-10-17`, `open, due 2026-10-17`,
    /// `not sent`, or `void`, as of `today`.
    pub fn payment_status(&self, today: NaiveDate) -> String {
        if self.status == Status::Void {
            return "void".into();
        }
        if let Some(d) = &self.paid_date {
            return format!("paid {d}");
        }
        let overdue = NaiveDate::parse_from_str(&self.due_date, "%Y-%m-%d")
            .map(|due| due < today)
            .unwrap_or(false);
        match (overdue, self.sent.is_empty()) {
            (true, _) => format!("OVERDUE since {}", self.due_date),
            (false, true) => format!("not sent, due {}", self.due_date),
            (false, false) => format!("open, due {}", self.due_date),
        }
    }

    pub fn record_path(&self) -> Result<PathBuf> {
        Ok(year_dir(self.year)?.join(format!("{}.json", self.number)))
    }

    pub fn pdf_path(&self) -> Result<PathBuf> {
        Ok(year_dir(self.year)?.join(format!("{}.pdf", self.number)))
    }

    /// The PDF's file name when attached or uploaded.
    pub fn pdf_name(&self) -> String {
        format!("Invoice {}.pdf", self.number)
    }

    /// Write the record and note it for the data repo.
    pub fn save(&self) -> Result<()> {
        let path = self.record_path()?;
        let text = serde_json::to_string_pretty(self)? + "\n";
        write_atomic(&path, text.as_bytes())?;
        datarepo::note_write(&path);
        Ok(())
    }

    /// Load a finalized or void invoice by number.
    pub fn load(number: &str) -> Result<Invoice> {
        all()?
            .into_iter()
            .find(|i| i.number == number)
            .with_context(|| format!("no invoice numbered {number:?}"))
    }
}

/// The bare, lowercased address of `Name <a@b.c>` or `a@b.c`, for comparing.
fn address_key(a: &str) -> String {
    let a = a.trim();
    let bare = match (a.rfind('<'), a.rfind('>')) {
        (Some(l), Some(r)) if l < r => &a[l + 1..r],
        _ => a,
    };
    bare.trim().to_lowercase()
}

pub struct Recipients {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
}

impl Recipients {
    fn all(&self) -> impl Iterator<Item = String> + '_ {
        self.to
            .iter()
            .map(|s| format!("to:{s}"))
            .chain(self.cc.iter().map(|s| format!("cc:{s}")))
            .chain(self.bcc.iter().map(|s| format!("bcc:{s}")))
    }
}

fn year_dir(year: i32) -> Result<PathBuf> {
    Ok(paths::invoices_dir()?.join(year.to_string()))
}

pub fn fmt_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// Every invoice record, oldest number first.
pub fn all() -> Result<Vec<Invoice>> {
    let root = paths::invoices_dir()?;
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    for year in std::fs::read_dir(&root)? {
        let year = year?.path();
        let is_year = year
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.len() == 4 && n.chars().all(|c| c.is_ascii_digit()));
        if !year.is_dir() || !is_year {
            continue;
        }
        for f in std::fs::read_dir(&year)? {
            let f = f?.path();
            if f.extension().is_some_and(|e| e == "json") {
                let text = std::fs::read_to_string(&f)
                    .with_context(|| format!("reading {}", f.display()))?;
                out.push(
                    serde_json::from_str::<Invoice>(&text)
                        .with_context(|| format!("parsing {}", f.display()))?,
                );
            }
        }
    }
    out.sort_by(|a, b| (a.year, a.seq, &a.number).cmp(&(b.year, b.seq, &b.number)));
    Ok(out)
}

/// The next number for an invoice issued in `year`: one past the highest
/// sequence already used (in that year, when the format includes `{year}`),
/// never below `start_seq`. Void invoices keep their numbers.
///
/// `external` are numbers issued elsewhere that share this sequence - Harvest's
/// invoice numbers with `invoice.harvest_numbering`. Those that fit the format
/// count toward the highest; the others belong to some other scheme and are
/// ignored.
pub fn next_number(
    config: &Config,
    year: i32,
    existing: &[Invoice],
    external: &[String],
) -> Result<(String, u32)> {
    let fmt = &config.invoice.number_format;
    let tokens = tokens(fmt)?;
    let per_year = tokens.contains(&Token::Year);
    let local = existing
        .iter()
        .filter(|i| !per_year || i.year == year)
        .map(|i| i.seq);
    let elsewhere = external
        .iter()
        .filter_map(|n| parse_number(&tokens, n))
        .filter(|(y, _)| !per_year || *y == Some(year))
        .map(|(_, seq)| seq);
    let max = local.chain(elsewhere).max().unwrap_or(0);
    let seq = max.max(config.invoice.start_seq.saturating_sub(1)) + 1;
    let number = format_number(fmt, year, seq)?;
    if existing.iter().any(|i| i.number == number) || external.contains(&number) {
        bail!("invoice number {number} is already used; check invoice.number_format");
    }
    Ok((number, seq))
}

#[derive(Debug, PartialEq)]
enum Token {
    Lit(String),
    Year,
    /// `{seq}` (width 0) or `{seq:0N}`.
    Seq(usize),
}

/// Split a number format into literals and placeholders.
fn tokens(fmt: &str) -> Result<Vec<Token>> {
    let mut out = Vec::new();
    let mut rest = fmt;
    while let Some(start) = rest.find('{') {
        if start > 0 {
            out.push(Token::Lit(rest[..start].to_string()));
        }
        let end = rest[start..]
            .find('}')
            .with_context(|| format!("unclosed {{ in number_format {fmt:?}"))?
            + start;
        out.push(match &rest[start + 1..end] {
            "year" => Token::Year,
            "seq" => Token::Seq(0),
            t if t.starts_with("seq:0") => Token::Seq(
                t["seq:0".len()..]
                    .parse()
                    .with_context(|| format!("bad width in {{{t}}}"))?,
            ),
            t => bail!("unknown placeholder {{{t}}} in number_format {fmt:?}"),
        });
        rest = &rest[end + 1..];
    }
    if !rest.is_empty() {
        out.push(Token::Lit(rest.to_string()));
    }
    if !out.iter().any(|t| matches!(t, Token::Seq(_))) {
        bail!("number_format {fmt:?} needs a {{seq}} placeholder");
    }
    Ok(out)
}

/// Read `(year, seq)` back out of a number, if it fits the format. `036`
/// fits `{seq:03}`; `INV-9` does not.
/// `(year, seq)` of a number under the configured format, if it fits.
pub fn seq_of(config: &Config, number: &str) -> Result<Option<(Option<i32>, u32)>> {
    Ok(parse_number(
        &tokens(&config.invoice.number_format)?,
        number,
    ))
}

fn parse_number(tokens: &[Token], number: &str) -> Option<(Option<i32>, u32)> {
    let mut rest = number.trim();
    let (mut year, mut seq) = (None, None);
    for t in tokens {
        match t {
            Token::Lit(l) => rest = rest.strip_prefix(l.as_str())?,
            Token::Year => {
                let digits = rest
                    .get(..4)
                    .filter(|d| d.bytes().all(|b| b.is_ascii_digit()))?;
                year = Some(digits.parse().ok()?);
                rest = &rest[4..];
            }
            Token::Seq(_) => {
                let n = rest.bytes().take_while(u8::is_ascii_digit).count();
                seq = Some(rest[..n].parse().ok()?);
                rest = &rest[n..];
            }
        }
    }
    rest.is_empty().then_some((year, seq?))
}

/// Expand `{year}`, `{seq}` and `{seq:0N}`. The result becomes a file name, so
/// only a safe character set is allowed.
pub fn format_number(fmt: &str, year: i32, seq: u32) -> Result<String> {
    let mut out = String::new();
    for t in tokens(fmt)? {
        match t {
            Token::Lit(l) => out.push_str(&l),
            Token::Year => out.push_str(&year.to_string()),
            Token::Seq(width) => out.push_str(&format!("{seq:0width$}")),
        }
    }
    if out.is_empty()
        || !out
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        bail!("number_format {fmt:?} produces {out:?}; use only letters, digits, - _ .");
    }
    Ok(out)
}

/// Lock (or with `None`, unlock) the listed entries to an invoice number,
/// saving each day. Locking refuses an entry that stopped being invoiceable.
pub fn set_entry_invoice(lines: &[Line], number: &str, lock: bool) -> Result<()> {
    let mut dates: Vec<&str> = lines.iter().map(|l| l.date.as_str()).collect();
    dates.sort_unstable();
    dates.dedup();
    for date in dates {
        let mut day =
            Day::load(date)?.with_context(|| format!("day {date} disappeared while invoicing"))?;
        for s in &mut day.sections {
            for e in &mut s.entries {
                if !lines.iter().any(|l| l.entry_id == e.id) {
                    continue;
                }
                if lock {
                    if !e.is_invoiceable() {
                        bail!("entry {} is no longer invoiceable", e.id);
                    }
                    e.invoice = Some(number.to_string());
                } else if e.invoice.as_deref() == Some(number) {
                    e.invoice = None;
                }
            }
        }
        day.save()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(extra: &str) -> Config {
        Config::parse(&format!(
            r#"
            [business]
            name = "Me"
            {extra}
            [tasks.dev]
            name = "Dev"
            [clients.acme]
            name = "Acme"
            email_to = ["ap@acme.test"]
            [clients.acme.projects.web]
            name = "Web"
            rate = 150.0
            default_task = "dev"
            "#
        ))
        .unwrap()
    }

    fn inv(number: &str, year: i32, seq: u32) -> Invoice {
        let c = cfg("");
        let d = NaiveDate::from_ymd_opt(year, 1, 1).unwrap();
        let mut i = Invoice::build(&c, "acme", vec![], d, (d, d)).unwrap();
        i.number = number.into();
        i.seq = seq;
        i
    }

    #[test]
    fn number_formats() {
        assert_eq!(
            format_number("{year}-{seq:03}", 2026, 7).unwrap(),
            "2026-007"
        );
        assert_eq!(format_number("INV{seq}", 2026, 12).unwrap(), "INV12");
        assert!(format_number("{year}", 2026, 1).is_err(), "needs seq");
        assert!(
            format_number("{year}/{seq}", 2026, 1).is_err(),
            "unsafe char"
        );
        assert!(format_number("{nope}-{seq}", 2026, 1).is_err());
    }

    #[test]
    fn numbering_restarts_per_year_and_honors_start_seq() {
        let c = cfg("");
        let existing = vec![inv("2025-004", 2025, 4), inv("2026-002", 2026, 2)];
        assert_eq!(next_number(&c, 2026, &existing, &[]).unwrap().0, "2026-003");
        assert_eq!(next_number(&c, 2027, &existing, &[]).unwrap().0, "2027-001");

        let c = cfg("[invoice]\nstart_seq = 40\n");
        assert_eq!(next_number(&c, 2026, &existing, &[]).unwrap().0, "2026-040");
    }

    #[test]
    fn numbering_without_year_is_global() {
        let c = cfg("[invoice]\nnumber_format = \"INV-{seq:04}\"\n");
        let existing = vec![inv("INV-0009", 2025, 9)];
        assert_eq!(next_number(&c, 2026, &existing, &[]).unwrap().0, "INV-0010");
    }

    #[test]
    fn recipients_merge_cc_and_never_repeat_an_address() {
        let c = cfg(r#"[email]
            host = "smtp.example.com"
            username = "me"
            from = "Me <me@example.com>"
            cc = ["books@me.example", "AP@acme.test"]
            bcc = ["me@example.com", "Books <books@me.example>"]
            "#);
        let mut i = inv("DRAFT", 2026, 0);
        i.client.email_cc = vec!["Controller <controller@acme.test>".into()];
        let r = i.recipients(&c);
        assert_eq!(r.to, vec!["ap@acme.test"]);
        assert_eq!(
            r.cc,
            vec!["Controller <controller@acme.test>", "books@me.example"],
            "client cc first, then [email] cc; the To address is not repeated"
        );
        assert_eq!(r.bcc, vec!["me@example.com"], "already in Cc: not repeated");
    }

    #[test]
    fn a_one_off_cc_changes_the_fingerprint() {
        let c = cfg("");
        let a = inv("DRAFT", 2026, 0);
        let mut b = a.clone();
        b.client.email_cc.push("cfo@acme.test".into());
        assert_ne!(a.fingerprint(&c), b.fingerprint(&c));
    }

    #[test]
    fn payment_status_reads_paid_open_overdue_and_void() {
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let mut i = inv("036", 2026, 36);
        i.status = Status::Finalized;
        i.due_date = "2026-10-17".into();
        assert_eq!(
            i.payment_status(d("2026-10-01")),
            "not sent, due 2026-10-17"
        );
        i.sent.push(SendEvent {
            at: "2026-09-17T10:00:00Z".into(),
            to: vec![],
            cc: vec![],
            bcc: vec![],
        });
        assert_eq!(i.payment_status(d("2026-10-17")), "open, due 2026-10-17");
        assert_eq!(
            i.payment_status(d("2026-10-18")),
            "OVERDUE since 2026-10-17"
        );
        assert!(i.is_outstanding());
        i.paid_date = Some("2026-10-20".into());
        assert_eq!(i.payment_status(d("2026-11-01")), "paid 2026-10-20");
        assert!(!i.is_outstanding());
        i.status = Status::Void;
        assert_eq!(i.payment_status(d("2026-11-01")), "void");
    }

    #[test]
    fn records_without_the_new_fields_still_load() {
        // Records written before payment tracking existed.
        let mut v = serde_json::to_value(inv("2026-001", 2026, 1)).unwrap();
        let m = v.as_object_mut().unwrap();
        m.remove("paid_date");
        m.remove("source");
        m.remove("harvest");
        let i: Invoice = serde_json::from_value(v).unwrap();
        assert_eq!(i.source, Source::Jimtime);
        assert!(i.paid_date.is_none() && i.harvest.is_none());
    }

    #[test]
    fn continues_harvest_numbering() {
        // Harvest issued 032..036; jimtime has none yet.
        let c = cfg("[invoice]\nnumber_format = \"{seq:03}\"\n");
        let harvest: Vec<String> = (32..=36).map(|n| format!("{n:03}")).collect();
        assert_eq!(
            next_number(&c, 2026, &[], &harvest).unwrap(),
            ("037".into(), 37)
        );

        // Once jimtime is ahead, its own records win; if Harvest issues one
        // more meanwhile, that wins instead. Never a duplicate either way.
        let mine = vec![inv("037", 2026, 37)];
        assert_eq!(next_number(&c, 2026, &mine, &harvest).unwrap().0, "038");
        let mut more = harvest.clone();
        more.push("038".into());
        assert_eq!(next_number(&c, 2026, &mine, &more).unwrap().0, "039");
    }

    #[test]
    fn numbers_from_another_scheme_are_ignored() {
        let c = cfg("[invoice]\nnumber_format = \"{seq:03}\"\n");
        let other = vec!["INV-500".to_string(), "2024-9".into(), "".into()];
        assert_eq!(next_number(&c, 2026, &[], &other).unwrap().0, "001");
    }

    #[test]
    fn parse_reads_back_what_format_writes() {
        for fmt in [
            "{seq:03}",
            "{year}-{seq:03}",
            "INV-{seq}",
            "{year}{seq:04}x",
        ] {
            let t = tokens(fmt).unwrap();
            let n = format_number(fmt, 2026, 42).unwrap();
            let (year, seq) = parse_number(&t, &n).unwrap();
            assert_eq!(seq, 42, "{fmt} -> {n}");
            assert_eq!(year.is_some(), fmt.contains("{year}"), "{fmt}");
        }
        let t = tokens("{year}-{seq:03}").unwrap();
        assert_eq!(parse_number(&t, "2025-007"), Some((Some(2025), 7)));
        assert_eq!(parse_number(&t, "2025-007a"), None, "trailing junk");
        assert_eq!(parse_number(&t, "25-007"), None, "short year");
    }

    #[test]
    fn per_year_formats_only_count_the_same_year_from_harvest() {
        let c = cfg("");
        let harvest = vec!["2025-040".to_string(), "2026-003".into()];
        assert_eq!(next_number(&c, 2026, &[], &harvest).unwrap().0, "2026-004");
    }

    #[test]
    fn lines_are_rounded_and_the_total_is_their_sum() {
        let c = cfg("");
        let line = |h: f64| Line {
            entry_id: format!("e{h}"),
            date: "2026-09-01".into(),
            project: "web".into(),
            project_name: "Web".into(),
            task: "dev".into(),
            task_name: "Dev".into(),
            notes: "n".into(),
            hours: h,
            rate: 33.33,
            amount: round2(h * 33.33),
        };
        let d = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let i = Invoice::build(&c, "acme", vec![line(1.333), line(1.333)], d, (d, d)).unwrap();
        assert_eq!(i.lines[0].amount, 44.43);
        assert_eq!(i.total, 88.86, "sum of rounded lines, not round(sum)");
        assert_eq!(i.due_date, "2026-10-30");
    }

    #[test]
    fn fingerprint_tracks_content_and_recipients_but_not_the_number() {
        let c = cfg("");
        let a = inv("DRAFT", 2026, 0);
        let mut b = a.clone();
        b.number = "2026-001".into();
        b.issue_date = "2026-12-31".into();
        assert_eq!(a.fingerprint(&c), b.fingerprint(&c));

        let mut other = a.clone();
        other.client.email_to = vec!["someone@else.test".into()];
        assert_ne!(a.fingerprint(&c), other.fingerprint(&c));
    }
}
