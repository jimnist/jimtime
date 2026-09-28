use super::Command;
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::config::Config as Settings;
use crate::datarepo::{self, Sync};
use crate::paths;
use crate::slug::slugify;
use crate::store::{Day, day_files, write_atomic};

/// Create, migrate and check the config file (config/jimtime.toml)
#[derive(Args)]
pub struct Config {
    #[command(subcommand)]
    cmd: ConfigCmd,
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Write a commented starter config
    Init,
    /// Convert the old harvest-projects.json into jimtime.toml and rewrite the
    /// day files to use client/project/task keys
    Migrate {
        /// Replace an existing jimtime.toml
        #[arg(long)]
        force: bool,
    },
    /// Validate the config and summarize it
    Check,
    /// Print the config file path
    Path,
}

#[async_trait::async_trait]
impl Command for Config {
    async fn run(&self) -> Result<()> {
        match &self.cmd {
            ConfigCmd::Init => init(),
            ConfigCmd::Migrate { force } => migrate(*force),
            ConfigCmd::Check => check(),
            ConfigCmd::Path => {
                println!("{}", paths::config_file()?.display());
                Ok(())
            }
        }
    }
}

fn init() -> Result<()> {
    let path = paths::config_file()?;
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    let sync = Sync::begin("config init", false)?;
    write_config(&path, STARTER)?;
    sync.commit("config: starter jimtime.toml")?;
    println!("Wrote {}", path.display());
    println!("Fill in [business], a client and a [[repos]] mapping, then run `jimtime config check`.");
    Ok(())
}

fn check() -> Result<()> {
    let c = Settings::load()?;
    println!("{} is valid.\n", paths::config_file()?.display());
    println!(
        "Business:  {}",
        if c.business.name.is_empty() {
            "(name not set)"
        } else {
            &c.business.name
        }
    );
    for (ck, client) in &c.clients {
        println!(
            "Client:    {} ({ck}), {}, invoices to: {}",
            client.name,
            client.currency,
            if client.email_to.is_empty() {
                "(none)".to_string()
            } else {
                client.email_to.join(", ")
            }
        );
        if !client.email_cc.is_empty() {
            println!("           cc: {}", client.email_cc.join(", "));
        }
        for (pk, p) in &client.projects {
            let rate = p
                .rate
                .map(|r| format!("{r}/h"))
                .unwrap_or_else(|| "no rate".into());
            println!("  Project: {} ({pk}), {rate}, default task {}", p.name, p.default_task);
        }
    }
    println!("Tasks:     {}", c.tasks.keys().cloned().collect::<Vec<_>>().join(", "));
    println!("Repos:     {}", c.repos.len());
    println!(
        "Harvest:   {}",
        if c.harvest.enabled { "enabled" } else { "disabled" }
    );
    println!(
        "Email:     {}",
        c.email
            .as_ref()
            .map(|e| format!("{}:{} as {}", e.host, e.port, e.username))
            .unwrap_or_else(|| "not configured".into())
    );
    if let Some(e) = &c.email {
        if !e.cc.is_empty() {
            println!("           cc on every invoice: {}", e.cc.join(", "));
        }
        if !e.bcc.is_empty() {
            println!("           bcc on every invoice: {}", e.bcc.join(", "));
        }
    }
    let mut cloud = Vec::new();
    if c.cloud.dropbox.is_some() {
        cloud.push("dropbox");
    }
    if c.cloud.google_drive.is_some() {
        cloud.push("google-drive");
    }
    println!(
        "Cloud:     {}",
        if cloud.is_empty() {
            "none".to_string()
        } else {
            cloud.join(", ")
        }
    );
    Ok(())
}

fn write_config(path: &Path, text: &str) -> Result<()> {
    // Parse before writing so a generated file is never invalid.
    Settings::parse(text).context("generated config did not validate (this is a bug)")?;
    write_atomic(path, text.as_bytes())?;
    datarepo::note_write(path);
    Ok(())
}

// --- migrate ---------------------------------------------------------------

#[derive(Deserialize)]
struct Legacy {
    #[serde(default)]
    repos: Vec<LegacyRepo>,
    #[serde(default)]
    aliases: BTreeMap<String, LegacyTask>,
}

#[derive(Deserialize)]
struct LegacyRepo {
    repo_path: String,
    client_id: u64,
    client_name: String,
    project_id: u64,
    project_name: String,
    default_task_id: u64,
    default_task_name: String,
    billable: bool,
}

#[derive(Deserialize)]
struct LegacyTask {
    task_id: u64,
    task_name: String,
}

/// Keys derived from the legacy mapping, by Harvest id.
struct Keys {
    clients: HashMap<u64, String>,
    projects: HashMap<u64, String>,
    tasks: HashMap<u64, String>,
}

