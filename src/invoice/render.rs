//! Rendering an invoice: HTML from a MiniJinja template, then PDF by printing
//! it with a headless Chromium-family browser. [ADR-0007]

use anyhow::{Context, Result, bail};
use minijinja::{Environment, context};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::Invoice;
use crate::config::Config;
use crate::paths;
use crate::view::{fmt_amount, fmt_hours};

/// The built-in template, used when the config names none.
pub const DEFAULT_TEMPLATE: &str = include_str!("default.html");

/// A project/task subtotal, for templates that summarize before the detail.
#[derive(Serialize)]
struct Group {
    project_name: String,
    task_name: String,
    hours: f64,
    rate: f64,
    amount: f64,
}

fn groups(inv: &Invoice) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for l in &inv.lines {
        match out.iter_mut().find(|g| {
            g.project_name == l.project_name && g.task_name == l.task_name && g.rate == l.rate
        }) {
            Some(g) => {
                g.hours += l.hours;
                g.amount = super::round2(g.amount + l.amount);
            }
            None => out.push(Group {
                project_name: l.project_name.clone(),
                task_name: l.task_name.clone(),
                hours: l.hours,
                rate: l.rate,
                amount: l.amount,
            }),
        }
    }
    out
}

/// The symbol for common currencies; others print their code.
pub fn currency_symbol(code: &str) -> &str {
    match code {
        "USD" | "CAD" | "AUD" | "NZD" | "SGD" | "HKD" | "MXN" => "$",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" | "CNY" => "¥",
        "INR" => "₹",
        "CHF" => "CHF ",
        _ => code,
    }
}

fn environment() -> Environment<'static> {
    let mut env = Environment::new();
    env.add_filter("money", |v: f64| fmt_amount(v));
    env.add_filter("hours", |v: f64| fmt_hours(v));
    env.add_filter("css_string", |v: String| {
        minijinja::Value::from_safe_string(css_string(&v))
    });
    // Optional fields can be tested with `{% if %}`, but printing a typo'd
    // variable is an error rather than a silent blank on a client's invoice.
    env.set_undefined_behavior(minijinja::UndefinedBehavior::SemiStrict);
    env
}

/// Escape text for the inside of a double-quoted CSS string. HTML escaping is
/// wrong there (CSS would print `&lt;` literally), and `"`, `\`, a newline or
/// `</style>` would break the stylesheet.
fn css_string(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '"' | '\\' | '<' | '>' | '&' | '\n' | '\r' => {
                out.push_str(&format!("\\{:x} ", c as u32))
            }
            c => out.push(c),
        }
    }
    out
}

/// The context every template (invoice, email subject and body) sees.
fn ctx(inv: &Invoice, draft: bool) -> minijinja::Value {
    context! {
        invoice => inv,
        business => &inv.business,
        client => &inv.client,
        lines => &inv.lines,
        groups => groups(inv),
        currency_symbol => currency_symbol(&inv.currency),
        draft => draft,
    }
}

/// Render the invoice HTML. `template` is a file, or `None` for the built-in.
pub fn html(inv: &Invoice, template: Option<&Path>, draft: bool) -> Result<String> {
    let source = match template {
        Some(p) => std::fs::read_to_string(p)
            .with_context(|| format!("reading invoice template {}", p.display()))?,
        None => DEFAULT_TEMPLATE.to_string(),
    };
    let mut env = environment();
    // The .html name turns on HTML auto-escaping.
    env.add_template_owned("invoice.html", source)?;
    env.get_template("invoice.html")?
        .render(ctx(inv, draft))
        .map_err(|e| anyhow::anyhow!("rendering the invoice template: {e:#}"))
}

/// Render a plain-text template (email subject/body) with the invoice context.
pub fn text(source: &str, inv: &Invoice) -> Result<String> {
    let mut env = environment();
    env.add_template_owned("text.txt", source.to_string())?;
    env.get_template("text.txt")?
        .render(ctx(inv, false))
        .map_err(|e| anyhow::anyhow!("rendering an email template: {e:#}"))
}

