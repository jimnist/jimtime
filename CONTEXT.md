# jimtime

Personal CLI for tracking billable time per git repo, reviewing and approving it, invoicing approved billable time, and optionally pushing it to Harvest.

## Language

**Entry**:
A single unit of tracked work: a date, hours, billable flag, notes, and a stable ID. The atom of the system.
_Avoid_: record, item, row (row is only the markdown rendering of an Entry)

**Store**:
The structured source of truth - one JSON file per day holding that day's Sections and Entries. Committed to git as the diffable billing record. The CLI reads and writes the Store; corrections are made through the CLI (or by editing the JSON directly).
_Avoid_: database, log

**View**:
An on-demand, ephemeral rendering of the Store for human eyes - terminal output from `review`/`today`, or the `report` markdown export. Never persisted, never parsed back.
_Avoid_: log

**Config**:
`$JIMTIME_HOME/config/jimtime.toml`: the business, Clients, Projects, Tasks, repo Mappings, and the invoice/email/git/Harvest/cloud settings. Non-secret; secrets come from the environment or the keychain. [ADR-0003, ADR-0006, ADR-0010]

**Key**:
The short slug that identifies a Client, Project or Task in the Config (`magic-mind`, `automations`, `programming`). Keys are the identity, names are labels, and Harvest ids are optional attributes. [ADR-0006]
_Avoid_: id (reserved for Entry IDs, invoice numbers and Harvest ids)

**Mapping**:
The association from a git repo's absolute toplevel path to a Client and Project, and through them a default Task, billable flag and rate.
_Avoid_: binding, link

**Section**:
Within a day, one client/project/task grouping of Entries, identified by their Keys. A storage and display grouping only. Approval is not a Section property. [ADR-0004]
_Avoid_: group, block

**Approval**:
A human-controlled boolean on each Entry marking it eligible to invoice and to push to Harvest. Only the CLI sets it (via `approve`); it is never auto-set. [ADR-0004]

**Needs-review**:
A per-Entry flag meaning "this is an estimate, look before approving." `approve` holds these back by default; approving an Entry clears it.

**Invoiceable**:
An Entry that is approved, billable, and not yet on an Invoice (`Entry::is_invoiceable`). [ADR-0008]

**Invoice**:
A numbered bill to one Client for a set of Invoiceable Entries. Its durable form is the invoice record (`invoices/YYYY/<number>.json`, a full snapshot) beside its PDF. Status is `finalized` or `void`; sends and uploads are recorded on it. [ADR-0007, ADR-0008]
_Avoid_: bill, statement

**Draft**:
A rendered, un-numbered preview of an Invoice that changes nothing. It prints a Fingerprint.

**Fingerprint**:
A short hash of a Draft's entries, amounts and recipients. `invoice finalize --confirm <fingerprint>` refuses if the selection has changed, so what was approved is what is sent. [ADR-0008]

**Finalize**:
Assign an Invoice its number, write its record and PDF, lock its Entries (`Entry.invoice`), then send and upload it. The irreversible step; `invoice void` is the undo, and it keeps the number used.

**Data repo**:
`$JIMTIME_HOME` when it is the toplevel of its own git repo. Every write is pulled, committed and pushed automatically. [ADR-0009]

**Import state**:
The record of which Entries have already been created in Harvest, keyed by Entry ID, used to prevent duplicate pushes.
_Avoid_: sync state
