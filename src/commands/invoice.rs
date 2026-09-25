use super::Command;
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::io::{IsTerminal, Write};
use std::path::Path;

use crate::cloud;
use crate::config::Config;
use crate::datarepo::{self, Sync};
use crate::harvest::HarvestApi;
use crate::daterange::RangeArgs;
use crate::invoice::mail::Mailer;
use crate::invoice::render::{self, currency_symbol};
use crate::invoice::{self as inv, Status};
use crate::paths;
use crate::timeutil;
use crate::view::{fmt_amount, fmt_hours};

/// Draft, finalize, send and void invoices built from approved billable time
///
/// The flow: `draft` renders a preview and prints a fingerprint; `finalize`
/// numbers the invoice after you approve it, locks its entries, saves the
/// PDF, emails it, and uploads it to any configured cloud folder.
#[derive(Args)]
pub struct Invoice {
    #[command(subcommand)]
    cmd: InvoiceCmd,
}

/// Which time an invoice bills.
#[derive(Args)]
struct Scope {
    /// Client key from the config
    #[arg(long)]
    client: String,
    /// Only this project (key) of the client
    #[arg(long)]
    project: Option<String>,
    #[command(flatten)]
    range: RangeArgs,
}

#[derive(Subcommand)]
enum InvoiceCmd {
    /// Render a preview PDF of what would be invoiced; changes nothing
    Draft {
        #[command(flatten)]
        scope: Scope,
        /// Do not open the PDF
        #[arg(long)]
        no_open: bool,
    },
    /// Number, save, lock, email and upload an invoice, after your approval
    ///
    /// Interactively, shows the PDF and asks. Otherwise requires
    /// `--confirm <fingerprint>` from `invoice draft`, and refuses if what
    /// would be billed has changed since.
    Finalize {
        #[command(flatten)]
        scope: Scope,
        /// The fingerprint printed by `invoice draft`
        #[arg(long)]
        confirm: Option<String>,
        /// Finalize without emailing (send later with `invoice send`)
        #[arg(long)]
        no_send: bool,
        /// Do not open the PDF for the interactive preview
        #[arg(long)]
        no_open: bool,
    },
    /// Email a finalized invoice (again)
    Send {
        number: String,
        /// Also send to this address (repeatable)
        #[arg(long)]
        to: Vec<String>,
    },
    /// Void an invoice: its entries become invoiceable again; the number stays used
    Void { number: String },
    /// List invoices
    List {
        /// Only this client (key)
        #[arg(long)]
        client: Option<String>,
    },
    /// Open an invoice's PDF
    Open { number: String },
}

#[async_trait::async_trait]
impl Command for Invoice {
    async fn run(&self) -> Result<()> {
        match &self.cmd {
            InvoiceCmd::Draft { scope, no_open } => draft(scope, *no_open).await,
            InvoiceCmd::Finalize {
                scope,
                confirm,
                no_send,
                no_open,
            } => finalize(scope, confirm.as_deref(), *no_send, *no_open).await,
            InvoiceCmd::Send { number, to } => send(number, to).await,
            InvoiceCmd::Void { number } => void(number),
            InvoiceCmd::List { client } => list(client.as_deref()),
            InvoiceCmd::Open { number } => {
                let i = inv::Invoice::load(number)?;
                render::open(&i.pdf_path()?)
            }
        }
    }
}

/// Select and price the time in scope, reporting what was held back.
fn build(config: &Config, scope: &Scope) -> Result<inv::Invoice> {
    if !scope.range.is_set() {
        bail!(
            "choose the period to bill, e.g. --last-month, --month, or --from YYYY-MM-DD --to YYYY-MM-DD"
        );
    }
    let (from, to) = scope.range.resolve()?;
    let sel = inv::select(
        config,
        &scope.client,
        scope.project.as_deref(),
        &timeutil::dates_between(from, to),
    )?;
    if sel.held_unapproved > 0 {
        eprintln!(
            "note: {} billable entr{} in this period {} unapproved{} and not included \
             (see `jimtime review --pending --client {} --from {} --to {}`).",
            sel.held_unapproved,
            if sel.held_unapproved == 1 { "y" } else { "ies" },
            if sel.held_unapproved == 1 { "is" } else { "are" },
            if sel.held_needs_review > 0 {
                format!(", {} flagged needs-review", sel.held_needs_review)
            } else {
                String::new()
            },
            scope.client,
            inv::fmt_date(from),
            inv::fmt_date(to),
        );
    }
    if sel.lines.is_empty() {
        bail!(
            "nothing to invoice for {} from {} to {}: no approved, billable, un-invoiced time",
            scope.client,
            inv::fmt_date(from),
            inv::fmt_date(to)
        );
    }
    if sel.harvest_linked > 0 {
        eprintln!(
            "WARNING: {} of these entr{} {} also pushed to Harvest. If you invoice this client from \
             Harvest too, this double-bills them.",
            sel.harvest_linked,
            if sel.harvest_linked == 1 { "y" } else { "ies" },
            if sel.harvest_linked == 1 { "was" } else { "were" },
        );
    }
    inv::Invoice::build(
        config,
        &scope.client,
        sel.lines,
        timeutil::today_naive()?,
        (from, to),
    )
}

