//! The per-day JSON store: the source of truth. [ADR-0001, ADR-0002]

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;
use crate::slug::slugify;

#[derive(Serialize, Deserialize, Default)]
pub struct Day {
    pub date: String,
    #[serde(default)]
    pub sections: Vec<Section>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Section {
    pub repo_path: String,
    pub client_id: u64,
    pub client_name: String,
    pub project_id: u64,
    pub project_name: String,
    pub task_id: u64,
    pub task_name: String,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub id: String,
    pub hours: f64,
    pub billable: bool,
    #[serde(default)]
    pub needs_review: bool,
    pub notes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harvest_time_entry_id: Option<u64>,
}

impl Day {
    /// Load the store for a date, or `None` if no file exists yet.
    pub fn load(date: &str) -> Result<Option<Day>> {
        let path = paths::day_file(date)?;
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let day =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Some(day))
    }

    /// Load the store for a date, or a fresh empty `Day`.
    pub fn load_or_new(date: &str) -> Result<Day> {
        Ok(Day::load(date)?.unwrap_or_else(|| Day {
            date: date.to_string(),
            sections: Vec::new(),
        }))
    }

    /// Write the store to disk, creating parent directories, as pretty JSON.
    pub fn save(&self) -> Result<()> {
        let path = paths::day_file(&self.date)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, text + "\n")
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// Find the index of the section matching a repo/client/project/task,
    /// if one exists on this day.
    fn section_index(
        &self,
        repo_path: &str,
        client_id: u64,
        project_id: u64,
        task_id: u64,
    ) -> Option<usize> {
        self.sections.iter().position(|s| {
            s.repo_path == repo_path
                && s.client_id == client_id
                && s.project_id == project_id
                && s.task_id == task_id
        })
    }

    /// Append an entry, creating its section if needed, and return its ID.
    #[allow(clippy::too_many_arguments)]
    pub fn add_entry(
        &mut self,
        proto: Section,
        hours: f64,
        billable: bool,
        needs_review: bool,
        notes: String,
    ) -> String {
        let idx = match self.section_index(
            &proto.repo_path,
            proto.client_id,
            proto.project_id,
            proto.task_id,
        ) {
            Some(i) => i,
            None => {
                self.sections.push(Section {
                    entries: Vec::new(),
                    approved: false,
                    ..proto.clone()
                });
                self.sections.len() - 1
            }
        };

        let section = &mut self.sections[idx];
        let id = next_entry_id(
            &self.date,
            &section.client_name,
            &section.project_name,
            &section.task_name,
            &section.entries,
        );
        section.entries.push(Entry {
            id: id.clone(),
            hours,
            billable,
            needs_review,
            notes,
            harvest_time_entry_id: None,
        });
        id
    }
}

/// `YYYY-MM-DD-<client>-<project>-<task>-###`, suffix incrementing within a
/// section (max existing suffix + 1, so it survives deletions).
fn next_entry_id(
    date: &str,
    client: &str,
    project: &str,
    task: &str,
    existing: &[Entry],
) -> String {
    let prefix = format!(
        "{date}-{}-{}-{}",
        slugify(client),
        slugify(project),
        slugify(task)
    );
    let next = existing
        .iter()
        .filter_map(|e| e.id.strip_prefix(&format!("{prefix}-")))
        .filter_map(|suffix| suffix.parse::<u32>().ok())
        .max()
        .map(|n| n + 1)
        .unwrap_or(1);
    format!("{prefix}-{next:03}")
}
