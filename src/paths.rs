//! Resolving the data home and the paths within it.
//!
//! The data home is `$JIMTIME_HOME` if set, otherwise the XDG data dir
//! (`~/.local/share/jimtime`). Keeping this in the environment means the code
//! carries no personal paths and stays shareable.

use anyhow::{Result, anyhow};
use std::path::PathBuf;

/// The data home: `$JIMTIME_HOME`, or the XDG data dir as a fallback.
pub fn home() -> Result<PathBuf> {
    if let Some(h) = std::env::var_os("JIMTIME_HOME") {
        let p = PathBuf::from(h);
        if p.as_os_str().is_empty() {
            return Err(anyhow!("JIMTIME_HOME is set but empty"));
        }
        return Ok(p);
    }
    dirs::data_dir()
        .map(|d| d.join("jimtime"))
        .ok_or_else(|| anyhow!("could not resolve a data directory; set JIMTIME_HOME"))
}

/// The config directory; templates are resolved relative to it.
pub fn config_dir() -> Result<PathBuf> {
    Ok(home()?.join("config"))
}

/// The config file. [ADR-0006]
pub fn config_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("jimtime.toml"))
}

/// The Harvest config, beside the main one. [ADR-0006]
pub fn harvest_config_file() -> Result<PathBuf> {
    Ok(config_dir()?.join(crate::config::HARVEST_FILE))
}

/// The pre-ADR-0006 repo->Harvest mapping, read only by `config migrate`.
pub fn legacy_mapping_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("harvest-projects.json"))
}

/// Root of the per-day store.
pub fn entries_dir() -> Result<PathBuf> {
    Ok(home()?.join("entries"))
}

/// Path to the store file for a given `YYYY-MM-DD` date.
pub fn day_file(date: &str) -> Result<PathBuf> {
    // date is validated upstream; take the year/month components for the tree.
    let (year, month) = (&date[0..4], &date[5..7]);
    Ok(entries_dir()?
        .join(year)
        .join(month)
        .join(format!("{date}.json")))
}

/// Root of the finalized invoices: `invoices/YYYY/<number>.{json,pdf}`.
pub fn invoices_dir() -> Result<PathBuf> {
    Ok(home()?.join("invoices"))
}

/// Where drafts are rendered. Ignored by git: a draft is not a record.
pub fn drafts_dir() -> Result<PathBuf> {
    Ok(invoices_dir()?.join(".drafts"))
}
