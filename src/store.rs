//! The per-day JSON store: the source of truth. [ADR-0001, ADR-0002]
//!
//! Approval is per-entry [ADR-0004]. Older files carried a single `approved`
//! flag on the section; those are migrated on load. Sections are identified by
//! client/project/task keys; Harvest ids are optional [ADR-0006]. Files written
//! before keys existed get them by slugifying the names on load.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::datarepo;
use crate::paths;
use crate::slug::slugify;

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
pub struct Day {
    pub date: String,
    #[serde(default)]
    pub sections: Vec<Section>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
pub struct Section {
    pub repo_path: String,
    #[serde(default)]
    pub client: String,
    pub client_name: String,
    #[serde(default)]
    pub project: String,
    pub project_name: String,
    #[serde(default)]
    pub task: String,
    pub task_name: String,
    /// Harvest ids, used only by the Harvest integration. Read from the legacy
    /// `client_id`/`project_id`/`task_id` names too.
    #[serde(default, alias = "client_id", skip_serializing_if = "Option::is_none")]
    pub harvest_client_id: Option<u64>,
    #[serde(default, alias = "project_id", skip_serializing_if = "Option::is_none")]
    pub harvest_project_id: Option<u64>,
    #[serde(default, alias = "task_id", skip_serializing_if = "Option::is_none")]
    pub harvest_task_id: Option<u64>,
    /// Legacy section-level approval. Read for migration only; new files store
    /// approval on each entry and omit this.
    #[serde(default, skip_serializing_if = "is_false")]
    pub approved: bool,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

impl Section {
    /// The identity of a section within a day.
    pub fn key(&self) -> (&str, &str, &str, &str) {
        (&self.repo_path, &self.client, &self.project, &self.task)
    }

    /// `Client - Project - Task`, for display.
    pub fn label(&self) -> String {
        format!(
            "{} - {} - {}",
            self.client_name, self.project_name, self.task_name
        )
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
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
    /// The number of the invoice this entry is billed on. Set by `invoice
    /// finalize`, cleared by `invoice void`. [ADR-0008]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice: Option<String>,
}

impl Entry {
    /// Eligible to push: approved, billable (unless including non-billable), and
    /// not already imported.
    pub fn is_pushable(&self, include_non_billable: bool) -> bool {
        self.approved
            && (self.billable || include_non_billable)
            && self.harvest_time_entry_id.is_none()
    }

    /// Eligible to invoice: approved, billable, and not on an invoice yet.
    /// [ADR-0008]
    pub fn is_invoiceable(&self) -> bool {
        self.approved && self.billable && self.invoice.is_none()
    }

    /// Linked to something outside the store (Harvest or an invoice), so its
    /// ID must never change.
    pub fn is_linked(&self) -> bool {
        self.harvest_time_entry_id.is_some() || self.invoice.is_some()
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Day {
    pub fn new(date: &str) -> Day {
        Day {
            date: date.to_string(),
            sections: Vec::new(),
        }
    }

    /// Load the store for a date, or `None` if no file exists yet. Applies the
    /// legacy migrations.
    pub fn load(date: &str) -> Result<Option<Day>> {
        let path = paths::day_file(date)?;
        if !path.exists() {
            return Ok(None);
        }
        Day::load_path(&path).map(Some)
    }

    /// Load a day file from an explicit path (the merge driver reads git's
    /// temp files). Applies the legacy migrations.
    pub fn load_path(path: &Path) -> Result<Day> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Day::parse(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Day> {
        let mut day: Day = serde_json::from_str(text)?;
        day.migrate();
        Ok(day)
    }

    /// Load the store for a date, or a fresh empty `Day`.
    pub fn load_or_new(date: &str) -> Result<Day> {
        Ok(Day::load(date)?.unwrap_or_else(|| Day::new(date)))
    }

    fn migrate(&mut self) {
        self.migrate_section_approval();
        self.fill_missing_keys();
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

    /// Files written before ADR-0006 have names but no keys. `config migrate`
    /// writes proper keys; this keeps unmigrated files readable meanwhile.
    fn fill_missing_keys(&mut self) {
        for s in &mut self.sections {
            if s.client.is_empty() {
                s.client = slugify(&s.client_name);
            }
            if s.project.is_empty() {
                s.project = slugify(&s.project_name);
            }
            if s.task.is_empty() {
                s.task = slugify(&s.task_name);
            }
        }
    }

    /// Serialize as the pretty JSON written to disk.
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)? + "\n")
    }

    /// Write the store to disk atomically and note it for the data repo commit.
    pub fn save(&self) -> Result<()> {
        let path = paths::day_file(&self.date)?;
        write_atomic(&path, self.to_json()?.as_bytes())?;
        datarepo::note_write(&path);
        Ok(())
    }

    /// Find the index of the section with this identity.
    fn section_index(&self, proto: &Section) -> Option<usize> {
        self.sections.iter().position(|s| s.key() == proto.key())
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
        let idx = match self.section_index(&proto) {
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
            invoice: None,
        });
        id
    }

