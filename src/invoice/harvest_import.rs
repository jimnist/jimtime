//! Importing invoicing history from Harvest, so it survives Harvest being
//! turned off. [ADR-0011]
//!
//! Read-only against Harvest. Re-runnable: records it created are refreshed
//! (state, paid date), entries already linked are left as they are.

use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, HashMap};

use super::{HarvestSnapshot, Invoice, Line, Party, SendEvent, Source, Status, round2, seq_of};

use crate::config::{Business, Config};
use crate::datarepo;
use crate::harvest::{HarvestApi, HarvestInvoice, TimeEntry};
use crate::paths;
use crate::store::{Day, Section, day_files, write_atomic};

/// What an import did (or, dry, would do).
#[derive(Default, Debug)]
pub struct Report {
    pub records_new: Vec<String>,
    /// Already imported, and changed in Harvest since (e.g. now paid).
    pub records_refreshed: Vec<String>,
    /// Already imported and unchanged: nothing written.
    pub records_unchanged: Vec<String>,
    pub drafts_skipped: Vec<String>,
    /// Local entries newly locked to the Harvest invoice that billed them.
    pub locked: usize,
    /// Harvest-only entries added to the store, already invoiced.
    pub backfilled_invoiced: usize,
    /// Harvest-only entries added to the store, not invoiced (need review).
    pub backfilled_unbilled: usize,
}

/// The config keys for a Harvest client, looked up by its id in harvest.toml.
fn client_key(config: &Config, id: u64, name: &str) -> Result<String> {
    config
        .client_for_harvest(id)
        .map(str::to_string)
        .with_context(|| {
            format!("Harvest client {name:?} (id {id}) is not under [clients.*] in harvest.toml")
        })
}

/// The config keys for a Harvest entry's client, project and task, looked up
/// by their ids in harvest.toml.
fn keys_for(config: &Config, te: &TimeEntry) -> Result<(String, String, String)> {
    let ck = client_key(config, te.client.id, &te.client.name)?;
    let pk = config
        .project_for_harvest(&ck, te.project.id)
        .with_context(|| {
            format!(
                "Harvest project {:?} (id {}) is not under [clients.{ck}.projects.*] in harvest.toml",
                te.project.name, te.project.id
            )
        })?
        .to_string();
    let tk = config
        .task_for_harvest(te.task.id)
        .with_context(|| {
            format!(
                "Harvest task {:?} (id {}) is not under [tasks.*] in harvest.toml",
                te.task.name, te.task.id
            )
        })?
        .to_string();
    Ok((ck, pk, tk))
}

/// The section a backfilled entry goes in: the config's names, and the repo
/// mapped to that client and project (if one is).
fn section_for(config: &Config, te: &TimeEntry) -> Result<Section> {
    let (ck, pk, tk) = keys_for(config, te)?;
    let client = &config.clients[&ck];
    let project = &client.projects[&pk];
    let repo_path = config
        .repos
        .iter()
        .find(|r| r.client == ck && r.project == pk)
        .map(|r| {
            crate::repo::canonical(crate::config::expand_tilde(&r.path))
                .display()
                .to_string()
        })
        .unwrap_or_default();
    Ok(Section {
        repo_path,
        client_name: client.name.clone(),
        project_name: project.name.clone(),
        task_name: config.tasks[&tk].name.clone(),
        harvest_client_id: Some(te.client.id),
        harvest_project_id: Some(te.project.id),
        harvest_task_id: Some(te.task.id),
        client: ck,
        project: pk,
        task: tk,
        ..Section::default()
    })
}

fn number_of(inv: &HarvestInvoice) -> Result<&str> {
    inv.number
        .as_deref()
        .with_context(|| format!("Harvest invoice id {} has no number", inv.id))
}