/// Render the invoice to a PDF at `out`.
///
/// The HTML is written next to the template (or into the drafts dir for the
/// built-in one) so that relative links in a template - a logo, a stylesheet -
/// resolve the way they do when the template is opened in a browser.
pub async fn pdf(
    config: &Config,
    inv: &Invoice,
    template: Option<&Path>,
    draft: bool,
    out: &Path,
) -> Result<()> {
    let html = html(inv, template, draft)?;
    let dir = match template.and_then(Path::parent) {
        Some(d) => d.to_path_buf(),
        None => paths::drafts_dir()?,
    };
    std::fs::create_dir_all(&dir)?;
    let page = tempfile::Builder::new()
        .prefix(".jimtime-render-")
        .suffix(".html")
        .tempfile_in(&dir)
        .with_context(|| format!("creating a temp file in {}", dir.display()))?;
    std::fs::write(page.path(), html)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    print_to_pdf(config, page.path(), out).await
}

/// Print an HTML file to PDF with headless Chrome. CSS `@page` controls size
/// and margins.
///
/// Headless mode runs on its own temporary profile, so the user's browser
/// profile is never touched and a running Chrome is no obstacle. Passing an
/// explicit `--user-data-dir` makes Chrome write the PDF and then never exit
/// (seen on macOS), so we deliberately do not.
async fn print_to_pdf(config: &Config, html: &Path, out: &Path) -> Result<()> {
    let browser = find_browser(config)?;
    let url = reqwest::Url::from_file_path(html)
        .map_err(|_| anyhow::anyhow!("{} is not an absolute path", html.display()))?;
    let _ = std::fs::remove_file(out);
    let log = tempfile::NamedTempFile::new().context("creating a browser log file")?;

    let mut cmd = tokio::process::Command::new(&browser);
    cmd.arg("--headless")
        .arg("--disable-gpu")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-extensions")
        .arg("--no-pdf-header-footer")
        // Give web fonts and images time to load before printing.
        .arg("--virtual-time-budget=10000")
        .arg(format!("--print-to-pdf={}", out.display()))
        .arg(url.as_str())
        // Chrome's helper processes inherit stdio and outlive the browser, so
        // waiting on pipes would hang until they exit. Log to a file instead
        // and wait only for the browser itself.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(log.reopen()?)
        .kill_on_drop(true);
    let status = tokio::time::timeout(Duration::from_secs(90), cmd.status())
        .await
        .with_context(|| format!("{} took too long to print the invoice", browser.display()))?
        .with_context(|| format!("running {}", browser.display()))?;

    let ok = out.metadata().map(|m| m.len() > 0).unwrap_or(false);
    if !status.success() || !ok {
        let stderr = std::fs::read_to_string(log.path()).unwrap_or_default();
        bail!(
            "{} could not print the invoice to PDF:\n{}",
            browser.display(),
            stderr.trim()
        );
    }
    Ok(())
}

/// A Chromium-family browser: `invoice.chrome`, `$JIMTIME_CHROME`, the usual
/// macOS app bundles, then common names on `PATH`.
pub fn find_browser(config: &Config) -> Result<PathBuf> {
    if let Some(p) = &config.invoice.chrome {
        let p = crate::config::expand_tilde(p);
        if !p.exists() {
            bail!(
                "invoice.chrome is set to {}, which does not exist",
                p.display()
            );
        }
        return Ok(p);
    }
    if let Some(p) = std::env::var_os("JIMTIME_CHROME").filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        if !p.exists() {
            bail!(
                "JIMTIME_CHROME is set to {}, which does not exist",
                p.display()
            );
        }
        return Ok(p);
    }
    const APPS: &[&str] = &[
        "Google Chrome.app/Contents/MacOS/Google Chrome",
        "Chromium.app/Contents/MacOS/Chromium",
        "Brave Browser.app/Contents/MacOS/Brave Browser",
        "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ];
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }
    for root in &roots {
        for app in APPS {
            let p = root.join(app);
            if p.exists() {
                return Ok(p);
            }
        }
    }
    const NAMES: &[&str] = &[
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "brave-browser",
        "microsoft-edge",
    ];
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for name in NAMES {
                let p = dir.join(name);
                if p.is_file() {
                    return Ok(p);
                }
            }
        }
    }
    bail!(
        "no Chrome, Chromium, Brave or Edge found to print the PDF.\n\
         Install one, or point `invoice.chrome` (config) or $JIMTIME_CHROME at its binary."
    )
}