    /// Every entry of the day with its section.
    #[cfg(test)]
    pub fn entries(&self) -> impl Iterator<Item = (&Section, &Entry)> {
        self.sections
            .iter()
            .flat_map(|s| s.entries.iter().map(move |e| (s, e)))
    }
}

/// Write a file via a temp file and rename, so a crash never leaves a
/// half-written billing record. Creates parent directories.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("creating a temp file in {}", parent.display()))?;
    std::io::Write::write_all(&mut tmp, bytes)
        .with_context(|| format!("writing {}", path.display()))?;
    // Temp files are created private (0600). Keep the mode the file already
    // had, or give a new one the usual 0644, so a save never changes it.
    let perms = match std::fs::metadata(path) {
        Ok(m) => m.permissions(),
        Err(_) => default_permissions(),
    };
    tmp.as_file()
        .set_permissions(perms)
        .with_context(|| format!("setting permissions on {}", path.display()))?;
    tmp.persist(path)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn default_permissions() -> std::fs::Permissions {
    std::os::unix::fs::PermissionsExt::from_mode(0o644)
}

#[cfg(not(unix))]
fn default_permissions() -> std::fs::Permissions {
    // Only the read-only bit exists elsewhere; a fresh temp file has it clear.
    let mut p = std::fs::metadata(std::env::temp_dir())
        .map(|m| m.permissions())
        .expect("temp dir exists");
    p.set_readonly(false);
    p
}

/// The ID prefix shared by a section's entries: `YYYY-MM-DD-<client>-<project>-<task>`.
pub fn id_prefix(date: &str, client: &str, project: &str, task: &str) -> String {
    format!(
        "{date}-{}-{}-{}",
        slugify(client),
        slugify(project),
        slugify(task)
    )
}

/// `YYYY-MM-DD-<client>-<project>-<task>-###`, suffix incrementing within a
/// section (max existing suffix + 1, so it survives deletions).
pub fn next_entry_id(
    date: &str,
    client: &str,
    project: &str,
    task: &str,
    existing: &[Entry],
) -> String {
    let prefix = id_prefix(date, client, project, task);
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
            client: "acme".into(),
            client_name: "Acme Corp".into(),
            project: "portal".into(),
            project_name: "Billing Portal".into(),
            task: "dev".into(),
            task_name: "Development".into(),
            ..Section::default()
        }
    }

    fn entry(id: &str) -> Entry {
        Entry {
            id: id.into(),
            hours: 1.0,
            billable: true,
            approved: true,
            needs_review: false,
            notes: "n".into(),
            harvest_time_entry_id: None,
            invoice: None,
        }
    }

    #[test]
    fn add_entry_creates_then_reuses_section_and_increments_id() {
        let mut day = Day::new("2026-07-28");
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
        let mut day = Day::new("2026-07-28");
        day.add_entry(proto(), 1.0, true, false, "dev".into());
        let mut meetings = proto();
        meetings.task = "meetings".into();
        meetings.task_name = "Meetings".into();
        day.add_entry(meetings, 0.5, false, false, "call".into());
        assert_eq!(day.sections.len(), 2);
    }

    #[test]
    fn legacy_file_migrates_approval_keys_and_harvest_ids() {
        let legacy = r#"{
            "date": "2026-07-20",
            "sections": [{
                "repo_path": "/tmp/acme", "client_id": 1, "client_name": "Acme",
                "project_id": 2, "project_name": "Billing Portal", "task_id": 3, "task_name": "Dev",
                "approved": true,
                "entries": [{ "id": "x-001", "hours": 1.0, "billable": true, "notes": "n" }]
            }]
        }"#;
        let day = Day::parse(legacy).unwrap();
        let s = &day.sections[0];
        assert!(!s.approved, "section flag cleared");
        assert!(s.entries[0].approved, "pushed down to entry");
        assert_eq!(s.key(), ("/tmp/acme", "acme", "billing-portal", "dev"));
        assert_eq!(
            (s.harvest_client_id, s.harvest_project_id, s.harvest_task_id),
            (Some(1), Some(2), Some(3))
        );

        let json = day.to_json().unwrap();
        assert!(json.contains("\"harvest_project_id\": 2"), "{json}");
        assert!(
            !json.contains("\"project_id\""),
            "old names are not written back"
        );
    }

    #[test]
    fn a_section_without_harvest_ids_omits_them() {
        let mut day = Day::new("2026-07-28");
        day.add_entry(proto(), 1.0, true, false, "x".into());
        let json = day.to_json().unwrap();
        assert!(!json.contains("harvest"), "{json}");
        assert!(!json.contains("invoice"), "{json}");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_writes_keep_the_mode_or_default_to_0644() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b.json");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;

        write_atomic(&path, b"one").unwrap();
        assert_eq!(mode(&path), 0o644, "new files are not private");

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(mode(&path), 0o640, "an existing mode is kept");
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
    }

    #[test]
    fn is_pushable_predicate() {
        let mut e = entry("x");
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

    #[test]
    fn is_invoiceable_predicate() {
        let mut e = entry("x");
        assert!(e.is_invoiceable());
        e.approved = false;
        assert!(!e.is_invoiceable(), "unapproved");
        e.approved = true;
        e.billable = false;
        assert!(!e.is_invoiceable(), "non-billable");
        e.billable = true;
        e.invoice = Some("2026-001".into());
        assert!(!e.is_invoiceable(), "already invoiced");
        e.invoice = None;
        e.harvest_time_entry_id = Some(9);
        assert!(
            e.is_invoiceable(),
            "Harvest and local invoicing are independent"
        );
    }

    #[test]
    fn clearing_the_harvest_link_makes_an_entry_pushable_again() {
        // The contract `harvest unpush` depends on: dropping the id is the whole
        // undo. Without it an auto-pushed entry is stuck, because `unapprove`
        // refuses to touch anything carrying a Harvest id.
        let mut e = entry("x");
        e.harvest_time_entry_id = Some(9);
        assert!(!e.is_pushable(false));
        e.harvest_time_entry_id = None;
        assert!(e.is_pushable(false), "unpushed entries are pushable again");
    }
}
