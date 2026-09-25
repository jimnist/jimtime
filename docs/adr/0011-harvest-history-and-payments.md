# Harvest's invoicing history is imported, and payments are tracked

Before jimtime invoiced, Harvest did, and it is the only place that knows which time was billed on which invoice, what each invoice came to, and whether it was paid.
Once Harvest is turned off, that is gone.
Worse, until jimtime knows it, time Harvest already billed looks invoiceable here: on the day this was written, 23 of 27 local entries were already on Harvest invoices.

## Decision

- **`jimtime invoice import-harvest`** reads Harvest (never writes to it) and brings the history home. It is re-runnable, and `--dry-run` shows what it would do.
  1. Harvest-only time entries (logged before jimtime existed) are added to the store under the client/project/task whose `harvest_id` matches.
     Billed ones are approved and locked to their invoice; unbilled ones are flagged needs-review.
  2. Local entries Harvest has billed are locked to that invoice number (`Entry.invoice`), so they are never invoiced again.
  3. Each issued Harvest invoice becomes a record (`invoices/YYYY/<number>.json`, `source: harvest`) with its PDF, downloaded from the client-facing link Harvest emails out (the API has no PDF endpoint).
  Drafts are skipped; a `closed` invoice stops the import until it is handled deliberately.
- **Harvest's money is authoritative for Harvest's invoices.**
  Harvest line items can be edited and discounted (invoice 034 had a 25% discount and a line billed below the tracked hours), so a record keeps Harvest's line items, discount, taxes and amount (`harvest`, `total`) as what the client was billed.
  Its `lines` record the time each invoice covered, at each entry's rate, and may not add up to `total`.
- **Everything is checked before anything is written**: every Harvest client, project and task maps to a config key by `harvest_id`, every number fits `number_format`, and no invoice number or entry is claimed by both sides differently.
  Any failure names what to fix and imports nothing.
- The import only reads, so like `invoice.harvest_numbering` it does not need `[harvest] enabled` (ADR-0006).
- **Payments**: an invoice record carries `paid_date`.
  `jimtime invoice paid <number> [--date]` records one (`--undo` takes it back), imports carry Harvest's, and a payment recorded here survives a re-import that has none.
  `invoice list` shows each invoice as paid, open, not sent, overdue or void, and the outstanding total per currency.
  A paid invoice cannot be voided without undoing the payment first, and an imported one is voided in Harvest, not here.

## Consequences

- The store becomes the complete billing record, back to the first Harvest entry, and `review` and `report` work across all of it.
- Imported invoice numbers feed the local numbering, so jimtime continues the sequence even after Harvest is gone.
- The import can be re-run until Harvest is switched off, picking up state changes (an invoice getting paid).
  A record is rewritten, and its PDF downloaded again, only when the invoice changed or its PDF is missing: Harvest renders a byte-different PDF on every download, so refreshing unconditionally would commit new binaries on every run.