fn summary(config: &Config, i: &inv::Invoice, send: bool) {
    println!("Invoice {} for {} ({})", i.number, i.client.name, i.client_key);
    println!("  Period:      {} to {}", i.period_from, i.period_to);
    println!("  Issued/due:  {} / {}", i.issue_date, i.due_date);
    println!(
        "  Lines:       {} entr{}, {}h",
        i.lines.len(),
        if i.lines.len() == 1 { "y" } else { "ies" },
        fmt_hours(i.total_hours)
    );
    println!(
        "  Total:       {}{} {}",
        currency_symbol(&i.currency),
        fmt_amount(i.total),
        i.currency
    );
    if send {
        let r = i.recipients(config);
        println!("  Email to:    {}", list_or_none(&r.to));
        if !r.cc.is_empty() {
            println!("  Cc:          {}", r.cc.join(", "));
        }
        if !r.bcc.is_empty() {
            println!("  Bcc:         {}", r.bcc.join(", "));
        }
    }
}

fn list_or_none(v: &[String]) -> String {
    if v.is_empty() {
        "(none)".into()
    } else {
        v.join(", ")
    }
}

/// The next invoice number, continuing past Harvest's numbers when
/// `invoice.harvest_numbering` is on. If Harvest cannot be read then, this
/// fails rather than risk issuing a number Harvest already used.
async fn next_number(config: &Config, year: i32) -> Result<(String, u32)> {
    let external = if config.invoice.harvest_numbering {
        HarvestApi::from_env()
            .context("invoice.harvest_numbering is on, so Harvest credentials are needed")?
            .invoice_numbers()
            .await
            .context(
                "reading Harvest's invoice numbers (invoice.harvest_numbering); \
                 refusing to guess the next number",
            )?
    } else {
        Vec::new()
    };
    inv::next_number(config, year, &inv::all()?, &external)
}

async fn draft(scope: &Scope, no_open: bool) -> Result<()> {
    let config = Config::load()?;
    let i = build(&config, scope)?;
    let client = config.client(&scope.client)?;
    let template = config.template_for(client)?;
    let out = paths::drafts_dir()?.join(format!("{}-draft.pdf", scope.client));
    render::pdf(&config, &i, template.as_deref(), true, &out).await?;

    summary(&config, &i, true);
    let fp = i.fingerprint(&config);
    println!("  Fingerprint: {fp}");
    // Informational: finalize pulls first and computes it again.
    match next_number(&config, i.year).await {
        Ok((n, _)) => println!("  Next number: {n}"),
        Err(e) => eprintln!("warning: could not work out the next invoice number: {e:#}"),
    }
    println!("\nPreview: {}", out.display());
    if !no_open {
        render::open(&out)?;
    }
    println!(
        "\nNothing was saved. To finalize exactly this invoice:\n  jimtime invoice finalize --client {}{} --from {} --to {} --confirm {fp}",
        scope.client,
        scope
            .project
            .as_ref()
            .map(|p| format!(" --project {p}"))
            .unwrap_or_default(),
        i.period_from,
        i.period_to,
    );
    Ok(())
}

async fn finalize(scope: &Scope, confirm: Option<&str>, no_send: bool, no_open: bool) -> Result<()> {
    let config = Config::load()?;
    // Everything that could stop the send is checked before anything is written.
    let mailer = if no_send {
        None
    } else {
        Some(Mailer::from_config(&config)?)
    };
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if confirm.is_none() && !interactive {
        bail!(
            "finalize needs your approval: run `jimtime invoice draft` first, look at the PDF, \
             then pass --confirm <fingerprint>"
        );
    }

    // Numbers must never collide, so this must see the latest invoices.
    let sync = Sync::begin("invoice finalize", true)?;
    let mut i = build(&config, scope)?;
    let (number, seq) = next_number(&config, i.year).await?;
    i.number = number.clone();
    i.seq = seq;
    i.status = Status::Finalized;
    if let Some(m) = &mailer {
        m.check_recipients(&config, &i, &[])?;
    }
    let fp = i.fingerprint(&config);

    let client = config.client(&scope.client)?;
    let template = config.template_for(client)?;
    let staged = paths::drafts_dir()?.join(format!("{number}.pdf"));
    render::pdf(&config, &i, template.as_deref(), false, &staged).await?;

    match confirm {
        Some(c) if c != fp => bail!(
            "what would be billed has changed since the draft (fingerprint {c}, now {fp}).\n\
             Run `jimtime invoice draft` again and check it."
        ),
        Some(_) => summary(&config, &i, mailer.is_some()),
        None => {
            summary(&config, &i, mailer.is_some());
            println!("\nPreview: {}", staged.display());
            if !no_open {
                render::open(&staged)?;
            }
            let action = if mailer.is_some() {
                format!("Finalize invoice {number} and email it")
            } else {
                format!("Finalize invoice {number} (without emailing)")
            };
            if !ask(&format!("\n{action}? [y/N] "))? {
                let _ = std::fs::remove_file(&staged);
                println!("Not finalized. Nothing was saved.");
                return Ok(());
            }
        }
    }

    // The point of no return: lock the entries, then record the invoice.
    inv::set_entry_invoice(&i.lines, &number, true)?;
    i.save()?;
    let pdf = i.pdf_path()?;
    move_file(&staged, &pdf)?;
    datarepo::note_write(&pdf);
    sync.checkpoint(&format!(
        "invoice: finalize {number} for {} ({}{})",
        i.client_key,
        currency_symbol(&i.currency),
        fmt_amount(i.total)
    ))?;
    println!("\nFinalized invoice {number}: {}", pdf.display());

    if let Some(m) = &mailer {
        match m.send(&config, &i, &pdf, &[]).await {
            Ok(ev) => {
                println!("Emailed to {}", ev.to.join(", "));
                i.sent.push(ev);
                i.save()?;
            }
            Err(e) => {
                sync.commit(&format!("invoice: {number} finalized, not sent"))?;
                return Err(e.context(format!(
                    "invoice {number} is finalized but was NOT emailed; retry with `jimtime invoice send {number}`"
                )));
            }
        }
    }

    upload(&config, &mut i, &pdf).await?;
    sync.commit(&format!("invoice: {number} issued"))
}

