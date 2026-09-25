//! The config file (`config/jimtime.toml`): the business, clients, projects,
//! tasks, repo mappings, and the invoice/email/git/Harvest/cloud settings.
//!
//! Clients, projects and tasks are identified by their **keys** (the TOML table
//! names). Harvest ids are optional attributes. [ADR-0006] Nothing here is
//! secret: passwords and tokens come from the environment or the keychain.
//! [ADR-0003, ADR-0010]

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::paths;
use crate::repo;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub business: Business,
    #[serde(default)]
    pub invoice: InvoiceSettings,
    pub email: Option<EmailSettings>,
    #[serde(default)]
    pub harvest: HarvestSettings,
    #[serde(default)]
    pub git: GitSettings,
    #[serde(default)]
    pub cloud: CloudSettings,
    #[serde(default)]
    pub clients: BTreeMap<String, Client>,
    #[serde(default)]
    pub tasks: BTreeMap<String, Task>,
    #[serde(default)]
    pub repos: Vec<RepoMapping>,
}

/// Who the invoices are from.
#[derive(Deserialize, Default, serde::Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Business {
    #[serde(default)]
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub address: Option<String>,
    pub tax_id: Option<String>,
    /// How to pay: bank details, a payment link. Printed on every invoice.
    pub payment_instructions: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvoiceSettings {
    /// Template path, relative to the config dir. Unset uses the built-in one.
    pub template: Option<String>,
    /// `{year}` and `{seq}` / `{seq:0N}` placeholders.
    #[serde(default = "default_number_format")]
    pub number_format: String,
    /// The first sequence number, e.g. to continue an existing numbering.
    #[serde(default = "default_start_seq")]
    pub start_seq: u32,
    #[serde(default = "default_due_days")]
    pub due_days: u32,
    /// Path to a Chromium-family browser binary used to print PDFs.
    pub chrome: Option<String>,
    /// Free text printed on every invoice (e.g. "Thank you!").
    pub notes: Option<String>,
    /// Share one number sequence with Harvest: finalize reads Harvest's
    /// invoice numbers (read-only, `HARVEST_*` credentials) and continues
    /// past the highest. Independent of `[harvest] enabled`. [ADR-0008]
    #[serde(default)]
    pub harvest_numbering: bool,
}

impl Default for InvoiceSettings {
    fn default() -> Self {
        Self {
            template: None,
            number_format: default_number_format(),
            start_seq: default_start_seq(),
            due_days: default_due_days(),
            chrome: None,
            notes: None,
            harvest_numbering: false,
        }
    }
}

fn default_number_format() -> String {
    "{year}-{seq:03}".into()
}
fn default_start_seq() -> u32 {
    1
}
fn default_due_days() -> u32 {
    30
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum SmtpSecurity {
    /// Implicit TLS, usually port 465.
    Tls,
    /// Plain connection upgraded with STARTTLS, usually port 587.
    Starttls,
    /// No encryption. Only allowed for a relay on this machine (a local
    /// postfix, a mail bridge, Mailpit for testing).
    None,
}

/// SMTP settings. The password is `$JIMTIME_SMTP_PASSWORD`. [ADR-0003]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmailSettings {
    pub host: String,
    #[serde(default = "default_smtp_port")]
    pub port: u16,
    #[serde(default = "default_security")]
    pub security: SmtpSecurity,
    pub username: String,
    /// `Name <address>` or a bare address.
    pub from: String,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub bcc: Vec<String>,
    /// MiniJinja template; sees the same context as the invoice template.
    #[serde(default = "default_subject")]
    pub subject: String,
    #[serde(default = "default_body")]
    pub body: String,
}