fn migrate(force: bool) -> Result<()> {
    let legacy_path = paths::legacy_mapping_file()?;
    let path = paths::config_file()?;
    if !legacy_path.exists() {
        bail!("nothing to migrate: {} does not exist", legacy_path.display());
    }
    if path.exists() && !force {
        bail!(
            "{} already exists; pass --force to replace it",
            path.display()
        );
    }
    let legacy: Legacy = serde_json::from_str(
        &std::fs::read_to_string(&legacy_path)
            .with_context(|| format!("reading {}", legacy_path.display()))?,
    )
    .with_context(|| format!("parsing {}", legacy_path.display()))?;

    let (text, keys) = render_migrated(&legacy);

    let sync = Sync::begin("config migrate", false)?;
    write_config(&path, &text)?;
    let rewritten = rewrite_days(&keys)?;
    std::fs::remove_file(&legacy_path)
        .with_context(|| format!("removing {}", legacy_path.display()))?;
    datarepo::note_write(&legacy_path);
    sync.commit("config: migrate harvest-projects.json to jimtime.toml")?;

    println!("Wrote {}", path.display());
    println!("Rewrote {rewritten} day file(s) with client/project/task keys.");
    println!("Removed {} (its content is now in jimtime.toml).", legacy_path.display());
    println!(
        "\nHarvest is now disabled by default. To keep pushing, set `enabled = true` under [harvest].\n\
         To invoice, fill in [business] and each project's `rate`, then `jimtime config check`."
    );
    Ok(())
}