/// Build the jimtime record of a Harvest invoice.
///
/// `lines` are the time it billed (one per entry, at the entry's rate); the
/// money is Harvest's: `total` is its amount and `harvest` its line items.
#[allow(clippy::too_many_arguments)]
fn record_for(
    config: &Config,
    business_name: &str,
    hinv: &HarvestInvoice,
    entries: &[(&TimeEntry, String)],
    address: Option<String>,
    existing: Option<&Invoice>,
) -> Result<Invoice> {
    let number = number_of(hinv)?;
    let issue = hinv
        .issue_date
        .clone()
        .with_context(|| format!("Harvest invoice {number} has no issue date"))?;
    let issue_year: i32 = issue[..4].parse().context("bad issue date")?;
    let (year, seq) = match seq_of(config, number)? {
        Some((y, s)) => (y.unwrap_or(issue_year), s),
        None => bail!(
            "Harvest invoice number {number:?} does not fit invoice.number_format {:?}; \
             set the format to match Harvest's (e.g. \"{{seq:03}}\" for 036) and re-run",
            config.invoice.number_format
        ),
    };
    let client_key = client_key(config, hinv.client.id, &hinv.client.name)?;

    let mut lines: Vec<Line> = Vec::new();
    for (te, local_id) in entries {
        let (_, pk, tk) = keys_for(config, te)?;
        let project = &config.clients[&client_key].projects[&pk];
        let rate = te.billable_rate.unwrap_or(0.0);
        lines.push(Line {
            entry_id: local_id.clone(),
            date: te.spent_date.clone(),
            project: pk.clone(),
            project_name: project.name.clone(),
            task: tk.clone(),
            task_name: config.tasks[&tk].name.clone(),
            notes: te.notes.clone().unwrap_or_default(),
            hours: te.hours,
            rate,
            amount: round2(te.hours * rate),
        });
    }
    lines.sort_by(|a, b| (&a.date, &a.entry_id).cmp(&(&b.date, &b.entry_id)));

    // A payment recorded here survives a re-import that has none.
    let paid_date = hinv
        .paid_date
        .clone()
        .or_else(|| existing.and_then(|e| e.paid_date.clone()));
    Ok(Invoice {
        number: number.to_string(),
        year,
        seq,
        status: Status::Finalized,
        client_key,
        client: Party {
            name: hinv.client.name.clone(),
            address,
            email_to: Vec::new(),
            email_cc: Vec::new(),
        },
        business: Business {
            name: business_name.to_string(),
            ..Business::default()
        },
        currency: hinv.currency.clone(),
        due_date: hinv.due_date.clone().unwrap_or_else(|| issue.clone()),
        period_from: hinv.period_start.clone().unwrap_or_else(|| issue.clone()),
        period_to: hinv.period_end.clone().unwrap_or_else(|| issue.clone()),
        issue_date: issue,
        total_hours: lines.iter().map(|l| l.hours).sum::<f64>() + 0.0,
        lines,
        total: hinv.amount,
        notes: hinv.notes.clone().filter(|n| !n.trim().is_empty()),
        sent: hinv
            .sent_at
            .iter()
            .map(|at| SendEvent {
                at: at.clone(),
                to: Vec::new(),
                cc: Vec::new(),
                bcc: Vec::new(),
            })
            .collect(),
        uploads: existing.map(|e| e.uploads.clone()).unwrap_or_default(),
        voided_at: None,
        paid_date,
        source: Source::Harvest,
        harvest: Some(HarvestSnapshot {
            id: hinv.id,
            state: hinv.state.clone(),
            subject: hinv.subject.clone(),
            line_items: hinv.line_items.clone(),
            discount_percent: hinv.discount,
            discount_amount: hinv.discount_amount,
            tax_percent: hinv.tax,
            tax_amount: hinv.tax_amount,
            tax2_percent: hinv.tax2,
            tax2_amount: hinv.tax2_amount,
        }),
    })
}

/// A local entry that came from (was pushed to) Harvest.
struct Local {
    date: String,
    id: String,
    invoice: Option<String>,
}

/// Every local entry with a Harvest id, by that id.
fn local_index() -> Result<HashMap<u64, Local>> {
    let mut out = HashMap::new();
    for path in day_files(&paths::entries_dir()?)? {
        let day = Day::load_path(&path)?;
        for (_, e) in day
            .sections
            .iter()
            .flat_map(|s| s.entries.iter().map(move |e| (s, e)))
        {
            if let Some(hid) = e.harvest_time_entry_id {
                out.insert(
                    hid,
                    Local {
                        date: day.date.clone(),
                        id: e.id.clone(),
                        invoice: e.invoice.clone(),
                    },
                );
            }
        }
    }
    Ok(out)
}

