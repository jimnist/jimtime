use super::Command;
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};


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
    write_config(STARTER, None)?;
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
        "Data sync: {}",
        if c.git.auto_sync {
            "on when the data home is its own repo"
        } else {
            "off ([git] auto_sync = false)"
        }
    );
    let h = &c.harvest;
    println!(
        "Harvest:   {}{}; ids for {} client(s), {} task(s)",
        if h.enabled { "pushing on" } else { "pushing off" },
        if h.numbering { ", numbering continues Harvest's" } else { "" },
        h.clients.len(),
        h.tasks.len()
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

/// Write jimtime.toml (and harvest.toml, when given), after checking the pair
/// loads, so a generated config is never invalid.
fn write_config(main: &str, harvest: Option<&str>) -> Result<()> {
    Settings::from_texts(main, harvest).context("the new config did not validate")?;
    let path = paths::config_file()?;
    write_atomic(&path, main.as_bytes())?;
    datarepo::note_write(&path);
    if let Some(h) = harvest {
        let hpath = paths::harvest_config_file()?;
        write_atomic(&hpath, h.as_bytes())?;
        datarepo::note_write(&hpath);
    }
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

/// Bring an older config up to date: the Harvest-only `harvest-projects.json`
/// becomes jimtime.toml + harvest.toml, and a jimtime.toml that still holds
/// Harvest settings has them moved out into harvest.toml. [ADR-0006]
fn migrate(force: bool) -> Result<()> {
    let legacy_path = paths::legacy_mapping_file()?;
    let path = paths::config_file()?;
    let hpath = paths::harvest_config_file()?;

    if path.exists() {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let table: toml::Table =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        if crate::config::legacy_harvest_key(&table).is_some() {
            return split_existing(&text, force);
        }
        if !(legacy_path.exists() && force) {
            bail!(
                "nothing to migrate: {} is current{}",
                path.display(),
                if legacy_path.exists() {
                    " (pass --force to rebuild it from harvest-projects.json)"
                } else {
                    ""
                }
            );
        }
    }
    if !legacy_path.exists() {
        bail!("nothing to migrate: {} does not exist", legacy_path.display());
    }
    if hpath.exists() && !force {
        bail!("{} already exists; pass --force to replace it", hpath.display());
    }
    let legacy: Legacy = serde_json::from_str(
        &std::fs::read_to_string(&legacy_path)
            .with_context(|| format!("reading {}", legacy_path.display()))?,
    )
    .with_context(|| format!("parsing {}", legacy_path.display()))?;

    let (main, harvest, keys) = render_migrated(&legacy);

    let sync = Sync::begin("config migrate", false)?;
    write_config(&main, Some(&render_harvest(&harvest)))?;
    let rewritten = rewrite_days(&keys)?;
    std::fs::remove_file(&legacy_path)
        .with_context(|| format!("removing {}", legacy_path.display()))?;
    datarepo::note_write(&legacy_path);
    sync.commit("config: migrate harvest-projects.json to jimtime.toml + harvest.toml")?;

    println!("Wrote {} and {}", path.display(), hpath.display());
    println!("Rewrote {rewritten} day file(s) with client/project/task keys.");
    println!("Removed {} (its content is now in those two).", legacy_path.display());
    println!(
        "\nHarvest pushing is off by default. To keep pushing, set `enabled = true` in harvest.toml.\n\
         To invoice, fill in [business] and each project's `rate`, then `jimtime config check`."
    );
    Ok(())
}

/// Move the Harvest settings out of an existing jimtime.toml, editing it in
/// place so its comments and everything else stay exactly as they were.
fn split_existing(text: &str, force: bool) -> Result<()> {
    let path = paths::config_file()?;
    let hpath = paths::harvest_config_file()?;
    if hpath.exists() && !force {
        bail!(
            "{} still has Harvest settings, but {} already exists; merge them by hand, \
             or pass --force to replace it",
            path.display(),
            hpath.display()
        );
    }
    let (main, harvest) = split_harvest(text)?;
    let sync = Sync::begin("config migrate", false)?;
    write_config(&main, Some(&render_harvest(&harvest)))?;
    sync.commit("config: move Harvest settings to harvest.toml")?;
    println!("Moved the Harvest settings out of {} into {}.", path.display(), hpath.display());
    println!(
        "  pushing {}, numbering {}, ids for {} client(s) and {} task(s)",
        if harvest.enabled { "on" } else { "off" },
        if harvest.numbering { "on" } else { "off" },
        harvest.clients.len(),
        harvest.tasks.len()
    );
    Ok(())
}

/// `(client key, Harvest id, [(project key, Harvest id)])`
type HarvestClientOut = (String, u64, Vec<(String, u64)>);

/// Everything harvest.toml holds, for writing it.
#[derive(Default, Debug, PartialEq)]
struct HarvestOut {
    enabled: bool,
    numbering: bool,
    /// `(key, id, name)`
    tasks: Vec<(String, u64, String)>,
    /// Clients with their Harvest ids and their projects'.
    clients: Vec<HarvestClientOut>,
}

/// The comment over jimtime.toml's tasks.
const TASKS_COMMENT: &str = "\
# --- Tasks ---
# Tasks are based on Harvest's tasks; their Harvest ids are in harvest.toml.
# jimtime knows the union of the tasks here and in harvest.toml, so either
# file can list a task the other does not.
";

/// Take the Harvest keys out of a jimtime.toml's text, returning the edited
/// text and what they said. Comments and layout are kept (`toml_edit`).
fn split_harvest(text: &str) -> Result<(String, HarvestOut)> {
    use toml_edit::{DocumentMut, Item};
    let mut doc: DocumentMut = text.parse().context("parsing jimtime.toml")?;
    let mut out = HarvestOut::default();
    let int = |i: Option<Item>| -> Option<u64> { i.and_then(|v| v.as_integer()).map(|n| n as u64) };

    // Comments above `[harvest]` belong to its header in TOML, but they are
    // usually about whatever came before it (a commented-out [email] block).
    // So empty the table and drop just its header line below, rather than
    // removing the table and its comments with it.
    let mut harvest_header = false;
    match doc.get_mut("harvest") {
        Some(Item::Table(h)) => {
            out.enabled = h.get("enabled").and_then(Item::as_bool).unwrap_or(false);
            h.clear();
            harvest_header = true;
        }
        Some(_) => {
            let h = doc.remove("harvest").expect("present");
            out.enabled = h.get("enabled").and_then(Item::as_bool).unwrap_or(false);
        }
        None => {}
    }
    if let Some(inv) = doc.get_mut("invoice").and_then(Item::as_table_like_mut) {
        out.numbering = inv
            .remove("harvest_numbering")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    }

    if let Some(tasks) = doc.get_mut("tasks").and_then(Item::as_table_like_mut) {
        for (key, item) in tasks.iter_mut() {
            let Some(t) = item.as_table_like_mut() else { continue };
            let name = t.get("name").and_then(Item::as_str).unwrap_or_default().to_string();
            if let Some(id) = int(t.remove("harvest_id")) {
                out.tasks.push((key.to_string(), id, name));
            }
        }
    }
    // Explain the union over the first task, replacing a bare section rule.
    if let Some(tasks) = doc.get_mut("tasks").and_then(Item::as_table_mut)
        && let Some((_, first)) = tasks.iter_mut().next()
        && let Some(t) = first.as_table_mut()
    {
        let prefix = t.decor().prefix().and_then(|p| p.as_str()).unwrap_or("").to_string();
        if !prefix.contains("union of the tasks") {
            let kept = prefix.replace("# --- Tasks ---\n", "");
            t.decor_mut().set_prefix(format!("\n{TASKS_COMMENT}{}", kept.trim_start_matches('\n')));
        }
    }

    if let Some(clients) = doc.get_mut("clients").and_then(Item::as_table_like_mut) {
        for (ck, item) in clients.iter_mut() {
            let Some(c) = item.as_table_like_mut() else { continue };
            let id = int(c.remove("harvest_id"));
            let mut projects = Vec::new();
            if let Some(ps) = c.get_mut("projects").and_then(Item::as_table_like_mut) {
                for (pk, p) in ps.iter_mut() {
                    if let Some(pid) = p.as_table_like_mut().and_then(|p| int(p.remove("harvest_id"))) {
                        projects.push((pk.to_string(), pid));
                    }
                }
            }
            match id {
                Some(id) => out.clients.push((ck.to_string(), id, projects)),
                None if !projects.is_empty() => bail!(
                    "clients.{ck} has Harvest project ids but no harvest_id of its own; add it, then re-run"
                ),
                None => {}
            }
        }
    }

    let mut edited = doc.to_string();
    if harvest_header {
        edited = edited
            .lines()
            .filter(|l| {
                let l = l.trim();
                !(l == "[harvest]" || l.starts_with("[harvest]") && l[9..].trim_start().starts_with('#'))
            })
            .map(|l| format!("{l}\n"))
            .collect();
    }
    // The suggestion comments this tool wrote under [invoice] are now stale.
    let edited = edited
        .replace(
            "# To carry on Harvest's invoice numbers (e.g. 036 -> 037), match its format\n\
             # and let finalize read the numbers Harvest has used:\n",
            "",
        )
        .replace("# harvest_numbering = true\n", "");
    Ok((edited, out))
}

/// The text of harvest.toml.
fn render_harvest(h: &HarvestOut) -> String {
    let mut out = format!(
        "# Harvest: what jimtime knows about your Harvest account. Non-secret; the\n\
         # credentials are HARVEST_ACCESS_TOKEN and HARVEST_ACCOUNT_ID in the environment.\n\
         \n\
         # Push time to Harvest (`jimtime harvest ...`, `approve --push`).\n\
         enabled = {}\n\
         \n\
         # Continue Harvest's invoice numbers: invoice finalize reads the numbers\n\
         # Harvest has used (read-only) and never reuses one. Works with enabled = false.\n\
         numbering = {}\n",
        h.enabled, h.numbering
    );
    out.push_str(
        "\n# --- Tasks ---\n\
         # jimtime's tasks are based on these Harvest tasks, by jimtime task key.\n\
         # jimtime knows the union of the tasks here and in jimtime.toml: a task only\n\
         # here is still usable (`add --task <key>`), named as below; one only in\n\
         # jimtime.toml just has no Harvest id, so it cannot be pushed.\n",
    );
    for (k, id, name) in &h.tasks {
        out.push_str(&format!("\n[tasks.{k}]\nid = {id}\nname = {}\n", q(name)));
    }
    out.push_str("\n# --- Clients and projects, by their jimtime keys ---\n");
    for (ck, id, projects) in &h.clients {
        out.push_str(&format!("\n[clients.{ck}]\nid = {id}\n"));
        for (pk, pid) in projects {
            out.push_str(&format!("\n[clients.{ck}.projects.{pk}]\nid = {pid}\n"));
        }
    }
    out
}

fn q(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// Build jimtime.toml's text and harvest.toml's content from the legacy
/// mapping. Task keys are the alias names the user already types (`--task
/// pm`); tasks with no alias get a slug.
fn render_migrated(legacy: &Legacy) -> (String, HarvestOut, Keys) {
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
    let mut harvest = HarvestOut::default();
    out.push('\n');
    out.push_str(TASKS_COMMENT);
    for (k, (name, id)) in &tasks {
        out.push_str(&format!("\n[tasks.{k}]\nname = {}\n", q(name)));
        harvest.tasks.push((k.clone(), *id, name.clone()));
    }

    out.push_str("\n# --- Clients and projects ---\n");
    for r in &legacy.repos {
        let ck = &keys.clients[&r.client_id];
        if !harvest.clients.iter().any(|(k, _, _)| k == ck) {
            harvest.clients.push((ck.clone(), r.client_id, Vec::new()));
            out.push_str(&format!(
                "\n[clients.{ck}]\nname = {}\ncurrency = \"USD\"\n\
                 # address = \"\"\"\n# 1 Client Way\n# City, ST 00000\n# \"\"\"\n\
                 # email_to = [\"billing@example.com\"]\n\
                 # email_cc = [\"controller@example.com\"]\n",
                q(&r.client_name),
            ));
        }
        let pk = &keys.projects[&r.project_id];
        let header = format!("[clients.{ck}.projects.{pk}]");
        if !out.contains(&header) {
            let (_, _, projects) = harvest
                .clients
                .iter_mut()
                .find(|(k, _, _)| k == ck)
                .expect("pushed above");
            projects.push((pk.clone(), r.project_id));
            out.push_str(&format!(
                "\n{header}\nname = {}\n# rate = 150.0\n# task_rates = {{ pm = 100.0 }}\n\
                 default_task = {}\nbillable = {}\n",
                q(&r.project_name),
                q(&keys.tasks[&r.default_task_id]),
                r.billable,
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
    (out, harvest, keys)
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
# here and set numbering = true in harvest.toml:
# number_format = "{seq:03}"

# [email]
# host = "smtp.fastmail.com"
# port = 465
# security = "tls"                      # or "starttls" (port 587)
# username = "you@example.com"
# from = "Your Name <you@example.com>"
# cc = ["books@example.com"]            # Cc on every invoice email
# bcc = ["you@example.com"]
# The password is $JIMTIME_SMTP_PASSWORD.

# Harvest settings (pushing, numbering, Harvest ids) are in harvest.toml.

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

# Using Harvest? Its settings and ids go in harvest.toml beside this file.

# --- Tasks ---
# If you use Harvest, tasks are based on Harvest's tasks; their Harvest ids
# are in harvest.toml. jimtime knows the union of the tasks here and in
# harvest.toml, so either file can list a task the other does not.

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
        Settings::from_texts(STARTER, None).unwrap();
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
        let (text, harvest, keys) = render_migrated(&legacy);
        assert!(!text.contains("harvest_id"), "no Harvest ids in jimtime.toml");
        let c = Settings::from_texts(&text, Some(&render_harvest(&harvest))).unwrap();
        assert!(!c.harvest.enabled);
        assert_eq!(c.harvest_task_id("pm"), Some(4));
        let p = &c.clients["magic-mind"].projects["automations"];
        assert_eq!(p.default_task, "programming");
        assert_eq!(c.harvest_project_id("magic-mind", "automations"), Some(2));
        assert_eq!(c.harvest_client_id("magic-mind"), Some(1));
        assert_eq!(c.repos[0].client, "magic-mind");
        assert_eq!(keys.tasks[&4], "pm", "alias key, not slug of the name");
    }

    /// The shape of a jimtime.toml written before harvest.toml existed, with
    /// hand edits (a real email_to, a comment) that must survive the split.
    const OLD: &str = r#"# jimtime config.

[business]
name = ""

[invoice]
number_format = "{seq:03}"
harvest_numbering = true
# template = "templates/invoice.html"   # relative to this config dir
# To carry on Harvest's invoice numbers (e.g. 036 -> 037), match its format
# and let finalize read the numbers Harvest has used:
# harvest_numbering = true

# [email]
# host = "smtp.fastmail.com"
# The password is $JIMTIME_SMTP_PASSWORD.

[harvest]
# Off by default. Turn on to use `jimtime harvest ...` and `approve --push`.
enabled = false

# --- Tasks ---

[tasks.pm]
name = "Project Management"
harvest_id = 26185919

[tasks.programming]
name = "Programming"
harvest_id = 26185917

# --- Clients and projects ---

[clients.magic-mind]
name = "Magic Mind"
currency = "USD"
email_to = ["payables@example.com"]   # hand edit
harvest_id = 17474327

[clients.magic-mind.projects.automations]
name = "Automations"
default_task = "programming"
billable = true
harvest_id = 47491699
"#;

    #[test]
    fn splitting_moves_every_harvest_key_and_keeps_the_rest() {
        let (main, h) = split_harvest(OLD).unwrap();
        assert_eq!(
            h,
            HarvestOut {
                enabled: false,
                numbering: true,
                tasks: vec![
                    ("pm".into(), 26185919, "Project Management".into()),
                    ("programming".into(), 26185917, "Programming".into()),
                ],
                clients: vec![(
                    "magic-mind".into(),
                    17474327,
                    vec![("automations".into(), 47491699)]
                )],
            }
        );
        let table: toml::Table = toml::from_str(&main).unwrap();
        assert_eq!(crate::config::legacy_harvest_key(&table), None, "{main}");
        assert!(!main.contains("harvest_numbering"), "{main}");
        assert!(main.contains("email_to = [\"payables@example.com\"]   # hand edit"));
        assert!(
            main.contains("# [email]\n# host = \"smtp.fastmail.com\"\n# The password is"),
            "comments above [harvest] are not Harvest's; they stay:\n{main}"
        );
        assert!(!main.contains("[harvest]") && !main.contains("Off by default"), "{main}");
        assert!(main.contains("number_format = \"{seq:03}\""));
        assert!(main.contains("union of the tasks here and in harvest.toml"), "{main}");
        assert_eq!(main.matches("# --- Tasks ---").count(), 1, "{main}");

        let c = Settings::from_texts(&main, Some(&render_harvest(&h))).unwrap();
        assert!(c.harvest.numbering && !c.harvest.enabled);
        assert_eq!(c.harvest_task_id("programming"), Some(26185917));
        assert_eq!(c.harvest_project_id("magic-mind", "automations"), Some(47491699));

        // Splitting again finds nothing left to move.
        let (again, h2) = split_harvest(&main).unwrap();
        assert_eq!(again, main);
        assert_eq!(h2, HarvestOut::default());
    }
}
