# Approval is per-entry, not per-section

The original model approved a whole section (a client/project/task grouping for a day).
But every other push gate - billable, already-imported - is per entry, so section-level approval was the odd one out, and it blocked the natural workflow: reviewing a list of individual entries and holding specific ones.

Approval now lives on each entry (`Entry.approved`).
`review` lists entries with their IDs and status; `approve` sweeps every unapproved entry in scope except those flagged `needs-review` (held until looked at) or passed to `--except`; the push predicate is uniformly `approved && billable && !imported` (`Entry::is_pushable`).

## Consequences

- Selection logic is one predicate at the entry level, shared by `review`, `dry-run`, and `push`.
- Legacy files that approved a whole section are migrated on load: the section flag is pushed down onto its entries and cleared, then written back in the new shape on the next save. The section keeps `approved` only as a `skip_serializing_if` read-compatibility field.