fn default_smtp_port() -> u16 {
    465
}
fn default_security() -> SmtpSecurity {
    SmtpSecurity::Tls
}
fn default_subject() -> String {
    "Invoice {{ invoice.number }} from {{ business.name }}".into()
}
fn default_body() -> String {
    "Hi,\n\n\
     Please find attached invoice {{ invoice.number }} for {{ invoice.total | money }} {{ invoice.currency }}, \
     due {{ invoice.due_date }}.\n\n\
     Thank you,\n{{ business.name }}\n"
        .into()
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct HarvestSettings {
    /// Off by default. [ADR-0006]
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitSettings {
    /// Pull, commit and push the data repo around every write. [ADR-0009]
    #[serde(default = "yes")]
    pub auto_sync: bool,
}

impl Default for GitSettings {
    fn default() -> Self {
        Self { auto_sync: true }
    }
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CloudSettings {
    pub dropbox: Option<DropboxSettings>,
    pub google_drive: Option<GoogleDriveSettings>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DropboxSettings {
    /// The app key of your Dropbox app (public; PKCE needs no secret).
    pub app_key: String,
    /// Absolute Dropbox folder, e.g. `/Invoices`.
    pub folder: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoogleDriveSettings {
    /// OAuth client id of a Google "Desktop app" client. The matching secret
    /// is `$JIMTIME_GDRIVE_CLIENT_SECRET`.
    pub client_id: String,
    /// Folder path under My Drive, e.g. `Invoices` or `Business/Invoices`.
    pub folder: String,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Client {
    pub name: String,
    pub address: Option<String>,
    #[serde(default = "default_currency")]
    pub currency: String,
    /// Invoice recipients.
    #[serde(default)]
    pub email_to: Vec<String>,
    #[serde(default)]
    pub email_cc: Vec<String>,
    /// Per-client template, relative to the config dir.
    pub template: Option<String>,
    pub harvest_id: Option<u64>,
    #[serde(default)]
    pub projects: BTreeMap<String, Project>,
}

fn default_currency() -> String {
    "USD".into()
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub name: String,
    /// Hourly rate in the client's currency.
    pub rate: Option<f64>,
    /// Per-task hourly rates, overriding `rate`, keyed by task key.
    #[serde(default)]
    pub task_rates: BTreeMap<String, f64>,
    /// Task key used when `add` is not given `--task`.
    pub default_task: String,
    #[serde(default = "yes")]
    pub billable: bool,
    pub harvest_id: Option<u64>,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub name: String,
    pub harvest_id: Option<u64>,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct RepoMapping {
    /// The repo's `git rev-parse --show-toplevel`, compared canonically.
    pub path: String,
    pub client: String,
    pub project: String,
}

/// A repo's mapping resolved against the config.
pub struct Resolved<'a> {
    pub client_key: &'a str,
    pub client: &'a Client,
    pub project_key: &'a str,
    pub project: &'a Project,
}

impl Config {
    /// Load and validate the config, with a pointer to `config migrate` when
    /// only the legacy Harvest mapping exists.
    pub fn load() -> Result<Self> {
        let path = paths::config_file()?;
        if !path.exists() {
            let legacy = paths::legacy_mapping_file()?;
            if legacy.exists() {
                bail!(
                    "no config found at:\n{}\n\n\
                     Found the old Harvest mapping at {}.\n\
                     Convert it with:  jimtime config migrate",
                    path.display(),
                    legacy.display()
                );
            }
            bail!(
                "no config found at:\n{}\n\nCreate one with:  jimtime config init",
                path.display()
            );
        }
        Self::load_from(&path)
    }

    /// Load the config if it exists, `None` if it does not. For callers that
    /// work without one (git sync settings default sensibly).
    pub fn load_optional() -> Result<Option<Self>> {
        let path = paths::config_file()?;
        if !path.exists() {
            return Ok(None);
        }
        Self::load_from(&path).map(Some)
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let cfg: Config = toml::from_str(text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Cross-reference checks the TOML shape cannot express, so a typo fails at
    /// load instead of mid-invoice.
    fn validate(&self) -> Result<()> {
        for (ck, c) in &self.clients {
            for (pk, p) in &c.projects {
                if !self.tasks.contains_key(&p.default_task) {
                    bail!(
                        "clients.{ck}.projects.{pk}: default_task {:?} is not defined in [tasks]",
                        p.default_task
                    );
                }
                for tk in p.task_rates.keys() {
                    if !self.tasks.contains_key(tk) {
                        bail!("clients.{ck}.projects.{pk}.task_rates: unknown task {tk:?}");
                    }
                }
            }
        }
        for r in &self.repos {
            let client = self.clients.get(&r.client).ok_or_else(|| {
                anyhow!("repos: {:?} maps to unknown client {:?}", r.path, r.client)
            })?;
            if !client.projects.contains_key(&r.project) {
                bail!(
                    "repos: {:?} maps to unknown project {:?} of client {:?}",
                    r.path,
                    r.project,
                    r.client
                );
            }
        }
        Ok(())
    }

    /// The mapping for a repo, comparing canonical paths.
    pub fn for_repo(&self, repo_path: &Path) -> Result<Resolved<'_>> {
        let target = repo::canonical(repo_path.to_path_buf());
        let m = self
            .repos
            .iter()
            .find(|m| repo::canonical(expand_tilde(&m.path)) == target)
            .ok_or_else(|| {
                anyhow!(
                    "no mapping found for repo:\n{}\n\nAdd a [[repos]] entry to:\n{}",
                    target.display(),
                    paths::config_file()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                )
            })?;
        // validate() guarantees both exist.
        let (client_key, client) = self.clients.get_key_value(&m.client).expect("validated");
        let (project_key, project) = client
            .projects
            .get_key_value(&m.project)
            .expect("validated");
        Ok(Resolved {
            client_key,
            client,
            project_key,
            project,
        })
    }

    pub fn client(&self, key: &str) -> Result<&Client> {
        self.clients.get(key).ok_or_else(|| {
            let known: Vec<&str> = self.clients.keys().map(String::as_str).collect();
            anyhow!(
                "unknown client {key:?}; known clients: {}",
                known.join(", ")
            )
        })
    }

    pub fn task(&self, key: &str) -> Result<&Task> {
        self.tasks.get(key).ok_or_else(|| {
            let known: Vec<&str> = self.tasks.keys().map(String::as_str).collect();
            anyhow!("unknown task {key:?}; known tasks: {}", known.join(", "))
        })
    }

    /// The hourly rate for a client/project/task, or an error naming what to set.
    pub fn rate(&self, client: &str, project: &str, task: &str) -> Result<f64> {
        let p = self
            .client(client)?
            .projects
            .get(project)
            .ok_or_else(|| anyhow!("unknown project {project:?} of client {client:?}"))?;
        p.task_rates.get(task).copied().or(p.rate).ok_or_else(|| {
            anyhow!(
                "no rate for {client}/{project}/{task}: set `rate` (or task_rates.{task}) \
                 under [clients.{client}.projects.{project}]"
            )
        })
    }

    /// Fail unless the Harvest integration is switched on. [ADR-0006]
    pub fn require_harvest(&self) -> Result<()> {
        if !self.harvest.enabled {
            bail!(
                "the Harvest integration is disabled.\n\
                 Turn it on in {} with:\n\n  [harvest]\n  enabled = true",
                paths::config_file()?.display()
            );
        }
        Ok(())
    }

    /// The template for a client: its own, else the global one, else `None`
    /// for the built-in default. Paths are relative to the config dir.
    pub fn template_for(&self, client: &Client) -> Result<Option<PathBuf>> {
        let rel = client.template.as_ref().or(self.invoice.template.as_ref());
        Ok(match rel {
            Some(r) => Some(paths::config_dir()?.join(expand_tilde(r))),
            None => None,
        })
    }
}

/// `~/x` → `$HOME/x`, so config paths can stay portable across machines.
pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        [business]
        name = "Jim Nist"

        [tasks.programming]
        name = "Programming"
        harvest_id = 26185917

        [tasks.pm]
        name = "Project Management"

        [clients.magic-mind]
        name = "Magic Mind"
        email_to = ["ap@example.com"]

        [clients.magic-mind.projects.automations]
        name = "Automations"
        rate = 150.0
        task_rates = { pm = 100.0 }
        default_task = "programming"

        [[repos]]
        path = "/tmp/mm"
        client = "magic-mind"
        project = "automations"
    "#;

    #[test]
    fn parses_with_defaults() {
        let c = Config::parse(SAMPLE).unwrap();
        assert!(!c.harvest.enabled, "Harvest is off by default");
        assert!(c.git.auto_sync);
        assert_eq!(c.invoice.number_format, "{year}-{seq:03}");
        assert_eq!(c.clients["magic-mind"].currency, "USD");
        assert!(c.clients["magic-mind"].projects["automations"].billable);
    }

    #[test]
    fn task_rate_overrides_project_rate() {
        let c = Config::parse(SAMPLE).unwrap();
        assert_eq!(
            c.rate("magic-mind", "automations", "programming").unwrap(),
            150.0
        );
        assert_eq!(c.rate("magic-mind", "automations", "pm").unwrap(), 100.0);
    }

    #[test]
    fn missing_rate_is_an_error_naming_the_setting() {
        let text = SAMPLE.replace("rate = 150.0\n", "");
        let c = Config::parse(&text).unwrap();
        let err = c
            .rate("magic-mind", "automations", "programming")
            .unwrap_err();
        assert!(err.to_string().contains("set `rate`"), "{err}");
    }

    #[test]
    fn unknown_default_task_fails_validation() {
        let text = SAMPLE.replace("default_task = \"programming\"", "default_task = \"nope\"");
        assert!(Config::parse(&text).is_err());
    }

    #[test]
    fn repo_mapping_to_unknown_project_fails_validation() {
        let text = SAMPLE.replace("project = \"automations\"", "project = \"nope\"");
        assert!(Config::parse(&text).is_err());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let text = format!("{SAMPLE}\n[harvest]\nenabeld = true\n");
        assert!(
            Config::parse(&text).is_err(),
            "typos must not pass silently"
        );
    }

    #[test]
    fn harvest_gate() {
        let c = Config::parse(SAMPLE).unwrap();
        assert!(c.require_harvest().is_err());
        let on = Config::parse(&format!("{SAMPLE}\n[harvest]\nenabled = true\n")).unwrap();
        assert!(on.require_harvest().is_ok());
    }
}
