# ADR-0023: Backing VAT out of amounts printed inclusive

Status: **Accepted** (Kone asked for VAT-inclusive amounts to be backed out, 2026-10-30) · Date: 2026-10-30 · Deciders: Kone Tshivhinda, Claude
Closes the open item in [ADR-0014](0014-context-pack-builder.md) §6. Builds on [ADR-0002](0002-money-representation.md) (integer money, half away from zero), [ADR-0013](0013-model-facing-reading-shape.md) (the model writes amounts as printed), [ADR-0018](0018-model-router-and-escalation.md) (a person decides what no model can) and [ADR-0022](0022-carried-balances-across-pages.md) (the same shape-version discipline). **Revised:** [ADR-0024](0024-mixed-inclusive-pages.md) replaces rule 1: a mixed page now divides its standard-rated lines and keeps the zero-rated, exempt and out-of-scope lines as printed. Rules 2 to 8 stand.

## Context

`reading-v3` already asks the page whether its line amounts include VAT (`amounts_include_vat`: `yes`, `no`, `not_shown`) and tells the model to write every amount exactly as printed. Until now the converter read that `yes` as a reason to read **no lines at all**: the page went to review with one problem, and the person retyped every line without VAT. Till slips, retail receipts and some service invoices print inclusive prices, so this is a common South African shape, and retyping it by hand is both work and a chance to introduce an error.

The model must not do the division: arithmetic on the model is silent and unverifiable (ADR-0013). ClearCut can do it deterministically, but only under a rule that is written down, because the division is not exact and different rounding choices give different answers.

## Decision

1. **Only a page that says the amounts include VAT is split, and only when every line is standard-rated or says nothing.** If any line is printed zero-rated, exempt or out of scope, the page keeps today's behaviour: one `amounts_include_vat` problem and no lines. A mixed page cannot be split safely, because the page-level statement does not say which lines the VAT was charged on.
2. **The rate is the profile's standard rate, not a constant in the converter.** `ConversionContext` gains `tax_profile`, and the converter reads every rate from it. Today the only profile is `ZA_VAT` (15%), so this ADR changes no rate; it means a second jurisdiction is chosen where a document is read, not found again in arithmetic.
3. **The division is per unit price, then multiplied by quantity.**

   ```text
   net unit price = round(gross unit price × 10 000 / (10 000 + rate))
   ```

   rounded half away from zero to the cent, in `i128` (ADR-0002). The line total is then `net unit price × quantity`, exactly, so the validator's rule that quantity times price equals the line total still holds. Backing the line total out first and dividing it back into a unit price would break that rule by a cent whenever the quantity is more than one.
4. **The printed line total is not used for a split line.** It repeats the inclusive unit price times the quantity and carries no information after the division. The printed net, VAT and gross totals stay exactly as printed and are compared with the recomputed ones by the ordinary checks.
5. **The page still goes to a person.** The `amounts_include_vat` problem is still reported, so the router still returns `Review(AmountsIncludeVat)`: a stronger model cannot improve arithmetic ClearCut has already done, and the person confirms that the page really is standard-rated and that the split figures look right. No document is completed on a backed-out figure without a person seeing it.
6. **The text check does not call a derived figure invented.** The check against the page's own text still looks for the invoice number, the VAT number, the printed totals, the credit-note amount and the carried figures, but skips line amounts on a page whose lines were derived by division: those figures are not on the page and could never match. `check_against_text` gains one flag for this; the router sets it from the reading's `amounts_include_vat`.
7. **The rounding difference is reported, not smoothed.** Where a document's own VAT was rounded per line, ClearCut's per-category VAT can differ from the printed VAT by one cent (the worked till-slip example does). `TotalsMismatch` already reports exactly that, and the page is with a person anyway. No tolerance is added.
8. **No shape or prompt change.** The model is still told to write amounts as printed and never to add or remove VAT; the arithmetic happens after conversion. `READING_SHAPE_VERSION` and `PROMPT_VERSION` stay as they are, and no stored reading becomes unreadable.

## What this protects, and what it does not

- **Protects:** a VAT-inclusive page now arrives at the reviewer with its lines and figures already split at a stated rate, instead of a blank form. The division is deterministic and tested, the validator still checks every line and total, and no page completes without a person.
- **Does not do:** mixed-rate inclusive pages, a second jurisdiction's rate (the seam exists, no profile does), per-line rounding differences beyond reporting them, and inclusive **totals** that are not consistent with their lines. It does not remove the amount the person still has to confirm.
- **Costs:** one division per line on a page that needs it; no model call is added and no price changes. A backed-out figure can differ from the printed figure, so the reviewer must look at the page, which the review form already asks for.

## Alternatives considered

- *Keep refusing and retype.* It is what the product did; it costs the person the most and is where transcription errors come from.
- *Ask the model to back the VAT out.* Silent arithmetic no check can see, and exactly what ADR-0013 forbids.
- *Back the line total out first, then divide by quantity.* Breaks quantity × price = line total whenever the division is not exact, and the validator would report a line that is right.
- *Split mixed pages line by line according to each line's category.* The page-level statement says the amounts include VAT but not at what rate per line; a page that prints zero-rated lines usually prints them exclusive. Guessing here would be a wrong VAT figure, so those pages stay with a person.
- *Drop the review when the split reconciles with the printed totals.* It would let an arithmetic assumption complete a document without a person, and the assumption (every line standard-rated) is not something the totals can prove.
- *Add a tolerance of one cent.* Money is exact to the cent (ADR-0002); a difference is information for the person, not noise to hide.

## Tests

Money division: an exact split (R1.15 at 15% is R1.00), a rounding split, a negative amount, and half away from zero at the boundary. Conversion: a page whose lines are split, with the rate taken from the context's profile; a second profile at a different rate; a zero-rated line on an inclusive page still refused with no lines; a line whose printed total disagrees with its unit price and quantity still split from the unit price; the page-level problem still reported once. Router: the verdict for an inclusive page that now has lines is still `Review(AmountsIncludeVat)` and no stronger model is tried. Text check: line amounts are skipped when the flag is set while the invoice number and totals are still checked. Pack: the worked till-slip example converts to lines R26.06 and R19.57 and still reports its one problem; its printed net reconciles and its printed VAT differs by one cent, which the validator reports.

## Not decided here

A second tax profile and how a document chooses it; VAT-inclusive **totals** where the lines and totals disagree; gold-set labels for inclusive documents (the label format records amounts as printed, and a backed-out reading is not directly comparable); and whether a page that reconciles exactly could ever skip the person.
