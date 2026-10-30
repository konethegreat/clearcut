# ADR-0024: Dividing only the standard-rated lines of a mixed page

Status: **Accepted** (Kone chose to deepen South African support before a second jurisdiction, 2026-10-30) · Date: 2026-10-30 · Deciders: Kone Tshivhinda, Claude
Changes rule 1 of [ADR-0023](0023-vat-inclusive-amounts.md). Builds on [ADR-0013](0013-model-facing-reading-shape.md) (the model writes amounts and tax categories as printed), [ADR-0018](0018-model-router-and-escalation.md) (a person decides what no model can) and [ADR-0023](0023-vat-inclusive-amounts.md) (the division rule this keeps).

## Context

ADR-0023 refused every inclusive page that had a line which was not standard-rated, because the page-level statement "the amounts include VAT" does not say which lines the VAT was charged on. That leaves a common South African till slip unread: groceries where the standard-rated items and zero-rated staples sit on one slip under one "all prices include VAT" line. The person retypes the whole slip, which is the work ADR-0023 set out to remove.

But each line already carries its own tax category in `reading-v3` (`standard`, `zero_rated`, `exempt`, `out_of_scope`, `not_shown`). A line that bears no VAT was charged nothing, so its printed amount **is** its net amount: there is nothing to divide out. The missing fact was not the rate but which lines carry it, and the reading already answers that wherever the page does.

## Decision

1. **Divide only the lines that bear the standard rate.** On a page that says its amounts include VAT, a `standard` line is divided per ADR-0023. A `zero_rated`, `exempt` or `out_of_scope` line is read as printed, with its own category: its printed price is already the net price.
2. **A silent line is standard only when no line names a non-standard category.** `not_shown` is taken as `standard` while the page has no explicit non-standard line, which is ADR-0023's all-standard-or-silent case. If the page mixes a silent line with an explicit zero-rated, exempt or out-of-scope line, the silent line is ambiguous (it may or may not have been charged VAT), and the page keeps ADR-0023's refusal: one `amounts_include_vat` problem and no lines.
3. **The page still goes to a person.** The `amounts_include_vat` problem is still reported on every inclusive page, split or refused, so the router still returns `Review(AmountsIncludeVat)`. The person confirms the page's categories and the split figures.
4. **The arithmetic of ADR-0023 is unchanged** for divided lines: per unit price, rounded half away from zero at the profile's rate, line total recomputed from the net price, printed line total ignored. A kept line is read the ordinary way, including its printed line total, so the validator's quantity-times-price rule applies to it.
5. **The text check still skips all line amounts on an inclusive page.** It remains one page-level flag. A kept zero-rated line's printed figures are therefore not compared with the page text, though its totals and header figures still are, and the page is reviewed. Making the flag per line would require the extraction to remember which lines were divided, which is a shape question larger than this change; it is recorded as not decided here rather than guessed at.
6. **No shape or prompt change.** The model already writes each line's category and never divides anything. `READING_SHAPE_VERSION` and `PROMPT_VERSION` stay as they are.

## What this protects, and what it does not

- **Protects:** a mixed slip now arrives with its standard lines split and its zero-rated lines intact, instead of a blank form. The division is still deterministic and tested, a refused page is still refused for a stated reason, and no page completes without a person.
- **Does not do:** a silent line on a mixed page, a second jurisdiction's rate, per-line text checking of derived amounts, and inclusive totals that disagree with their lines beyond being reported.
- **Costs:** one branch per line at conversion; no model call is added. The reviewer now has to confirm categories as well as arithmetic, which the review form already shows.

## Alternatives considered

- *Keep refusing mixed pages.* Simplest, and what ADR-0023 chose, but it leaves a common slip to be retyped by hand, which is exactly the error the product exists to prevent.
- *Divide every line and mark them all standard.* Wrong whenever a zero-rated line is present: it would take VAT off an amount that never carried any, understating the net and inventing VAT.
- *Treat a silent line as zero-rated on a mixed page.* The opposite error, understating the VAT and the net.
- *Ask the model to say whether each line's printed price includes VAT.* Arithmetic and classification the model cannot prove, and a shape change for a question the page's own categories usually answer.
- *Drop the review when the split reconciles with the printed totals.* The same reason as ADR-0023: an assumption must not complete a document without a person.

## Tests

Conversion: a mixed page with one standard and one zero-rated line divides only the standard one; a zero-rated, exempt and out-of-scope line each read as printed with their own category; a slip mixing standard and zero-rated lines reconciles its net against its own totals; a silent line beside an explicit zero-rated line is still refused with one problem; a refused page reports no line problems. The core suite stands at 654 passing, the whole workspace green, clippy and fmt clean.

## Not decided here

Per-line text checking of derived amounts; a silent line on a mixed page; statement-shaped documents that put several invoices on one page; inclusive totals that disagree with their lines; and a second tax profile and how a document chooses it.
