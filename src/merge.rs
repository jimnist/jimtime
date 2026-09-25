//! Semantic 3-way merge of a day file, run by git as a merge driver so two
//! machines editing the same day do not produce JSON conflict markers.
//! [ADR-0009]
//!
//! Sections merge by their identity, entries by ID, and each field on its own:
//! a field changed on one side takes that side, a field changed identically on
//! both takes either, and a field changed two different ways is a conflict.
//! When both sides minted the same ID for different work (each machine counts
//! `-001`, `-002`... on its own), the side with no external link is renumbered.
//! Nothing is ever guessed: a real conflict fails the merge.

use anyhow::{Result, bail};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::collections::HashMap;

use crate::store::{Day, Entry, Section, next_entry_id};

type SectionKey = (String, String, String, String);

fn key_of(s: &Section) -> SectionKey {
    let (a, b, c, d) = s.key();
    (a.into(), b.into(), c.into(), d.into())
}

/// A section's metadata, without its entries.
fn meta(s: &Section) -> Section {
    Section {
        entries: Vec::new(),
        ..s.clone()
    }
}

/// The outcome of merging one day.
#[derive(Debug)]
pub struct Merged {
    pub day: Day,
    /// `(old id, new id)` for each entry that was renumbered.
    pub renumbered: Vec<(String, String)>,
}

/// Merge `ours` and `theirs` against their common ancestor `base` (absent when
/// both sides created the file).
pub fn merge_days(base: Option<&Day>, ours: &Day, theirs: &Day) -> Result<Merged> {
    if ours.date != theirs.date {
        bail!(
            "refusing to merge different days: {} vs {}",
            ours.date,
            theirs.date
        );
    }
    let empty = Day::new(&ours.date);
    let base = base.unwrap_or(&empty);
    let mut conflicts: Vec<String> = Vec::new();

    // Section metadata, in order: ours first, then any new ones from theirs.
    let mut order: Vec<SectionKey> = Vec::new();
    let mut metas: HashMap<SectionKey, Section> = HashMap::new();
    let base_meta: HashMap<SectionKey, Section> =
        base.sections.iter().map(|s| (key_of(s), meta(s))).collect();
    let their_meta: HashMap<SectionKey, Section> = theirs
        .sections
        .iter()
        .map(|s| (key_of(s), meta(s)))
        .collect();
    for s in ours.sections.iter().chain(theirs.sections.iter()) {
        let k = key_of(s);
        if metas.contains_key(&k) {
            continue;
        }
        let our = ours.sections.iter().find(|x| key_of(x) == k).map(meta);
        let their = their_meta.get(&k).cloned();
        let merged = match (our, their) {
            (Some(o), Some(t)) => match merge_value(base_meta.get(&k), &o, &t) {
                Ok(m) => m,
                Err(fields) => {
                    conflicts.push(format!("section {k:?}: {} changed on both sides", fields));
                    o
                }
            },
            (Some(o), None) => o,
            (None, Some(t)) => t,
            (None, None) => unreachable!("key came from one of the sides"),
        };
        order.push(k.clone());
        metas.insert(k, merged);
    }

    // Entries by ID, remembering which section each one sits in.
    let index = |d: &Day| -> Vec<(String, SectionKey, Entry)> {
        d.sections
            .iter()
            .flat_map(|s| {
                s.entries
                    .iter()
                    .map(move |e| (e.id.clone(), key_of(s), e.clone()))
            })
            .collect()
    };
    let (b_list, o_list, t_list) = (index(base), index(ours), index(theirs));
    let find = |l: &[(String, SectionKey, Entry)], id: &str| {
        l.iter()
            .find(|(i, _, _)| i == id)
            .map(|(_, k, e)| (k.clone(), e.clone()))
    };

    let mut ids: Vec<String> = Vec::new();
    for (id, _, _) in o_list.iter().chain(t_list.iter()).chain(b_list.iter()) {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }

    // (section, entry, renumber?) in output order.
    let mut placed: Vec<(SectionKey, Entry, bool)> = Vec::new();
    for id in &ids {
        let (b, o, t) = (find(&b_list, id), find(&o_list, id), find(&t_list, id));
        match (b, o, t) {
            // Unchanged-or-identical on both sides.
            (_, Some(o), Some(t)) if o == t => placed.push((o.0, o.1, false)),
            // Both sides added this ID for different work: keep one, renumber
            // the other - whichever is not linked to Harvest or an invoice.
            (None, Some(o), Some(t)) => {
                if !t.1.is_linked() {
                    placed.push((o.0, o.1, false));
                    placed.push((t.0, t.1, true));
                } else if !o.1.is_linked() {
                    placed.push((t.0, t.1, false));
                    placed.push((o.0, o.1, true));
                } else {
                    conflicts.push(format!(
                        "entry {id}: added on both sides and both are linked externally"
                    ));
                }
            }
            (Some(b), Some(o), Some(t)) => {
                let section = pick(&b.0, &o.0, &t.0)
                    .unwrap_or_else(|| {
                        conflicts.push(format!("entry {id}: moved to two different sections"));
                        &o.0
                    })
                    .clone();
                match merge_value(Some(&b.1), &o.1, &t.1) {
                    Ok(e) => placed.push((section, e, false)),
                    Err(fields) => {
                        conflicts.push(format!("entry {id}: {fields} changed on both sides"))
                    }
                }
            }
            // Deleted on one side: fine if the other left it alone.
            (Some(b), Some(o), None) => {
                if o != b {
                    conflicts.push(format!(
                        "entry {id}: deleted on one side, changed on the other"
                    ));
                }
            }
            (Some(b), None, Some(t)) => {
                if t != b {
                    conflicts.push(format!(
                        "entry {id}: deleted on one side, changed on the other"
                    ));
                }
            }
            (None, Some(o), None) => placed.push((o.0, o.1, false)),
            (None, None, Some(t)) => placed.push((t.0, t.1, false)),
            (Some(_), None, None) | (None, None, None) => {}
        }
    }

    if !conflicts.is_empty() {
        bail!("cannot merge {}:\n  {}", ours.date, conflicts.join("\n  "));
    }

    let mut day = Day::new(&ours.date);
    for k in &order {
        let mut s = metas[k].clone();
        s.entries = placed
            .iter()
            .filter(|(pk, _, renumber)| pk == k && !renumber)
            .map(|(_, e, _)| e.clone())
            .collect();
        day.sections.push(s);
    }

    // Renumber after everything else is placed, so new IDs skip every kept one.
    let mut renumbered = Vec::new();
    for (k, e, _) in placed.into_iter().filter(|(_, _, r)| *r) {
        let s = day
            .sections
            .iter_mut()
            .find(|s| key_of(s) == k)
            .expect("every placed section is in order");
        let new_id = next_entry_id(
            &ours.date,
            &s.client_name,
            &s.project_name,
            &s.task_name,
            &s.entries,
        );
        renumbered.push((e.id.clone(), new_id.clone()));
        s.entries.push(Entry { id: new_id, ..e });
    }

    // A section whose entries were all deleted has no reason to stay.
    day.sections.retain(|s| !s.entries.is_empty());

    Ok(Merged { day, renumbered })
}

