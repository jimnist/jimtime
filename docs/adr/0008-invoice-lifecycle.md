# Invoice lifecycle: draft, then finalize after preview and approval, then send

An invoice is a promise to a client, so it gets the same treatment as a Harvest push: a look-before-you-write step, an explicit human gate, and no silent partial success.

## Decision

- **Invoiceable** is one predicate, like `Entry::is_pushable`: `approved && billable && invoice is None` (`Entry::is_invoiceable`).
  Unapproved and needs-review entries are never invoiced; `draft` reports how many were left out so nothing goes missing quietly.
- **`invoice draft --client <key> <range>`** renders the PDF to `invoices/.drafts/`, opens it, and prints a short **draft fingerprint**: a hash of the selected entries, their amounts, and the recipients.
  It writes nothing to the store.
  A range is required, because "which time does this bill" should always be deliberate.
- **`invoice finalize`** recomputes the selection, renders the numbered PDF, and asks for approval.
  Interactively it shows the preview and prompts; non-interactively (Claude Code, scripts) it requires `--confirm <fingerprint>` from a draft, and refuses if the selection changed since then.
  So the thing approved is exactly the thing sent.
- **On approval**, in this order:
  1. The number is assigned, and the invoice record (`invoices/YYYY/<number>.json`, a full snapshot of lines, rates, parties and totals) and its PDF are written.
  2. Each entry's `invoice` field is set to the number, which locks it.
  3. The change is committed and pushed (ADR-0009).
  4. The PDF is emailed over SMTP to the client's `email_to` (plus configured cc/bcc), and the send is recorded on the invoice record.
  5. The PDF is uploaded to any configured cloud folder (ADR-0010).
  Failures in steps 4 and 5 leave a finalized, unsent or un-uploaded invoice, reported loudly and retried with `invoice send <number>` / `cloud upload <number>`.
  The number is never reassigned.
- **Numbering** is `invoice.number_format` (default `{year}-{seq:03}`), where `seq` is the highest existing sequence for that year plus one (or across all years if the format has no `{year}`), starting at `invoice.start_seq`.
  There is no counter file: the records are the counter.
- **Continuing Harvest's numbering.**
  Moving from Harvest invoicing, the numbers must carry on, and while both systems might issue invoices they must never collide.
  With `numbering = true` in harvest.toml, `draft` and `finalize` read every Harvest invoice number (read-only), parse the ones that fit `number_format`, and continue past the highest of Harvest's and jimtime's.
  Numbers in some other scheme are ignored.
  If Harvest cannot be read, finalize refuses rather than guess, like the required pull.
  `start_seq` alone would also continue the sequence, but would duplicate a number Harvest issued after the switch.
  Invoices are created on one machine at a time, so finalize only has to see the latest records: when the data repo has a remote, finalize requires a successful pull first and refuses to run offline.
- **`invoice void <number>`** marks the record void and unlocks its entries.
  The number stays used.
  `unapprove` refuses invoiced entries, mirroring how it refuses pushed ones.
- **Recipients** are the client's `email_to` (To), its `email_cc` plus `[email] cc` (Cc), and `[email] bcc` (Bcc), each address once, in its most visible field.
  A one-off `--cc` on `draft`/`finalize` joins the invoice's snapshot, so it is in the fingerprint and on the record; `send --to/--cc` add recipients to that one email only, and each send records who got it.
- The SMTP password comes only from `$JIMTIME_SMTP_PASSWORD` (ADR-0003); host, port, username and addresses are non-secret config.

## Consequences

- Re-running a finalize can never double-bill an entry: invoiced entries fail the predicate.
- The invoice record, not the PDF, is the durable truth; the PDF can be re-rendered from it.
