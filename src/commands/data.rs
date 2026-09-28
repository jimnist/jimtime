use super::Command;
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};

use crate::datarepo::{self, RepoState, Sync, git, git_ok};
use crate::merge::merge_days;
use crate::paths;
use crate::store::{Day, write_atomic};

/// Manage the data repo: $JIMTIME_HOME as its own git repo, auto-synced
#[derive(Args)]
pub struct Data {
    #[command(subcommand)]
    cmd: DataCmd,
}

#[derive(Subcommand)]
enum DataCmd {
    /// Make $JIMTIME_HOME a git repo (or clone one into it) and set up sync
    Init {
        /// The private remote to push to, e.g. git@github.com:you/jimtime-data.git
        #[arg(long)]
        remote: Option<String>,
    },
    /// Show whether sync is active and how the data repo compares to its remote
    Status,
    /// Pull and push now (every write already does this)
    Sync,
    /// Git merge driver for day files (called by git, not by hand)
    #[command(hide = true)]
    MergeDay {
        base: PathBuf,
        ours: PathBuf,
        theirs: PathBuf,
        /// The file's path in the repo, for messages
        path: Option<String>,
    },
}

#[async_trait::async_trait]
impl Command for Data {
    async fn run(&self) -> Result<()> {
        match &self.cmd {
            DataCmd::Init { remote } => init(remote.as_deref()),
            DataCmd::Status => status(),
            DataCmd::Sync => sync_now(),
            DataCmd::MergeDay {
                base,
                ours,
                theirs,
                path,
            } => merge_day(base, ours, theirs, path.as_deref()),
        }
    }
}

/// One line for `jimtime status`.
pub fn describe_state() -> Result<String> {
    Ok(match datarepo::state()? {
        RepoState::Active { root } => {
            if datarepo::has_upstream(&root) {
                "auto-sync on (commits and pushes every change)".into()
            } else {
                "auto-sync on, commit only (no remote; see `jimtime data init --remote`)".into()
            }
        }
        RepoState::Disabled { .. } => "a git repo, auto-sync off ([git] auto_sync = false)".into(),
        RepoState::Nested { toplevel } => format!(
            "inside another repo ({}); not synced - see `jimtime data status`",
            toplevel.display()
        ),
        RepoState::NotARepo => "not a git repo; not synced - see `jimtime data init`".into(),
    })
}

fn init(remote: Option<&str>) -> Result<()> {
    let home = paths::home()?;
    match datarepo::state()? {
        RepoState::Nested { toplevel } => bail!(
            "{} is inside another git repo ({}).\n\
             jimtime will not commit into a repo it does not own. Move the data into its own \
             repo first; to keep its history:\n\n  {}",
            home.display(),
            toplevel.display(),
            subtree_hint(&toplevel, &home)
        ),
        RepoState::Active { root } | RepoState::Disabled { root } => {
            if let Some(url) = remote {
                add_remote_and_push(&root, url)?;
            }
            let sync = Sync::begin("data init", false)?;
            datarepo::ensure_setup(&root)?;
            sync.commit("data: set up jimtime sync")?;
            println!("{} is already a git repo; sync is set up.", root.display());
        }
        RepoState::NotARepo => {
            let empty = !home.exists()
                || std::fs::read_dir(&home)
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(true);
            if let (true, Some(url)) = (empty, remote) {
                let parent = home.parent().context("JIMTIME_HOME has no parent")?;
                std::fs::create_dir_all(parent)?;
                git_ok(
                    parent,
                    &["clone", "--quiet", url, &home.display().to_string()],
                )?;
                let root = std::fs::canonicalize(&home)?;
                let sync = Sync::begin("data init", false)?;
                datarepo::ensure_setup(&root)?;
                sync.commit("data: set up jimtime sync")?;
                println!("Cloned {url} into {}.", home.display());
            } else {
                std::fs::create_dir_all(&home)?;
                let root = std::fs::canonicalize(&home)?;
                git_ok(&root, &["init", "--quiet", "-b", "main"])?;
                datarepo::ensure_setup(&root)?;
                git_ok(&root, &["add", "-A"])?;
                git_ok(&root, &["commit", "--quiet", "-m", "data: initial jimtime data"])?;
                if let Some(url) = remote {
                    add_remote_and_push(&root, url)?;
                }
                println!("Made {} a git repo.", home.display());
            }
        }
    }
    println!("{}", describe_state()?);
    Ok(())
}

