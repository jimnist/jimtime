//! The per-day JSON store: the source of truth. [ADR-0001, ADR-0002]
//!
//! Approval is per-entry [ADR-0004]. Older files carried a single `approved`
//! flag on the section; those are migrated on load.

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
    /// Legacy section-level approval. Read for migration only; new files store
    /// approval on each entry and omit this.
    #[serde(default, skip_serializing_if = "is_false")]
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
    pub approved: bool,
    #[serde(default)]
    pub needs_review: bool,
    pub notes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harvest_time_entry_id: Option<u64>,
}

impl Entry {
    /// Eligible to push: approved, billable (unless including non-billable), and
    /// not already imported.
    pub fn is_pushable(&self, include_non_billable: bool) -> bool {
        self.approved
            && (self.billable || include_non_billable)
            && self.harvest_time_entry_id.is_none()
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Day {
    /// Load the store for a date, or `None` if no file exists yet. Applies the
    /// legacy section-approval migration.
    pub fn load(date: &str) -> Result<Option<Day>> {
        let path = paths::day_file(date)?;
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let mut day: Day =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        day.migrate_section_approval();
        Ok(Some(day))
    }

    /// Load the store for a date, or a fresh empty `Day`.
    pub fn load_or_new(date: &str) -> Result<Day> {
        Ok(Day::load(date)?.unwrap_or_else(|| Day {
            date: date.to_string(),
            sections: Vec::new(),
        }))
    }

    /// Legacy files approved whole sections. Push that down to the entries and
    /// clear the section flag, so approval is uniformly per-entry.
    fn migrate_section_approval(&mut self) {
        for s in &mut self.sections {
            if s.approved {
                for e in &mut s.entries {
                    e.approved = true;
                }
                s.approved = false;
            }
        }
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

    /// Find the index of the section matching a repo/client/project/task.
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
            approved: false,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn proto() -> Section {
        Section {
            repo_path: "/tmp/acme".into(),
            client_id: 1,
            client_name: "Acme Corp".into(),
            project_id: 2,
            project_name: "Billing Portal".into(),
            task_id: 3,
            task_name: "Development".into(),
            approved: false,
            entries: vec![],
        }
    }

    #[test]
    fn add_entry_creates_then_reuses_section_and_increments_id() {
        let mut day = Day {
            date: "2026-07-28".into(),
            sections: vec![],
        };
        let id1 = day.add_entry(proto(), 1.0, true, false, "one".into());
        let id2 = day.add_entry(proto(), 0.5, true, true, "two".into());

        assert_eq!(day.sections.len(), 1, "same section reused");
        assert_eq!(id1, "2026-07-28-acme-corp-billing-portal-development-001");
        assert_eq!(id2, "2026-07-28-acme-corp-billing-portal-development-002");
        assert!(!day.sections[0].entries[0].approved);
        assert!(day.sections[0].entries[1].needs_review);
    }

    #[test]
    fn different_task_makes_a_new_section() {
        let mut day = Day {
            date: "2026-07-28".into(),
            sections: vec![],
        };
        day.add_entry(proto(), 1.0, true, false, "dev".into());
        let mut meetings = proto();
        meetings.task_id = 4;
        meetings.task_name = "Meetings".into();
        day.add_entry(meetings, 0.5, false, false, "call".into());
        assert_eq!(day.sections.len(), 2);
    }

    #[test]
    fn legacy_section_approval_migrates_to_entries() {
        let legacy = r#"{
            "date": "2026-07-20",
            "sections": [{
                "repo_path": "/tmp/acme", "client_id": 1, "client_name": "Acme",
                "project_id": 2, "project_name": "Portal", "task_id": 3, "task_name": "Dev",
                "approved": true,
                "entries": [{ "id": "x-001", "hours": 1.0, "billable": true, "notes": "n" }]
            }]
        }"#;
        let mut day: Day = serde_json::from_str(legacy).unwrap();
        day.migrate_section_approval();
        assert!(!day.sections[0].approved, "section flag cleared");
        assert!(day.sections[0].entries[0].approved, "pushed down to entry");
    }

    #[test]
    fn is_pushable_predicate() {
        let mut e = Entry {
            id: "x".into(),
            hours: 1.0,
            billable: true,
            approved: true,
            needs_review: false,
            notes: "n".into(),
            harvest_time_entry_id: None,
        };
        assert!(e.is_pushable(false));
        e.approved = false;
        assert!(!e.is_pushable(false));
        e.approved = true;
        e.billable = false;
        assert!(!e.is_pushable(false));
        assert!(e.is_pushable(true), "non-billable included when asked");
        e.billable = true;
        e.harvest_time_entry_id = Some(9);
        assert!(!e.is_pushable(false), "already imported");
    }
}
