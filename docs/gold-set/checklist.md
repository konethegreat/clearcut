# Gold-set checklists

_Kone Tshivhinda & Claude, 22 Oct 2026._

## Before the first real document (once)

These are blockers. Nothing real is collected until they are all ticked.

- [ ] A private location for the gold set exists **outside the repository**: an encrypted drive
      or a private bucket, with a named list of who can open it.
- [ ] Backups of it are encrypted too (or there are none, and that is a decision).
- [ ] The operator's **Information Officer** is named (POPIA).
- [ ] The [consent form](consent-form-template.md) has been reviewed by an attorney.
- [ ] For documents that are not the operator's own: a way to get consent from **both** the
      issuer and the recipient (or a rule that only own documents are used).
- [ ] Everyone who will label has read the [anonymisation guide](anonymisation-guide.md) and
      the [labelling guide](labelling-guide.md).
- [ ] It is agreed that **no real document is sent to any outside service** (including a
      language model) until the provider's retention terms are verified and P2-19 is done.
- [ ] `clearcut-gold guard .` passes in the repository, and `/gold/` and `/gold-*/` are in
      `.gitignore` (CI checks both).

## For each document

- [ ] Source is allowed: the operator's own, or consent reference `C-...` recorded.
- [ ] Anonymised on a copy, and the checks in the anonymisation guide are all true.
- [ ] `clearcut-gold new-case ...` run (correct `--split`, `--tier`, `--pages`).
- [ ] `label.json` filled in as printed; `anonymised` set to `true`.
- [ ] If this supplier already has cases, the new case is in the **same split** as they are.
- [ ] A **test** case was labelled from scratch (no model draft).
- [ ] `clearcut-gold check` reports no problem for this case.
- [ ] Tags chosen; `expect.findings` confirmed against the document, not copied from the tool.
- [ ] Original file name not used anywhere in the case.

## For each batch of about 25

- [ ] A second person reviewed every critical field of at least 1 case in 5 (initials in
      `reviewed_by`), and the disagreements were written down and explained.
- [ ] `clearcut-gold check` on the whole set is clean, and the coverage table was read:
      suppliers, mix of kinds, documents with planted or real mistakes.
- [ ] No supplier appears in both splits (the check enforces this; look anyway).
- [ ] Anything that did not fit (credit notes with negative amounts, inclusive prices,
      statements, glyph-scrambled PDFs) was noted for Kone rather than forced.

## Targets (adjust as we learn)

| What | First target | Goal |
|---|---|---|
| Cases | 25 | 200+ |
| Distinct suppliers | 12 | 60+ |
| Documents with a real mistake or a missing required field | 4 | 10+ |
| Mix | text PDFs 40%, scans and photos 30%, multi-page 10%, credit notes 5%, statements 5%, handwritten 5%, e-invoice XML 5% | same |
| Tax cases | standard, zero-rated, exempt and no-VAT all present; some foreign currency | same |

## When consent is withdrawn, or `retain_until` passes

- [ ] Delete every case carrying that consent reference (or past its date): the whole folder.
- [ ] Delete any copy elsewhere (exports, caches, a local mapping note).
- [ ] Confirm in writing to the person.
- [ ] Re-run `clearcut-gold check`: the set must still be sound, and the coverage numbers will
      have changed.
