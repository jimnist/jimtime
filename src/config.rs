//! The config files: `config/jimtime.toml` (the business, clients, projects,
//! tasks, repo mappings, and the invoice/email/git/cloud settings) and, when
//! Harvest is used, `config/harvest.toml` (everything Harvest: whether it is
//! on, numbering, and the Harvest ids of clients, projects and tasks).
//!
//! Clients, projects and tasks are identified by their **keys** (the TOML table
//! names). [ADR-0006] The tasks jimtime knows are the union of both files'.
//! Nothing here is secret: passwords and tokens come from the environment or
//! the keychain. [ADR-0003, ADR-0010]

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
    /// From `harvest.toml`, not this file.
    #[serde(skip)]
    pub harvest: HarvestConfig,
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

/// `config/harvest.toml`: everything Harvest, kept out of jimtime.toml so the
/// main config reads the same whether or not Harvest is used. Absent means
/// Harvest is off and nothing has a Harvest id. [ADR-0006]
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct HarvestConfig {
    /// Push time to Harvest (`harvest ...`, `approve --push`). Off by default.
    #[serde(default)]
    pub enabled: bool,
    /// Continue Harvest's invoice numbers: finalize reads them (read-only)
    /// and never reuses one. Works with `enabled = false`. [ADR-0008]
    #[serde(default)]
    pub numbering: bool,
    /// Harvest ids of jimtime's clients and their projects, by key.
    #[serde(default)]
    pub clients: BTreeMap<String, HarvestClient>,
    /// Harvest's tasks, by jimtime task key. A task here but not in
    /// jimtime.toml is still a jimtime task, named as in Harvest.
    #[serde(default)]
    pub tasks: BTreeMap<String, HarvestTask>,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct HarvestClient {
    pub id: u64,
    #[serde(default)]
    pub projects: BTreeMap<String, HarvestProject>,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct HarvestProject {
    pub id: u64,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct HarvestTask {
    pub id: u64,
    /// Harvest's name for it; required when jimtime.toml does not list the
    /// task itself.
    pub name: Option<String>,
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
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub name: String,
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

    /// Load `path` (a jimtime.toml) and the `harvest.toml` beside it, if any.
    pub fn load_from(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let hpath = path.with_file_name(HARVEST_FILE);
        let htext = match std::fs::read_to_string(&hpath) {
            Ok(t) => Some(t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).with_context(|| format!("reading {}", hpath.display())),
        };
        let main = Self::parse_main(&text).with_context(|| format!("in {}", path.display()))?;
        let harvest = match &htext {
            Some(t) => toml::from_str(t).with_context(|| format!("in {}", hpath.display()))?,
            None => HarvestConfig::default(),
        };
        main.join(harvest)
            .with_context(|| format!("in {} / {}", path.display(), hpath.display()))
    }

    /// Parse a jimtime.toml alone (no Harvest).
    #[cfg(test)]
    pub fn parse(text: &str) -> Result<Self> {
        Self::parse_main(text)?.join(HarvestConfig::default())
    }

    /// Parse a jimtime.toml and a harvest.toml together.
    #[cfg(test)]
    pub fn parse_with_harvest(text: &str, harvest: &str) -> Result<Self> {
        Self::from_texts(text, Some(harvest))
    }

    /// Parse and validate the two files' contents as they would load.
    pub fn from_texts(main: &str, harvest: Option<&str>) -> Result<Self> {
        let h = match harvest {
            Some(t) => toml::from_str(t).context(HARVEST_FILE)?,
            None => HarvestConfig::default(),
        };
        Self::parse_main(main)?.join(h)
    }

    /// Parse and validate a jimtime.toml, before Harvest is joined in.
    pub fn parse_main(text: &str) -> Result<Self> {
        let table: toml::Table = toml::from_str(text)?;
        if let Some(key) = legacy_harvest_key(&table) {
            bail!(
                "{key} is a Harvest setting, and those now live in config/{HARVEST_FILE}.\n\
                 Move them there with:  jimtime config migrate"
            );
        }
        Ok(toml::from_str(text)?)
    }

    /// Add harvest.toml: its tasks join jimtime's (the union), and its client
    /// and project keys must be ones jimtime.toml defines. Then validate.
    fn join(mut self, harvest: HarvestConfig) -> Result<Self> {
        for (ck, hc) in &harvest.clients {
            let client = self.clients.get(ck).ok_or_else(|| {
                anyhow!("{HARVEST_FILE}: clients.{ck} is not a client in jimtime.toml")
            })?;
            for pk in hc.projects.keys() {
                if !client.projects.contains_key(pk) {
                    bail!(
                        "{HARVEST_FILE}: clients.{ck}.projects.{pk} is not a project of {ck} in jimtime.toml"
                    );
                }
            }
        }
        for (tk, ht) in &harvest.tasks {
            if !self.tasks.contains_key(tk) {
                let name = ht.name.clone().ok_or_else(|| {
                    anyhow!(
                        "{HARVEST_FILE}: tasks.{tk} is not in jimtime.toml, so it needs a `name`"
                    )
                })?;
                self.tasks.insert(tk.clone(), Task { name });
            }
        }
        self.harvest = harvest;
        self.validate()?;
        Ok(self)
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
                 Turn it on in {} with:\n\n  enabled = true",
                paths::harvest_config_file()?.display()
            );
        }
        Ok(())
    }

    pub fn harvest_client_id(&self, client: &str) -> Option<u64> {
        self.harvest.clients.get(client).map(|c| c.id)
    }

    pub fn harvest_project_id(&self, client: &str, project: &str) -> Option<u64> {
        self.harvest
            .clients
            .get(client)
            .and_then(|c| c.projects.get(project))
            .map(|p| p.id)
    }

    pub fn harvest_task_id(&self, task: &str) -> Option<u64> {
        self.harvest.tasks.get(task).map(|t| t.id)
    }

    /// The client key whose Harvest id is `id`.
    pub fn client_for_harvest(&self, id: u64) -> Option<&str> {
        self.harvest
            .clients
            .iter()
            .find(|(_, c)| c.id == id)
            .map(|(k, _)| k.as_str())
    }

    /// The key of `client`'s project whose Harvest id is `id`.
    pub fn project_for_harvest(&self, client: &str, id: u64) -> Option<&str> {
        self.harvest
            .clients
            .get(client)?
            .projects
            .iter()
            .find(|(_, p)| p.id == id)
            .map(|(k, _)| k.as_str())
    }

    /// The task key whose Harvest id is `id`.
    pub fn task_for_harvest(&self, id: u64) -> Option<&str> {
        self.harvest
            .tasks
            .iter()
            .find(|(_, t)| t.id == id)
            .map(|(k, _)| k.as_str())
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

/// The Harvest config's file name, beside jimtime.toml.
pub const HARVEST_FILE: &str = "harvest.toml";

/// The first Harvest setting found in a jimtime.toml, from before they moved
/// to harvest.toml: `[harvest]`, `invoice.harvest_numbering`, or a
/// `harvest_id` on a client, project or task.
pub fn legacy_harvest_key(t: &toml::Table) -> Option<String> {
    if t.contains_key("harvest") {
        return Some("[harvest]".into());
    }
    let sub = |v: Option<&toml::Value>| v.and_then(toml::Value::as_table).cloned();
    if sub(t.get("invoice")).is_some_and(|i| i.contains_key("harvest_numbering")) {
        return Some("invoice.harvest_numbering".into());
    }
    for (tk, task) in sub(t.get("tasks")).unwrap_or_default() {
        if task
            .as_table()
            .is_some_and(|x| x.contains_key("harvest_id"))
        {
            return Some(format!("tasks.{tk}.harvest_id"));
        }
    }
    for (ck, client) in sub(t.get("clients")).unwrap_or_default() {
        let Some(client) = client.as_table() else {
            continue;
        };
        if client.contains_key("harvest_id") {
            return Some(format!("clients.{ck}.harvest_id"));
        }
        for (pk, p) in sub(client.get("projects")).unwrap_or_default() {
            if p.as_table().is_some_and(|x| x.contains_key("harvest_id")) {
                return Some(format!("clients.{ck}.projects.{pk}.harvest_id"));
            }
        }
    }
    None
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
        let text = format!("{SAMPLE}\n[git]\nauto_snyc = true\n");
        assert!(
            Config::parse(&text).is_err(),
            "typos must not pass silently"
        );
        assert!(
            Config::parse_with_harvest(SAMPLE, "enabeld = true").is_err(),
            "in harvest.toml too"
        );
    }

    const HARVEST: &str = r#"
        enabled = true
        numbering = true

        [tasks.programming]
        id = 26185917

        [tasks.design]
        id = 26185916
        name = "Design"

        [clients.magic-mind]
        id = 17474327

        [clients.magic-mind.projects.automations]
        id = 47491699
    "#;

    #[test]
    fn harvest_gate() {
        let c = Config::parse(SAMPLE).unwrap();
        assert!(c.require_harvest().is_err(), "no harvest.toml: off");
        let on = Config::parse_with_harvest(SAMPLE, HARVEST).unwrap();
        assert!(on.require_harvest().is_ok());
        assert!(on.harvest.numbering);
    }

    #[test]
    fn harvest_ids_come_from_harvest_toml_both_ways() {
        let c = Config::parse_with_harvest(SAMPLE, HARVEST).unwrap();
        assert_eq!(c.harvest_client_id("magic-mind"), Some(17474327));
        assert_eq!(
            c.harvest_project_id("magic-mind", "automations"),
            Some(47491699)
        );
        assert_eq!(c.harvest_task_id("programming"), Some(26185917));
        assert_eq!(c.harvest_task_id("pm"), None, "a jimtime-only task");
        assert_eq!(c.client_for_harvest(17474327), Some("magic-mind"));
        assert_eq!(
            c.project_for_harvest("magic-mind", 47491699),
            Some("automations")
        );
        assert_eq!(c.task_for_harvest(26185916), Some("design"));
    }

    #[test]
    fn tasks_are_the_union_of_both_files() {
        let c = Config::parse_with_harvest(SAMPLE, HARVEST).unwrap();
        // jimtime.toml's name wins for a task in both files.
        assert_eq!(c.task("programming").unwrap().name, "Programming");
        assert_eq!(
            c.task("pm").unwrap().name,
            "Project Management",
            "jimtime only"
        );
        assert_eq!(
            c.task("design").unwrap().name,
            "Design",
            "harvest.toml only"
        );
    }

    #[test]
    fn a_harvest_only_task_needs_a_name() {
        let err = Config::parse_with_harvest(SAMPLE, "[tasks.qa]\nid = 5\n")
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("tasks.qa") && err.contains("name"), "{err}");
    }

    #[test]
    fn harvest_toml_cannot_name_clients_or_projects_jimtime_lacks() {
        assert!(Config::parse_with_harvest(SAMPLE, "[clients.nope]\nid = 1\n").is_err());
        let bad = "[clients.magic-mind]\nid = 1\n[clients.magic-mind.projects.nope]\nid = 2\n";
        assert!(Config::parse_with_harvest(SAMPLE, bad).is_err());
    }

    #[test]
    fn old_harvest_keys_in_jimtime_toml_point_at_migrate() {
        for (old, key) in [
            ("[harvest]\nenabled = true\n", "[harvest]"),
            (
                "[invoice]\nharvest_numbering = true\n",
                "invoice.harvest_numbering",
            ),
        ] {
            let err = Config::parse(&format!("{SAMPLE}\n{old}"))
                .err()
                .unwrap()
                .to_string();
            assert!(err.contains(key) && err.contains("config migrate"), "{err}");
        }
        let text = SAMPLE.replace(
            "name = \"Automations\"",
            "name = \"Automations\"\nharvest_id = 9",
        );
        let err = Config::parse(&text).err().unwrap().to_string();
        assert!(
            err.contains("clients.magic-mind.projects.automations.harvest_id"),
            "{err}"
        );
    }
}
