# jimtime

Personal CLI for tracking billable time per git repo, reviewing and approving it, and pushing approved billable entries to Harvest.

## Language

**Entry**:
A single unit of tracked work: a date, hours, billable flag, notes, and a stable ID. The atom of the system.
_Avoid_: record, item, row (row is only the markdown rendering of an Entry)

**Store**:
The structured source of truth and only persisted artifact - one JSON file per day holding that day's Sections and Entries. Committed to git as the diffable billing record. The CLI reads and writes the Store; corrections are made through the CLI (or by editing the JSON directly).
_Avoid_: database, log

**View**:
An on-demand, ephemeral rendering of the Store for human eyes - terminal output from `review`/`today`, or a future `report` export. Never persisted, never parsed back.
_Avoid_: log, report (report is one specific View command)

**Mapping**:
The association from a git repo's absolute toplevel path to a Harvest client/project/task and a default billable flag.
_Avoid_: binding, link

**Section**:
Within a day, one client/project/task grouping. Approval happens at Section granularity.
_Avoid_: group, block

**Approval**:
A human-controlled state on a Section marking its Entries eligible to push to Harvest. Only the CLI sets it; it is never auto-set.

**Import state**:
The record of which Entries have already been created in Harvest, keyed by Entry ID, used to prevent duplicate pushes.
_Avoid_: sync state