/// Import everything. With `dry_run`, only report what would change.
pub async fn run(config: &Config, api: &HarvestApi, dry_run: bool) -> Result<Report> {
    let me = api.me().await?;
    let (company, domain) = api.company().await?;
    let invoices = api.invoices().await?;
    // Only your own time: the query is filtered by user, and this makes sure
    // an admin token on a shared account can never import someone else's.
    let entries: Vec<TimeEntry> = api
        .time_entries(me)
        .await?
        .into_iter()
        .filter(|te| te.user.id == me)
        .collect();
    let mut report = Report::default();

    // Check everything up front, so a half-import never happens for a reason
    // that was knowable before writing.
    let mut problems = Vec::new();
    for i in &invoices {
        match i.state.as_str() {
            "draft" => report
                .drafts_skipped
                .push(i.number.clone().unwrap_or_default()),
            "open" | "paid" => {
                if let Err(e) = number_of(i) {
                    problems.push(format!("{e:#}"));
                }
            }
            s => problems.push(format!(
                "Harvest invoice {} is {s:?}, which the import does not handle yet",
                i.number.as_deref().unwrap_or("?")
            )),
        }
    }
    for te in &entries {
        if let Err(e) = keys_for(config, te) {
            let msg = format!("{e:#}");
            if !problems.contains(&msg) {
                problems.push(msg);
            }
        }
    }
    let existing: HashMap<String, Invoice> = super::all()?
        .into_iter()
        .map(|i| (i.number.clone(), i))
        .collect();
    for i in invoices.iter().filter(|i| i.state != "draft") {
        let n = number_of(i)?;
        if existing.get(n).is_some_and(|e| e.source != Source::Harvest) {
            problems.push(format!(
                "invoice {n} exists in both Harvest and jimtime; resolve that by hand first"
            ));
        }
    }
    let mut local = local_index()?;
    for te in &entries {
        if let (Some(l), Some(link)) = (local.get(&te.id), &te.invoice) {
            let theirs = link.number.as_deref().unwrap_or_default();
            if l.invoice.as_deref().is_some_and(|mine| mine != theirs) {
                problems.push(format!(
                    "entry {} is on invoice {} here but on {theirs} in Harvest",
                    l.id,
                    l.invoice.as_deref().unwrap_or_default()
                ));
            }
        }
    }
    if !problems.is_empty() {
        bail!("nothing was imported:\n  {}", problems.join("\n  "));
    }

    let issued = |te: &TimeEntry| -> Option<String> {
        let link = te.invoice.as_ref()?;
        invoices
            .iter()
            .find(|i| i.id == link.id && i.state != "draft")
            .and_then(|i| i.number.clone())
    };

    // 1. Backfill Harvest-only entries, oldest first so ids are stable.
    let mut missing: Vec<&TimeEntry> = entries
        .iter()
        .filter(|te| !local.contains_key(&te.id))
        .collect();
    missing.sort_by(|a, b| (&a.spent_date, a.id).cmp(&(&b.spent_date, b.id)));
    let mut by_date: BTreeMap<&str, Vec<&TimeEntry>> = BTreeMap::new();
    for te in missing {
        by_date.entry(te.spent_date.as_str()).or_default().push(te);
    }
    for (date, tes) in &by_date {
        let mut day = Day::load_or_new(date)?;
        for te in tes {
            let number = issued(te);
            if number.is_some() {
                report.backfilled_invoiced += 1;
            } else {
                report.backfilled_unbilled += 1;
            }
            let id = day.add_entry(
                section_for(config, te)?,
                te.hours,
                te.billable,
                number.is_none(),
                te.notes.clone().unwrap_or_default(),
            );
            let e = find_mut(&mut day, &id);
            // Billed in Harvest means approved in all but name; unbilled time
            // is left for a human to look at.
            e.approved = number.is_some();
            e.harvest_time_entry_id = Some(te.id);
            e.invoice = number.clone();
            local.insert(
                te.id,
                Local {
                    date: date.to_string(),
                    id,
                    invoice: number,
                },
            );
        }
        if !dry_run {
            day.save()?;
        }
    }

    // 2. Lock local entries Harvest has invoiced.
    let mut locks: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for te in &entries {
        let (Some(l), Some(number)) = (local.get(&te.id), issued(te)) else {
            continue;
        };
        if l.invoice.is_none() {
            locks
                .entry(l.date.clone())
                .or_default()
                .push((l.id.clone(), number));
        }
    }
    for (date, ids) in &locks {
        report.locked += ids.len();
        if dry_run {
            continue;
        }
        let mut day = Day::load(date)?.with_context(|| format!("day {date} disappeared"))?;
        for (id, number) in ids {
            let e = find_mut(&mut day, id);
            e.invoice = Some(number.clone());
            e.approved = true;
            e.needs_review = false;
        }
        day.save()?;
    }

    // 3. The invoice records and their PDFs.
    let mut addresses: HashMap<u64, Option<String>> = HashMap::new();
    for hinv in invoices.iter().filter(|i| i.state != "draft") {
        let number = number_of(hinv)?.to_string();
        let address = match addresses.get(&hinv.client.id) {
            Some(a) => a.clone(),
            None => {
                let a = api.client_address(hinv.client.id).await?;
                addresses.insert(hinv.client.id, a.clone());
                a
            }
        };
        let billed: Vec<(&TimeEntry, String)> = entries
            .iter()
            .filter(|te| te.invoice.as_ref().is_some_and(|l| l.id == hinv.id))
            .map(|te| (te, local[&te.id].id.clone()))
            .collect();
        let record = record_for(
            config,
            &company,
            hinv,
            &billed,
            address,
            existing.get(&number),
        )?;
        let pdf_path = record.pdf_path()?;
        // Harvest renders a fresh PDF (new timestamps) on every download, so
        // only fetch one when the invoice changed or its PDF is missing;
        // otherwise every re-run would commit five new binaries.
        match existing.get(&number) {
            None => report.records_new.push(number.clone()),
            Some(prior) if same_record(prior, &record) && pdf_path.exists() => {
                report.records_unchanged.push(number.clone());
                continue;
            }
            Some(_) => report.records_refreshed.push(number.clone()),
        }
        if dry_run {
            continue;
        }
        let pdf = api
            .invoice_pdf(&domain, &hinv.client_key)
            .await
            .with_context(|| format!("downloading the PDF of Harvest invoice {number}"))?;
        write_atomic(&pdf_path, &pdf)?;
        datarepo::note_write(&pdf_path);
        record.save()?;
    }
    Ok(report)
}