/// The 3-way pick for one value: whichever side changed it, or `None` if both
/// changed it differently.
fn pick<'a, T: PartialEq>(base: &'a T, ours: &'a T, theirs: &'a T) -> Option<&'a T> {
    if ours == theirs || theirs == base {
        Some(ours)
    } else if ours == base {
        Some(theirs)
    } else {
        None
    }
}

/// Field-by-field 3-way merge of two structs through their JSON form. On
/// conflict, returns the names of the conflicting fields.
fn merge_value<T: Serialize + DeserializeOwned>(
    base: Option<&T>,
    ours: &T,
    theirs: &T,
) -> std::result::Result<T, String> {
    let to_map = |v: &T| -> Map<String, Value> {
        match serde_json::to_value(v).expect("store types serialize") {
            Value::Object(m) => m,
            _ => unreachable!("store types are structs"),
        }
    };
    let (o, t) = (to_map(ours), to_map(theirs));
    let b = base.map(to_map).unwrap_or_default();
    let mut out = Map::new();
    let mut bad = Vec::new();
    for k in o.keys().chain(t.keys()) {
        if out.contains_key(k) || bad.contains(k) {
            continue;
        }
        let (bv, ov, tv) = (b.get(k), o.get(k), t.get(k));
        match pick(&bv, &ov, &tv) {
            Some(Some(v)) => {
                out.insert(k.clone(), (*v).clone());
            }
            Some(None) => {}
            None => bad.push(k.clone()),
        }
    }
    if !bad.is_empty() {
        return Err(bad.join(", "));
    }
    serde_json::from_value(Value::Object(out)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(task: &str) -> Section {
        Section {
            repo_path: "/r".into(),
            client: "acme".into(),
            client_name: "Acme".into(),
            project: "web".into(),
            project_name: "Web".into(),
            task: task.into(),
            task_name: task.into(),
            ..Section::default()
        }
    }

    fn day_with(entries: &[(&str, &str, f64)]) -> Day {
        let mut d = Day::new("2026-09-25");
        for (task, notes, hours) in entries {
            d.add_entry(section(task), *hours, true, false, (*notes).into());
        }
        d
    }

    fn ids(d: &Day) -> Vec<String> {
        d.entries().map(|(_, e)| e.id.clone()).collect()
    }

    #[test]
    fn independent_adds_on_different_tasks_both_survive() {
        let base = day_with(&[("dev", "a", 1.0)]);
        let mut ours = base.clone();
        ours.add_entry(section("dev"), 2.0, true, false, "b".into());
        let mut theirs = base.clone();
        theirs.add_entry(section("ops"), 0.5, true, false, "c".into());

        let m = merge_days(Some(&base), &ours, &theirs).unwrap();
        assert_eq!(m.day.entries().count(), 3);
        assert!(m.renumbered.is_empty());
    }

    #[test]
    fn same_id_minted_on_both_sides_renumbers_the_incoming_one() {
        let base = day_with(&[("dev", "a", 1.0)]);
        let mut ours = base.clone();
        ours.add_entry(section("dev"), 2.0, true, false, "ours".into());
        let mut theirs = base.clone();
        theirs.add_entry(section("dev"), 3.0, true, false, "theirs".into());
        assert_eq!(ids(&ours), ids(&theirs), "both minted -002");

        let m = merge_days(Some(&base), &ours, &theirs).unwrap();
        let notes: Vec<_> = m
            .day
            .entries()
            .map(|(_, e)| (e.id.clone(), e.notes.clone()))
            .collect();
        assert_eq!(notes.len(), 3);
        assert!(notes[1].0.ends_with("-002") && notes[1].1 == "ours");
        assert!(notes[2].0.ends_with("-003") && notes[2].1 == "theirs");
        assert_eq!(m.renumbered.len(), 1);
    }

    #[test]
    fn a_linked_entry_keeps_its_id_and_the_other_side_is_renumbered() {
        let base = Day::new("2026-09-25");
        let mut ours = base.clone();
        ours.add_entry(section("dev"), 1.0, true, false, "ours".into());
        let mut theirs = base.clone();
        theirs.add_entry(section("dev"), 2.0, true, false, "theirs".into());
        theirs.sections[0].entries[0].invoice = Some("2026-001".into());

        let m = merge_days(Some(&base), &ours, &theirs).unwrap();
        let (_, kept) = m
            .day
            .entries()
            .find(|(_, e)| e.id.ends_with("-001"))
            .unwrap();
        assert_eq!(kept.notes, "theirs", "the invoiced entry keeps -001");
        assert_eq!(m.renumbered.len(), 1);
    }

    #[test]
    fn different_fields_changed_on_each_side_merge() {
        let base = day_with(&[("dev", "a", 1.0)]);
        let mut ours = base.clone();
        ours.sections[0].entries[0].approved = true;
        let mut theirs = base.clone();
        theirs.sections[0].entries[0].notes = "better notes".into();

        let m = merge_days(Some(&base), &ours, &theirs).unwrap();
        let e = &m.day.sections[0].entries[0];
        assert!(e.approved);
        assert_eq!(e.notes, "better notes");
    }

    #[test]
    fn same_field_changed_two_ways_is_a_conflict() {
        let base = day_with(&[("dev", "a", 1.0)]);
        let mut ours = base.clone();
        ours.sections[0].entries[0].hours = 2.0;
        let mut theirs = base.clone();
        theirs.sections[0].entries[0].hours = 3.0;
        let err = merge_days(Some(&base), &ours, &theirs).unwrap_err();
        assert!(err.to_string().contains("hours"), "{err}");
    }

    #[test]
    fn deleting_an_untouched_entry_wins_but_delete_vs_edit_conflicts() {
        let base = day_with(&[("dev", "a", 1.0), ("dev", "b", 1.0)]);
        let mut ours = base.clone();
        ours.sections[0].entries.remove(1);
        let m = merge_days(Some(&base), &ours, &base).unwrap();
        assert_eq!(m.day.entries().count(), 1);

        let mut theirs = base.clone();
        theirs.sections[0].entries[1].hours = 5.0;
        assert!(merge_days(Some(&base), &ours, &theirs).is_err());
    }

    #[test]
    fn a_new_optional_field_on_one_side_is_kept() {
        let base = day_with(&[("dev", "a", 1.0)]);
        let ours = base.clone();
        let mut theirs = base.clone();
        theirs.sections[0].entries[0].harvest_time_entry_id = Some(42);
        let m = merge_days(Some(&base), &ours, &theirs).unwrap();
        assert_eq!(m.day.sections[0].entries[0].harvest_time_entry_id, Some(42));
    }

    #[test]
    fn both_sides_created_the_file() {
        let ours = day_with(&[("dev", "ours", 1.0)]);
        let theirs = day_with(&[("dev", "theirs", 2.0)]);
        let m = merge_days(None, &ours, &theirs).unwrap();
        assert_eq!(m.day.entries().count(), 2);
    }
}