/// Open a file in the desktop's default viewer.
pub fn open(path: &Path) -> Result<()> {
    open_target(path.as_os_str())
}

/// Open a URL in the default browser.
pub fn open_url(url: &str) -> Result<()> {
    open_target(std::ffi::OsStr::new(url))
}

fn open_target(target: &std::ffi::OsStr) -> Result<()> {
    let path = Path::new(target);
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(path)
        .spawn()
        .with_context(|| format!("opening {} with {opener}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invoice::{Line, round2};
    use chrono::NaiveDate;

    fn invoice() -> Invoice {
        let c = Config::parse(
            r#"
            [business]
            name = "Jim <Consulting>"
            payment_instructions = "ACH"
            [tasks.dev]
            name = "Dev"
            [clients.acme]
            name = "Acme"
            address = "1 Way\nTown"
            [clients.acme.projects.web]
            name = "Web"
            rate = 150.0
            default_task = "dev"
            "#,
        )
        .unwrap();
        let line = |id: &str, h: f64| Line {
            entry_id: id.into(),
            date: "2026-09-01".into(),
            project: "web".into(),
            project_name: "Web".into(),
            task: "dev".into(),
            task_name: "Dev".into(),
            notes: "Built <things>".into(),
            hours: h,
            rate: 150.0,
            amount: round2(h * 150.0),
        };
        let d = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        Invoice::build(&c, "acme", vec![line("a", 1.5), line("b", 10.0)], d, (d, d)).unwrap()
    }

    #[test]
    fn default_template_renders_escaped_totals() {
        let html = html(&invoice(), None, false).unwrap();
        assert!(html.contains("1,725.00"), "total");
        assert!(
            html.contains("Jim &lt;Consulting&gt;"),
            "business name escaped"
        );
        assert!(html.contains("Built &lt;things&gt;"), "notes escaped");
        assert!(!html.contains("class=\"draft\""), "no watermark");
    }

    #[test]
    fn css_strings_are_escaped_for_css_not_html() {
        assert_eq!(
            css_string(r#"Jim "JN" <Co>"#),
            r#"Jim \22 JN\22  \3c Co\3e "#
        );
        let html = html(&invoice(), None, false).unwrap();
        assert!(html.contains(r#"content: "Jim \3c Consulting\3e  · Invoice DRAFT""#));
    }

    #[test]
    fn draft_is_marked() {
        let html = html(&invoice(), None, true).unwrap();
        assert!(html.contains("<div class=\"draft\">DRAFT</div>"));
    }

    #[test]
    fn groups_sum_same_project_task_rate() {
        let g = groups(&invoice());
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].hours, 11.5);
        assert_eq!(g[0].amount, 1725.0);
    }

    #[test]
    fn email_text_renders_with_filters() {
        let s = text(
            "Invoice {{ invoice.number }}: {{ currency_symbol }}{{ invoice.total | money }}",
            &invoice(),
        )
        .unwrap();
        assert_eq!(s, "Invoice DRAFT: $1,725.00");
    }

    #[test]
    fn a_typo_in_a_template_is_an_error_not_a_blank() {
        assert!(text("{{ invoice.nubmer }}", &invoice()).is_err());
    }
}