fn add_remote_and_push(root: &Path, url: &str) -> Result<()> {
    if git(root, &["remote", "get-url", "origin"])?.status.success() {
        bail!("the data repo already has an `origin` remote");
    }
    git_ok(root, &["remote", "add", "origin", url])?;
    let branch = git_ok(root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    git_ok(root, &["push", "--quiet", "-u", "origin", &branch])?;
    Ok(())
}

fn subtree_hint(toplevel: &Path, home: &Path) -> String {
    let prefix = std::fs::canonicalize(home)
        .ok()
        .and_then(|h| h.strip_prefix(toplevel).ok().map(Path::to_path_buf))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<subdir>".into());
    format!(
        "git -C '{}' subtree split --prefix={prefix} -b jimtime-data\n  \
         git clone -b jimtime-data '{}' <new home>   # then push it to a private remote\n  \
         export JIMTIME_HOME=<new home> && jimtime data init --remote <url>",
        toplevel.display(),
        toplevel.display()
    )
}

fn status() -> Result<()> {
    let home = paths::home()?;
    println!("Data home: {}", home.display());
    println!("Sync:      {}", describe_state()?);
    let root = match datarepo::state()? {
        RepoState::Active { root } | RepoState::Disabled { root } => root,
        RepoState::Nested { toplevel } => {
            println!(
                "\nTo give the data its own repo while keeping its history:\n  {}",
                subtree_hint(&toplevel, &home)
            );
            return Ok(());
        }
        RepoState::NotARepo => return Ok(()),
    };
    if let Ok(url) = git_ok(&root, &["remote", "get-url", "origin"]) {
        println!("Remote:    {url}");
    }
    if datarepo::has_upstream(&root) {
        let fetched = git(&root, &["fetch", "--quiet"])?.status.success();
        if let Some((ahead, behind)) = datarepo::ahead_behind(&root) {
            println!(
                "Upstream:  {ahead} to push, {behind} to pull{}",
                if fetched { "" } else { " (could not fetch; as of the last fetch)" }
            );
        }
    }
    let dirty = git_ok(&root, &["status", "--porcelain"])?;
    if !dirty.is_empty() {
        println!("Uncommitted hand edits (`jimtime data sync` commits them):\n{dirty}");
    }
    Ok(())
}

/// Commit any hand edits, then pull and push.
fn sync_now() -> Result<()> {
    let root = match datarepo::state()? {
        RepoState::Active { root } => root,
        _ => bail!("sync is not active: {}", describe_state()?),
    };
    let dirty = git_ok(&root, &["status", "--porcelain"])?;
    if !dirty.is_empty() {
        // A broken file must not reach the other machines.
        check_hand_edits(&root)?;
        git_ok(&root, &["add", "-A"])?;
        git_ok(&root, &["commit", "--quiet", "-m", "data: hand edits"])?;
        println!("Committed hand edits:\n{dirty}");
    }
    if !datarepo::has_upstream(&root) {
        println!("Committed locally; the data repo has no remote (`jimtime data init --remote <url>`).");
        return Ok(());
    }
    let sync = Sync::begin("data sync", true)?;
    sync.commit("data: sync")?;
    let out = git(&root, &["push", "--quiet"])?;
    if !out.status.success() {
        bail!("push failed:\n{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    println!("Data repo is in sync.");
    Ok(())
}

/// Every day file parses and the config validates.
fn check_hand_edits(root: &Path) -> Result<()> {
    for path in crate::store::day_files(&root.join("entries"))? {
        Day::load_path(&path).context("a hand-edited day file is invalid; fix it before syncing")?;
    }
    crate::config::Config::load_optional()
        .context("the hand-edited config is invalid; fix it before syncing")?;
    Ok(())
}

/// `%O %A %B %P`: merge into the `ours` file, or fail so git reports a conflict.
fn merge_day(base: &Path, ours: &Path, theirs: &Path, path: Option<&str>) -> Result<()> {
    let name = path.unwrap_or("day file");
    // With no common ancestor, git passes an empty base file.
    let base_text = std::fs::read_to_string(base).unwrap_or_default();
    let base_day = if base_text.trim().is_empty() {
        None
    } else {
        Some(Day::parse(&base_text).with_context(|| format!("parsing the base of {name}"))?)
    };
    let our_day = Day::load_path(ours).with_context(|| format!("parsing our {name}"))?;
    let their_day = Day::load_path(theirs).with_context(|| format!("parsing their {name}"))?;

    let merged = merge_days(base_day.as_ref(), &our_day, &their_day)
        .with_context(|| format!("jimtime could not merge {name}"))?;
    for (old, new) in &merged.renumbered {
        eprintln!("jimtime: {name}: both sides created {old}; one is now {new}");
    }
    write_atomic(ours, merged.day.to_json()?.as_bytes())
}
