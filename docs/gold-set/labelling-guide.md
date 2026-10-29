# Labelling guide

_Kone Tshivhinda & Claude, 22 Oct 2026._

A label is the **truth about a document**: what a careful person reads on it. The system is
graded against it, so a wrong label is worse than a missing one. When unsure, ask the second
person; never guess.

Start a case with `clearcut-gold new-case` (see the [README](README.md)). It writes a
`label.json` you complete. Run `clearcut-gold check` after every few edits: it tells you,
without ever printing what the document says, where the label is wrong.

## The golden rule: label what is printed

Record what the document **says**, even when it is wrong. A printed total that does not add
up is a valuable test case: label the wrong total as printed and say in `expect.findings`
what the validator should report about it (see below). Do not correct, round or "fix"
anything.

## The file, field by field

```json
{
  "format": 1,
  "case": "g-0001",
  "document": { "file": "document.pdf", "sha256": "...", "pages": 2 },
  "tags": ["text-layer", "multi-page", "tax-invoice"],
  "split": "dev",
  "provenance": { "tier": "consented", "consent_ref": "C-017", "anonymised": true,
                  "retain_until": "2031-03-31", "labelled_by": "KT", "reviewed_by": null },
  "truth": { ... },
  "expect": { "findings": [] }
}
```

`new-case` fills in everything outside `truth`, `tags` and `expect`. Do not edit `sha256`
(it is the document's fingerprint) or `pages` (unless you miscounted).

### Amounts and quantities: decimal text

Write amounts **as strings with a dot**: `"1500.00"`, not `1500`, not `"1 500,00"`, not
`"R1,500.00"`. At most 2 decimals for money, 3 for quantities (`"2.5"`). Negative amounts
begin with `-` (`"-115.00"`). The tool refuses anything ambiguous (`1,500`, `1.005`, `01.5`,
`-0`) and never rounds.

### `truth`

| Field | What to write |
|---|---|
| `currency` | `ZAR`, `USD`, `EUR` or `GBP`: the document's currency |
| `invoice_number` | `{ "value": "...", "page": 1 }`: the invoice or credit-note number, exactly as printed |
| `invoice_date` | `{ "value": "2026-03-31", "page": 1 }`: the date of issue, `YYYY-MM-DD` |
| `supplier` | `{ "value": { "name", "address", "vat_number" }, "page": 1 }`: who issued it. Leave out `address` or `vat_number` if the document has none |
| `recipient` | the same, for the person billed (leave the whole thing out if there is none) |
| `lines` | one entry per line item, in order: `page`, `description`, `quantity`, `unit_price` (**excluding VAT**), `line_total` (excluding VAT, **as printed**), `tax` |
| `stated_totals` | `{ "net", "vat", "gross", "page" }`: the totals **as printed** (not recomputed) |
| `due` | `{ "date", "terms", "page" }`: the due date and, if printed, the terms |
| `credit_note` | for a credit note only: `{ "original_invoice_number", "amount", "page" }` |

**`tax`** is one of `standard` (15%), `zero_rated`, `exempt`, `out_of_scope` (not a VAT
supply, or the supplier is not registered).

**`page`** is the page (counted from 1) where the value is printed. For totals printed on
several pages, use the page with the final figures.

**Leave a field out** when the document does not have it. Never write `"N/A"`, `"-"` or `""`.

### `tags`

Short lowercase words (letters, digits, inner dashes) that describe the document, so results
can be broken down by kind. Use these where they apply:

- how it was made: `text-layer`, `scan`, `photo`, `handwritten`, `obfuscated-glyphs` (a PDF whose
  text layer looks present but copies as nonsense)
- shape: `multi-page`, `long` (over 10 pages), `credit-note`, `statement`, `e-invoice`
- tax: `vat-standard`, `zero-rated`, `exempt`, `no-vat`
- other: `foreign-currency`, `unknown-supplier`, `rounding`, `wrong-totals`, `wrong-dates`,
  `missing-field`

### `expect.findings`: what the validator should report

ClearCut recomputes every line total, VAT amount and total from the quantities and prices
and compares them with what is printed. For most documents the answer is "nothing to report":
leave `findings` empty. When the **document itself** has a mistake or a gap, the validator
should say so, and you list it:

| Key | When |
|---|---|
| `line_total_mismatch:N` | line N's printed total is not quantity x unit price |
| `totals_mismatch:net`, `:vat`, `:gross` | a printed total differs from the total rebuilt from the lines |
| `balance_mismatch:N` | page N carries forward an amount that is not exactly the amount page N+1 brings forward (ADR-0022; both must be printed) |
| `due_before_issue` | the due date is before the invoice date |
| `no_lines` | no line items |
| `compliance:supplier_address` (and `supplier_name`, `supplier_vat_number`, `serial_number`, `date_of_issue`, `description`, `recipient_name`, `recipient_address`, `recipient_vat_number`) | a field South African tax-invoice rules require is missing |
| `currency_mismatch:N`, `non_positive_quantity:N`, `amount_out_of_range:N`, `totals_out_of_range` | rarer |

Lines are counted from 1 in these keys. **How to find them:** label the document, run
`check`, and read what it says about "reported but not expected" or "expected but not
reported". **Do not just copy what the validator reports.** For each one, confirm it is a
real property of the document (the printed VAT really is wrong); if it is not, your label is
wrong. VAT is computed per tax category and compared to the cent, so a one-cent difference from
the supplier rounding per line is a finding: list it, and tell Kone, because it shows
whether a tolerance is worth allowing.

## Who labels what

- **One person labels; a second reviews every critical field of at least 1 case in 5**, and
  adds their initials to `reviewed_by`. Critical fields are the invoice number and date, the
  currency, the supplier's name and VAT number, the three totals, each line's quantity, unit
  price and total, and a credit note's amount.
- **Dev** cases may be pre-filled by a model and corrected by a person to save time.
  **Test** cases are labelled from scratch, by a person, with no model draft.
- If two people disagree, find out why. A high disagreement rate means this guide is
  unclear, which is useful to know.

## Questions the guide cannot answer yet (tell Kone, do not guess)

1. **Credit notes that print negative amounts.** Label them as printed (`"-100.00"`). The
   validator will report `non_positive_quantity:N`; list that in `expect`. If many documents
   turn out like this we will change the model, not your labels.
2. **Documents that print only VAT-inclusive prices.** The label wants prices excluding VAT.
   Do not compute them. Tag the case `inclusive-prices` and ask.
3. **Statements** (lists of transactions or fees with a balance) do not fit the invoice shape
   well. Collect a few, tag them `statement`, and wait for the design of fee-line
   reconciliation (P2-21).
4. **Documents with several invoices, or a cover page.** Label the first invoice only and tag
   `multiple-documents`; we will decide how to split them.