fn q(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// Build the new config text from the legacy mapping. Task keys are the alias
/// names the user already types (`--task pm`); tasks with no alias get a slug.
fn render_migrated(legacy: &Legacy) -> (String, Keys) {
    let mut keys = Keys {
        clients: HashMap::new(),
        projects: HashMap::new(),
        tasks: HashMap::new(),
    };
    let mut tasks: BTreeMap<String, (String, u64)> = BTreeMap::new();
    for (alias, t) in &legacy.aliases {
        keys.tasks.entry(t.task_id).or_insert_with(|| alias.clone());
        tasks.insert(alias.clone(), (t.task_name.clone(), t.task_id));
    }
    for r in &legacy.repos {
        if let std::collections::hash_map::Entry::Vacant(slot) = keys.tasks.entry(r.default_task_id)
        {
            let k = slugify(&r.default_task_name);
            slot.insert(k.clone());
            tasks.insert(k, (r.default_task_name.clone(), r.default_task_id));
        }
        keys.clients
            .entry(r.client_id)
            .or_insert_with(|| slugify(&r.client_name));
        keys.projects
            .entry(r.project_id)
            .or_insert_with(|| slugify(&r.project_name));
    }

    let mut out = String::from(MIGRATED_HEADER);
    out.push_str("\n# --- Tasks ---\n");
    for (k, (name, id)) in &tasks {
        out.push_str(&format!(
            "\n[tasks.{k}]\nname = {}\nharvest_id = {id}\n",
            q(name)
        ));
    }

    out.push_str("\n# --- Clients and projects ---\n");
    let mut seen_clients = Vec::new();
    for r in &legacy.repos {
        let ck = &keys.clients[&r.client_id];
        if !seen_clients.contains(ck) {
            seen_clients.push(ck.clone());
            out.push_str(&format!(
                "\n[clients.{ck}]\nname = {}\ncurrency = \"USD\"\n\
                 # address = \"\"\"\n# 1 Client Way\n# City, ST 00000\n# \"\"\"\n\
                 # email_to = [\"billing@example.com\"]\n\
                 # email_cc = [\"controller@example.com\"]\n\
                 harvest_id = {}\n",
                q(&r.client_name),
                r.client_id
            ));
        }
        let pk = &keys.projects[&r.project_id];
        let header = format!("[clients.{ck}.projects.{pk}]");
        if !out.contains(&header) {
            out.push_str(&format!(
                "\n{header}\nname = {}\n# rate = 150.0\n# task_rates = {{ pm = 100.0 }}\n\
                 default_task = {}\nbillable = {}\nharvest_id = {}\n",
                q(&r.project_name),
                q(&keys.tasks[&r.default_task_id]),
                r.billable,
                r.project_id
            ));
        }
    }

    out.push_str("\n# --- Repo mappings ---\n");
    for r in &legacy.repos {
        out.push_str(&format!(
            "\n[[repos]]\npath = {}\nclient = {}\nproject = {}\n",
            q(&r.repo_path),
            q(&keys.clients[&r.client_id]),
            q(&keys.projects[&r.project_id])
        ));
    }
    (out, keys)
}

/// Rewrite every day file with keys resolved by Harvest id, so sections match
/// the new config exactly (a slug of "Project Management" is not `pm`).
fn rewrite_days(keys: &Keys) -> Result<usize> {
    let mut count = 0;
    for path in day_files(&paths::entries_dir()?)? {
        let mut day = Day::load_path(&path)?;
        for s in &mut day.sections {
            if let Some(k) = s.harvest_client_id.and_then(|id| keys.clients.get(&id)) {
                s.client = k.clone();
            }
            if let Some(k) = s.harvest_project_id.and_then(|id| keys.projects.get(&id)) {
                s.project = k.clone();
            }
            if let Some(k) = s.harvest_task_id.and_then(|id| keys.tasks.get(&id)) {
                s.task = k.clone();
            }
        }
        day.save()?;
        count += 1;
    }
    Ok(count)
}

const MIGRATED_HEADER: &str = r#"# jimtime config. Non-secret: passwords and tokens come from the
# environment or the OS keychain, never this file. See the README.

[business]
name = ""
# email = "you@example.com"
# address = """
# 123 Main St
# City, ST 00000
# """
# payment_instructions = "Pay by ACH to ..."

[invoice]
# number_format = "{year}-{seq:03}"
# start_seq = 1
# due_days = 30
# template = "templates/invoice.html"   # relative to this config dir
# To carry on Harvest's invoice numbers (e.g. 036 -> 037), match its format
# and let finalize read the numbers Harvest has used:
# number_format = "{seq:03}"
# harvest_numbering = true

# [email]
# host = "smtp.fastmail.com"
# port = 465
# security = "tls"                      # or "starttls" (port 587)
# username = "you@example.com"
# from = "Your Name <you@example.com>"
# cc = ["books@example.com"]            # Cc on every invoice email
# bcc = ["you@example.com"]
# The password is $JIMTIME_SMTP_PASSWORD.

[harvest]
# Off by default. Turn on to use `jimtime harvest ...` and `approve --push`.
enabled = false

# [cloud.dropbox]
# app_key = "..."
# folder = "/Invoices"

# [cloud.google_drive]
# client_id = "....apps.googleusercontent.com"
# folder = "Invoices"
"#;

const STARTER: &str = r#"# jimtime config. Non-secret: passwords and tokens come from the
# environment or the OS keychain, never this file. See the README.

[business]
name = "Your Name"
# email = "you@example.com"
# address = """
# 123 Main St
# City, ST 00000
# """
# payment_instructions = "Pay by ACH to ..."

[invoice]
# number_format = "{year}-{seq:03}"
# start_seq = 1
# due_days = 30
# template = "templates/invoice.html"   # relative to this config dir

# [email]
# host = "smtp.fastmail.com"
# port = 465
# security = "tls"                      # or "starttls" (port 587)
# username = "you@example.com"
# from = "Your Name <you@example.com>"
# cc = ["books@example.com"]            # Cc on every invoice email
# bcc = ["you@example.com"]
# The password is $JIMTIME_SMTP_PASSWORD.

[harvest]
enabled = false

[tasks.development]
name = "Development"

[tasks.meetings]
name = "Meetings"

[clients.acme]
name = "Acme Corp"
currency = "USD"
email_to = ["billing@acme.example"]
# email_cc = ["controller@acme.example"]
# address = """
# 1 Acme Way
# """

[clients.acme.projects.website]
name = "Website"
rate = 150.0
# task_rates = { meetings = 100.0 }
default_task = "development"
billable = true

[[repos]]
path = "~/code/acme/website"
client = "acme"
project = "website"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_config_is_valid() {
        Settings::parse(STARTER).unwrap();
    }

    #[test]
    fn migration_keeps_alias_keys_and_harvest_ids() {
        let legacy: Legacy = serde_json::from_str(
            r#"{
              "repos": [{
                "repo_path": "/code/mm", "client_id": 1, "client_name": "Magic Mind",
                "project_id": 2, "project_name": "Automations",
                "default_task_id": 3, "default_task_name": "Programming", "billable": true
              }],
              "aliases": {
                "programming": { "task_id": 3, "task_name": "Programming" },
                "pm": { "task_id": 4, "task_name": "Project Management" }
              }
            }"#,
        )
        .unwrap();
        let (text, keys) = render_migrated(&legacy);
        let c = Settings::parse(&text).unwrap();
        assert!(!c.harvest.enabled);
        assert_eq!(c.tasks["pm"].harvest_id, Some(4));
        let p = &c.clients["magic-mind"].projects["automations"];
        assert_eq!(p.default_task, "programming");
        assert_eq!(p.harvest_id, Some(2));
        assert_eq!(c.repos[0].client, "magic-mind");
        assert_eq!(keys.tasks[&4], "pm", "alias key, not slug of the name");
    }
}