/// Upload to configured cloud folders, recording successes and warning about
/// failures without failing the command.
async fn upload(config: &Config, i: &mut inv::Invoice, pdf: &Path) -> Result<()> {
    let before = i.uploads.len();
    for (p, e) in cloud::upload_all(config, i, pdf).await {
        eprintln!(
            "warning: could not upload to {}: {e:#}\n  Retry with `jimtime cloud upload {}`",
            p.name(),
            i.number
        );
    }
    if i.uploads.len() != before {
        i.save()?;
    }
    Ok(())
}

async fn send(number: &str, extra_to: &[String]) -> Result<()> {
    let config = Config::load()?;
    let mailer = Mailer::from_config(&config)?;
    let sync = Sync::begin("invoice send", false)?;
    let mut i = inv::Invoice::load(number)?;
    if i.status == Status::Void {
        bail!("invoice {number} is void");
    }
    mailer.check_recipients(&config, &i, extra_to)?;
    let pdf = i.pdf_path()?;
    let ev = mailer.send(&config, &i, &pdf, extra_to).await?;
    println!("Emailed invoice {number} to {}", ev.to.join(", "));
    i.sent.push(ev);
    i.save()?;
    sync.commit(&format!("invoice: {number} sent"))
}

fn void(number: &str) -> Result<()> {
    let sync = Sync::begin("invoice void", false)?;
    let mut i = inv::Invoice::load(number)?;
    if i.status == Status::Void {
        bail!("invoice {number} is already void");
    }
    inv::set_entry_invoice(&i.lines, number, false)?;
    i.status = Status::Void;
    i.voided_at = Some(timeutil::now_rfc3339()?);
    i.save()?;
    sync.commit(&format!("invoice: void {number}"))?;

    println!(
        "Voided invoice {number}. Its {} entr{} can be invoiced again; the number stays used.",
        i.lines.len(),
        if i.lines.len() == 1 { "y" } else { "ies" }
    );
    if let Some(last) = i.sent.last() {
        println!(
            "It was emailed to {} on {} - let them know it is void.",
            last.to.join(", "),
            last.at
        );
    }
    Ok(())
}

fn list(client: Option<&str>) -> Result<()> {
    let all: Vec<inv::Invoice> = inv::all()?
        .into_iter()
        .filter(|i| client.is_none_or(|c| i.client_key == c))
        .collect();
    if all.is_empty() {
        println!("No invoices.");
        return Ok(());
    }
    println!(
        "{:<12} {:<10}  {:<24} {:>14}  STATUS",
        "NUMBER", "ISSUED", "CLIENT", "TOTAL"
    );
    for i in &all {
        let status = match i.status {
            Status::Void => "void".to_string(),
            _ if i.sent.is_empty() => "not sent".to_string(),
            _ => format!("sent {}", &i.sent.last().expect("non-empty").at[..10]),
        };
        let uploaded = if i.uploads.is_empty() { "" } else { ", uploaded" };
        println!(
            "{:<12} {:<10}  {:<24} {:>14}  {status}{uploaded}",
            i.number,
            i.issue_date,
            truncate(&i.client.name, 24),
            format!("{} {}", fmt_amount(i.total), i.currency),
        );
    }
    Ok(())
}

fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(width - 1).collect();
        out.push('…');
        out
    }
}

fn ask(question: &str) -> Result<bool> {
    print!("{question}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "Yes"))
}

fn move_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(from, to)
        .or_else(|_| std::fs::copy(from, to).map(|_| ()).and_then(|_| std::fs::remove_file(from)))
        .with_context(|| format!("moving the PDF to {}", to.display()))
}
