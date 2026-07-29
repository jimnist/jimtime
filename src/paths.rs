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

/// Path to the repo->Harvest mapping config.
pub fn mapping_file() -> Result<PathBuf> {
    Ok(home()?.join("config").join("harvest-projects.json"))
}

/// Path to the store file for a given `YYYY-MM-DD` date.
pub fn day_file(date: &str) -> Result<PathBuf> {
    // date is validated upstream; take the year/month components for the tree.
    let (year, month) = (&date[0..4], &date[5..7]);
    Ok(home()?
        .join("entries")
        .join(year)
        .join(month)
        .join(format!("{date}.json")))
}
