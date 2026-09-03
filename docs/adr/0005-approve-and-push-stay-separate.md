# Approve and push stay separate, with `--push` as opt-in

In practice approving is almost always followed immediately by pushing, so folding the push into `approve` is tempting.
The two acts differ in kind, though, and the difference is not cosmetic.

Approving is local and reversible: it sets a flag in a JSON file, and `unapprove` takes it back.
Pushing creates a time entry in a billing system, and until now nothing could take that back - `unapprove` deliberately refuses to touch an entry carrying a `harvest_time_entry_id`, because the hours have already left for the client.

Making push automatic would therefore turn every approval into an irreversible external write, and would quietly strand `unapprove`: any auto-pushed entry becomes permanently un-unapprovable the moment it is approved.
It would also route around `harvest dry-run`, which exists precisely as the look-before-you-write step, and remove the ability to approve several days and push once.

So the default stays two commands, and `approve --push` is opt-in for the common case.
`harvest unpush` is added as the missing inverse: it deletes the Harvest entry and clears the local link, which makes the entry pushable again and lets `unapprove` accept it.

## Consequences

- `approve --push` pushes only what that run actually approved. Nothing approved means no push and no credential lookup.
- Non-billable entries are approved but not pushed, matching `Entry::is_pushable`. The push output says so rather than failing.
- `unpush` does not unapprove. The two stay orthogonal and compose: `unpush` then `unapprove`.
- Harvest will not delete an invoiced or locked entry. Those stay linked and the command reports the error, so the local store never claims something is unbilled when it is not.
- A 404 on delete is treated as success: the entry is already gone, which is the state the caller asked for.
- Deleting is saved to the store immediately, entry by entry, so a mid-run failure leaves the store agreeing with Harvest - the same discipline `push` already uses.
