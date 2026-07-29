//! Detecting the git repo the command was invoked from.
//!
//! The binary runs in the caller's working directory (no wrapper), so a plain
//! `git rev-parse --show-toplevel` resolves the real working repo.

use anyhow::{Result, bail};
use std::path::PathBuf;
use std::process::Command;

/// The canonical toplevel path of the git repo containing the current
/// directory. Errors clearly if the cwd is not inside a git repo.
pub fn current_repo() -> Result<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output();

    let out = match out {
        Ok(o) => o,
        Err(_) => bail!("could not run `git`; is it installed and on PATH?"),
    };

    if !out.status.success() {
        bail!(
            "current directory is not inside a git repo.\n\
             Run this from a mapped project repo, or use a command that does not \
             require repo context."
        );
    }

    let path = String::from_utf8(out.stdout)?.trim().to_string();
    Ok(canonical(PathBuf::from(path)))
}

/// Resolve symlinks so two spellings of the same path compare equal.
/// Falls back to the input if canonicalization fails.
pub fn canonical(p: PathBuf) -> PathBuf {
    std::fs::canonicalize(&p).unwrap_or(p)
}