/// Whether two records say the same thing, as written to disk.
fn same_record(a: &Invoice, b: &Invoice) -> bool {
    serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
}

fn find_mut<'a>(day: &'a mut Day, id: &str) -> &'a mut crate::store::Entry {
    day.sections
        .iter_mut()
        .flat_map(|s| s.entries.iter_mut())
        .find(|e| e.id == id)
        .expect("the entry was just found or added")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harvest::{IdOnly, InvoiceLineItem, InvoiceLink, Named};

    fn config() -> Config {
        Config::parse_with_harvest(
            r#"
            [invoice]
            number_format = "{seq:03}"
            [tasks.programming]
            name = "Programming"
            [clients.mm]
            name = "Magic Mind"
            [clients.mm.projects.auto]
            name = "Automations"
            default_task = "programming"
            rate = 150.0
            [[repos]]
            path = "/nonexistent/mm"
            client = "mm"
            project = "auto"
            "#,
            r#"
            [tasks.programming]
            id = 30
            [clients.mm]
            id = 10
            [clients.mm.projects.auto]
            id = 20
            "#,
        )
        .unwrap()
    }

    fn te(id: u64, hours: f64, invoice: Option<(u64, &str)>) -> TimeEntry {
        TimeEntry {
            id,
            spent_date: "2026-03-02".into(),
            hours,
            notes: Some(format!("work {id}")),
            billable: true,
            billable_rate: Some(150.0),
            invoice: invoice.map(|(id, n)| InvoiceLink {
                id,
                number: Some(n.into()),
            }),
            client: Named {
                id: 10,
                name: "Magic Mind".into(),
            },
            project: Named {
                id: 20,
                name: "Automations".into(),
            },
            task: Named {
                id: 30,
                name: "Programming".into(),
            },
            user: IdOnly { id: 1 },
        }
    }

    fn hinv(amount: f64) -> HarvestInvoice {
        HarvestInvoice {
            id: 99,
            number: Some("034".into()),
            client: Named {
                id: 10,
                name: "Magic Mind".into(),
            },
            client_key: "k".into(),
            amount,
            currency: "USD".into(),
            state: "paid".into(),
            issue_date: Some("2026-07-24".into()),
            due_date: Some("2026-08-23".into()),
            period_start: None,
            period_end: None,
            subject: Some("July".into()),
            notes: None,
            sent_at: Some("2026-07-24T17:00:00Z".into()),
            paid_date: Some("2026-08-01".into()),
            discount: Some(25.0),
            discount_amount: 112.5,
            tax: None,
            tax_amount: 0.0,
            tax2: None,
            tax2_amount: 0.0,
            line_items: vec![InvoiceLineItem {
                kind: "Service".into(),
                description: Some("edited".into()),
                quantity: 2.25,
                unit_price: 150.0,
                amount: 337.5,
            }],
        }
    }

    #[test]
    fn keys_resolve_by_harvest_id_and_unknown_ids_are_named() {
        let c = config();
        assert_eq!(
            keys_for(&c, &te(1, 1.0, None)).unwrap(),
            ("mm".into(), "auto".into(), "programming".into())
        );
        let mut t = te(1, 1.0, None);
        t.task.id = 31;
        let err = keys_for(&c, &t).unwrap_err().to_string();
        assert!(err.contains("Harvest task") && err.contains("31"), "{err}");
    }

    #[test]
    fn the_record_keeps_harvests_money_and_links_the_time() {
        let c = config();
        let t = te(7, 2.83, Some((99, "034")));
        let r = record_for(
            &c,
            "engine",
            &hinv(337.5),
            &[(&t, "local-7".into())],
            None,
            None,
        )
        .unwrap();
        assert_eq!((r.number.as_str(), r.seq, r.year), ("034", 34, 2026));
        assert_eq!(r.source, Source::Harvest);
        assert_eq!(
            r.total, 337.5,
            "Harvest's amount, discount and edits included"
        );
        assert_eq!(r.lines[0].entry_id, "local-7");
        assert_eq!(r.lines[0].hours, 2.83, "the time as tracked");
        assert_eq!(r.paid_date.as_deref(), Some("2026-08-01"));
        let h = r.harvest.unwrap();
        assert_eq!(h.line_items[0].quantity, 2.25, "the line as billed");
        assert_eq!(h.discount_amount, 112.5);
        assert_eq!(r.period_from, "2026-07-24", "no period: the issue date");
    }

    #[test]
    fn reimporting_an_unchanged_invoice_builds_the_same_record() {
        // What keeps a re-run from rewriting (and re-committing) anything.
        let c = config();
        let t = te(7, 2.83, Some((99, "034")));
        let first = record_for(&c, "e", &hinv(337.5), &[(&t, "l7".into())], None, None).unwrap();
        let again = record_for(
            &c,
            "e",
            &hinv(337.5),
            &[(&t, "l7".into())],
            None,
            Some(&first),
        )
        .unwrap();
        assert!(same_record(&first, &again));

        let mut paid_now = hinv(337.5);
        paid_now.paid_date = Some("2026-09-25".into());
        let changed =
            record_for(&c, "e", &paid_now, &[(&t, "l7".into())], None, Some(&first)).unwrap();
        assert!(!same_record(&first, &changed));
    }

    #[test]
    fn a_local_payment_survives_a_reimport_without_one() {
        let c = config();
        let mut open = hinv(1.0);
        open.paid_date = None;
        let mut prior = record_for(&c, "e", &open, &[], None, None).unwrap();
        prior.paid_date = Some("2026-10-01".into());
        let again = record_for(&c, "e", &open, &[], None, Some(&prior)).unwrap();
        assert_eq!(again.paid_date.as_deref(), Some("2026-10-01"));
    }

    #[test]
    fn a_number_outside_the_format_is_an_error_naming_the_fix() {
        let mut c = config();
        c.invoice.number_format = "{year}-{seq:03}".into();
        let err = record_for(&c, "e", &hinv(1.0), &[], None, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("number_format"), "{err}");
    }

    #[test]
    fn backfilled_sections_use_config_names_keys_and_ids() {
        let s = section_for(&config(), &te(1, 1.0, None)).unwrap();
        assert_eq!(s.key(), ("/nonexistent/mm", "mm", "auto", "programming"));
        assert_eq!(s.label(), "Magic Mind - Automations - Programming");
        assert_eq!(s.harvest_task_id, Some(30));
    }
}
