//! Keeping the data home in sync as its own git repo. [ADR-0009]
//!
//! Every command that writes wraps its work in a [`Sync`]: `begin` pulls, the
//! command writes (each writer calls [`note_write`]), and `commit` commits
//! exactly those files and pushes. Sync is active only when `$JIMTIME_HOME` is
//! the toplevel of its own repo; jimtime never touches a repo it does not own.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;

use crate::config::Config;
use crate::paths;

/// Files written during this process, committed by [`Sync::commit`].
static WRITES: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Record that a file in the data home was written.
pub fn note_write(path: &Path) {
    let mut w = WRITES.lock().unwrap_or_else(|e| e.into_inner());
    if !w.iter().any(|p| p == path) {
        w.push(path.to_path_buf());
    }
}

fn take_writes() -> Vec<PathBuf> {
    std::mem::take(&mut *WRITES.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Routes day files to the semantic merge driver.
pub const GITATTRIBUTES: &str = "entries/**/*.json merge=jimtime-day\n";
/// Drafts are previews, never records.
pub const GITIGNORE: &str = "invoices/.drafts/\n.DS_Store\n";
const DRIVER: &str = "jimtime-day";

/// Where the data home stands with respect to git.
pub enum RepoState {
    /// `$JIMTIME_HOME` is the toplevel of its own repo and sync is on.
    Active { root: PathBuf },
    /// It is its own repo, but `[git] auto_sync = false`.
    Disabled { root: PathBuf },
    /// It sits inside another repo, which jimtime leaves alone.
    Nested { toplevel: PathBuf },
    /// Not in a git repo at all.
    NotARepo,
}

pub fn state() -> Result<RepoState> {
    let home = paths::home()?;
    if !home.exists() {
        return Ok(RepoState::NotARepo);
    }
    let out = Command::new("git")
        .arg("-C")
        .arg(&home)
        .args(["rev-parse", "--show-toplevel"])
        .output();
    let Ok(out) = out else {
        return Ok(RepoState::NotARepo);
    };
    if !out.status.success() {
        return Ok(RepoState::NotARepo);
    }
    let toplevel = canonical(PathBuf::from(String::from_utf8(out.stdout)?.trim()));
    let root = canonical(home);
    if toplevel != root {
        return Ok(RepoState::Nested { toplevel });
    }
    let auto = Config::load_optional()?
        .map(|c| c.git.auto_sync)
        .unwrap_or(true);
    Ok(if auto { RepoState::Active { root } } else { RepoState::Disabled { root } })
}

fn canonical(p: PathBuf) -> PathBuf {
    std::fs::canonicalize(&p).unwrap_or(p)
}

/// Run git in `root`, returning the output whether or not it succeeded.
pub fn git(root: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .context("could not run `git`; is it installed and on PATH?")
}

/// Run git in `root` and fail with its stderr if it fails.
pub fn git_ok(root: &Path, args: &[&str]) -> Result<String> {
    let out = git(root, args)?;
    if !out.status.success() {
        bail!(
            "git {} failed:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Whether the current branch tracks a remote branch.
pub fn has_upstream(root: &Path) -> bool {
    git(
        root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .map(|o| o.status.success())
    .unwrap_or(false)
}

/// `(ahead, behind)` of the upstream, from the last fetch.
pub fn ahead_behind(root: &Path) -> Option<(u32, u32)> {
    let s = git_ok(
        root,
        &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
    )
    .ok()?;
    let mut it = s.split_whitespace().filter_map(|n| n.parse().ok());
    Some((it.next()?, it.next()?))
}

/// Make sure the repo routes day files to the merge driver. `.gitattributes`
/// and `.gitignore` are versioned; the driver itself lives in `.git/config`,
/// which is not, so it is (re-)registered on every sync.
pub fn ensure_setup(root: &Path) -> Result<()> {
    for (name, content) in [(".gitattributes", GITATTRIBUTES), (".gitignore", GITIGNORE)] {
        let path = root.join(name);
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let missing: String = content
            .lines()
            .filter(|l| !existing.lines().any(|e| e.trim() == *l))
            .map(|l| format!("{l}\n"))
            .collect();
        if !missing.is_empty() {
            let mut text = existing;
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&missing);
            std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
            note_write(&path);
        }
    }

    let exe = std::env::current_exe().context("locating the jimtime binary")?;
    let driver = format!("'{}' data merge-day %O %A %B %P", exe.display());
    let key = format!("merge.{DRIVER}.driver");
    let current = git(root, &["config", "--get", &key])
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    if current.as_deref() != Some(driver.as_str()) {
        git_ok(
            root,
            &[
                "config",
                &format!("merge.{DRIVER}.name"),
                "jimtime day-file merge",
            ],
        )?;
        git_ok(root, &["config", &key, &driver])?;
    }
    Ok(())
}

/// A write transaction against the data repo.
pub struct Sync {
    root: Option<PathBuf>,
    label: String,
    done: bool,
}

impl Sync {
    /// Start a write: pull first so the write lands on the latest data.
    ///
    /// A failed pull (offline) is a warning, unless `require_remote` is set, in
    /// which case it is an error: finalizing an invoice must see the latest
    /// invoice numbers. [ADR-0008]
    pub fn begin(label: &str, require_remote: bool) -> Result<Sync> {
        take_writes();
        let root = match state()? {
            RepoState::Active { root } => Some(root),
            _ => None,
        };
        if let Some(root) = &root {
            ensure_setup(root)?;
            if has_upstream(root) {
                pull(root, require_remote)?;
            }
        }
        Ok(Sync {
            root,
            label: label.to_string(),
            done: false,
        })
    }

    /// Commit the files written since `begin` with this message, then push.
    pub fn commit(mut self, message: &str) -> Result<()> {
        self.done = true;
        self.commit_inner(message)
    }

    /// Commit what has been written so far without ending the transaction,
    /// for multi-step commands whose later steps may fail.
    pub fn checkpoint(&self, message: &str) -> Result<()> {
        self.commit_inner(message)
    }

    fn commit_inner(&self, message: &str) -> Result<()> {
        let writes = take_writes();
        let Some(root) = &self.root else {
            return Ok(());
        };
        let files: Vec<String> = writes
            .iter()
            .map(|p| canonical(p.clone()))
            .filter(|p| p.starts_with(root))
            .map(|p| p.display().to_string())
            .collect();
        if files.is_empty() {
            return Ok(());
        }
        let mut add = vec!["add", "--"];
        add.extend(files.iter().map(String::as_str));
        git_ok(root, &add)?;

        let mut staged = vec!["diff", "--cached", "--quiet", "--"];
        staged.extend(files.iter().map(String::as_str));
        if git(root, &staged)?.status.success() {
            return Ok(()); // rewritten, but byte-identical
        }
        let mut commit = vec!["commit", "--quiet", "-m", message, "--"];
        commit.extend(files.iter().map(String::as_str));
        git_ok(root, &commit)?;

        if has_upstream(root) {
            let out = git(root, &["push", "--quiet"])?;
            if !out.status.success() {
                eprintln!(
                    "warning: committed locally, but could not push the data repo ({}); \
                     the next command will retry.",
                    first_line(&out.stderr)
                );
            }
        }
        Ok(())
    }
}

impl Drop for Sync {
    /// A command that bailed part-way may still have saved files (a Harvest
    /// push saves each id as it goes). Commit them so the record keeps up.
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let msg = format!("{} (incomplete)", self.label);
        if let Err(e) = self.commit_inner(&msg) {
            eprintln!("warning: could not commit the data repo: {e:#}");
        }
    }
}

/// `git pull --rebase`, with the merge driver handling day files. A rebase that
/// still conflicts is aborted so the data home is never left mid-rebase.
fn pull(root: &Path, required: bool) -> Result<()> {
    let out = git(root, &["pull", "--rebase", "--autostash", "--quiet"])?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Pass on what the merge driver said (e.g. an entry it renumbered).
    for line in stderr.lines().filter(|l| l.starts_with("jimtime:")) {
        eprintln!("{line}");
    }
    if out.status.success() {
        return Ok(());
    }
    if rebase_in_progress(root) {
        let _ = git(root, &["rebase", "--abort"]);
        // git's hints describe continuing the rebase, which we just aborted.
        let detail: Vec<&str> = stderr
            .lines()
            .filter(|l| {
                !l.starts_with("hint:")
                    && !l.starts_with("jimtime:")
                    // git says it twice; keep the `error:` one.
                    && !l.starts_with("Could not apply")
                    && !l.trim().is_empty()
            })
            .collect();
        bail!(
            "pulling the data repo hit a conflict jimtime could not merge on its own:\n  {}\n\n\
             Nothing was changed. Resolve it by hand with:\n  git -C '{}' pull --rebase",
            detail.join("\n  "),
            root.display()
        );
    }
    if required {
        bail!(
            "could not pull the data repo ({}), and this command needs the latest data",
            first_line(&out.stderr)
        );
    }
    eprintln!(
        "warning: could not pull the data repo ({}); continuing offline.",
        first_line(&out.stderr)
    );
    Ok(())
}

/// The first non-empty line of git's stderr, without its `fatal: ` prefix.
fn first_line(stderr: &[u8]) -> String {
    let s = String::from_utf8_lossy(stderr);
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| {
            l.trim_start_matches("fatal: ")
                .trim_start_matches("error: ")
                .to_string()
        })
        .unwrap_or_else(|| "no detail".into())
}

fn rebase_in_progress(root: &Path) -> bool {
    ["rebase-merge", "rebase-apply"].iter().any(|d| {
        git_ok(root, &["rev-parse", "--git-path", d])
            .map(|p| root.join(p).exists())
            .unwrap_or(false)
    })
}
