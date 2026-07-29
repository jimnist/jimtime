//! The repo->Harvest mapping config (`config/harvest-projects.json`).

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

use crate::paths;
use crate::repo;

#[derive(Deserialize)]
pub struct Mappings {
    #[serde(default)]
    pub repos: Vec<RepoMapping>,
    #[serde(default)]
    pub aliases: HashMap<String, TaskAlias>,
}

#[derive(Deserialize, Clone)]
pub struct RepoMapping {
    pub repo_path: String,
    pub client_id: u64,
    pub client_name: String,
    pub project_id: u64,
    pub project_name: String,
    pub default_task_id: u64,
    pub default_task_name: String,
    pub billable: bool,
}

#[derive(Deserialize, Clone)]
pub struct TaskAlias {
    pub task_id: u64,
    pub task_name: String,
}

impl Mappings {
    /// Load the mapping config, with a helpful error if it is missing.
    pub fn load() -> Result<Self> {
        let path = paths::mapping_file()?;
        if !path.exists() {
            bail!(
                "no mapping config found at:\n{}\n\n\
                 Create it with a `repos` array mapping each repo's absolute path \
                 to a Harvest client/project/task.",
                path.display()
            );
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Find the mapping for a repo, comparing canonical paths. Errors with a
    /// pointer to the config if the repo is unmapped.
    pub fn for_repo(&self, repo_path: &Path) -> Result<&RepoMapping> {
        let target = repo::canonical(repo_path.to_path_buf());
        self.repos
            .iter()
            .find(|m| repo::canonical(m.repo_path.clone().into()) == target)
            .ok_or_else(|| {
                anyhow!(
                    "no Harvest mapping found for repo:\n{}\n\nAdd a mapping to:\n{}",
                    target.display(),
                    paths::mapping_file()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                )
            })
    }

    /// Resolve a task alias (e.g. `development`) to its id and name.
    pub fn alias(&self, name: &str) -> Result<&TaskAlias> {
        self.aliases
            .get(name)
            .ok_or_else(|| anyhow!("unknown task alias {name:?}; not defined in aliases"))
    }
}
