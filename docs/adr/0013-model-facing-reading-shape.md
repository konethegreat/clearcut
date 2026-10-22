# ADR-0013: The model-facing reading shape

Status: **Proposed** · Date: 2026-10-23 · Deciders: Kone Tshivhinda, Claude
Task P2-13b. Builds on [ADR-0012](0012-anthropic-adapter.md) (the adapter and its schema profile), [ADR-0004](0004-ingestion-and-narration-pipeline.md) (per-page extraction), [ADR-0002](0002-money-representation.md) (money), and core's `extraction` module (`Extraction`, `validate`). Facts: [research 06](../research/06-anthropic-api.md) §6. Design only; the build order follows the sections.

## Why this exists
The adapter sends the model a JSON Schema and gets JSON back. The obvious schema is the one generated for core's `Extraction`. **It does not fit.** Measured on 23 Oct 2026 by running the real adapter profile over the `schemars` output for `Extraction`: **30 union-typed properties against the API's limit of 16** (every `Option<…>` and every `Locator` becomes a union), 8 004 bytes. The API would answer 400, or we would have to cut fields to fit.

It is also the wrong thing to ask a model for, for reasons beyond the limit:
- It asks the model for things it is bad at or that we know better: a **page number** for every value (the caller knows which page it sent), a **confidence** (a model's own confidence is poorly calibrated; ours should come from tier and checks, P2-09), a **locator** (pixel boxes from a model are unreliable), and the `document` and `extractor` (ours, not the model's).
- It asks for money as `{minor, currency}` integers, and a model reading "R 1 234,50" must do arithmetic to get 123450. Arithmetic is where silent errors come from.

So: ask the model for **what is printed, as text**, in a flat shape that fits the limits with room to spare, and let Rust turn that text into exact integers with a strict parser and fill in everything the model should not decide.

```text
page image + text ──► model ──► PageReading (flat, all text) ──► strict conversion in Rust ──► Extraction + problems
                      (ADR-0012)   this ADR §1                      this ADR §2                  (then validate, P2-08)
```

## 1. The shape (`PageReading`)
One reading is **one page**. Every property is **required**, so the schema has **0 optional and 0 union properties** (the limits are 24 and 16). "Not shown on this page" is an empty string or an empty list, never a missing key and never `null`. Objects are closed (`additionalProperties: false` from the profile, and `deny_unknown_fields` on the Rust side as the second lock).

```json
{
  "currency": "ZAR",                  // ZAR | USD | EUR | GBP | not_shown
  "invoice_number": "INV-2041",       // as printed; "" if not on this page
  "invoice_date": "2026-10-01",       // YYYY-MM-DD (day/month/year as printed in South Africa); "" if none
  "supplier":  { "name": "...", "address": "...", "vat_number": "..." },
  "recipient": { "name": "...", "address": "...", "vat_number": "..." },
  "lines": [
    { "description": "Consulting, October", "quantity": "2",
      "unit_price": "1500.00", "line_total": "3000.00",
      "tax": "standard" }             // standard | zero_rated | exempt | out_of_scope | not_shown
  ],
  "totals": { "net": "3000.00", "vat": "450.00", "gross": "3450.00" },   // each "" if not printed
  "due": { "date": "2026-10-31", "terms": "30 days" },
  "credit_note": { "original_invoice_number": "", "amount": "" }
}
```

- **Amounts** are decimal text: digits, one `.`, an optional leading `-`, **no thousands separators, no currency symbol**, at most 2 decimal places. **Quantities** the same with at most 3 places. **Dates** are `YYYY-MM-DD`. The descriptions in the schema say this in one sentence per field (the schema is part of the prompt and is cached with it).
- Nothing in it asks for a page, a locator, a confidence, a document id or an extractor.
- Free text (descriptions, names, addresses, terms) is **data, never instructions**; it is length-capped on conversion (§2) and passes through the same untrusted-document handling as everything else.
- A schema profile test asserts the generated schema is accepted by `clearcut_anthropic::schema::profile` with **0 optional and 0 union properties**, and a **golden snapshot** of the schema fails on any change, because a changed schema breaks the prompt cache and makes gold-set scores incomparable. `READING_SHAPE_VERSION` (`reading-v1`) is bumped with it and recorded in `Extractor::Model { prompt_version }`.

## 2. The conversion (in `clearcut-core`)
`PageReading::into_extraction(context) -> Result<Conversion, ConversionError>`, a pure function with no I/O. `context` carries what the model must not decide: the `DocumentId`, the page number, the `Extractor`, the per-mille `Confidence` to attach (from the tier, until P2-09 refines it), and `currency_known: Option<Currency>` (a currency **established by another page** of the same document, never a tenant default).

- **Strict parsing, exactly.** Amounts, quantities and dates go through one strict decimal parser (ADR-0002: integers, never floats). It already exists and is tested in `clearcut-eval` for gold-set labels; it **moves into `clearcut-core`** (`decimal`), and `clearcut-eval` uses it from there, so the labels and the model are read by the same code.
- **Empty means absent.** `""` becomes `None`; an empty `lines` list becomes no lines; a party whose three fields are all empty becomes `None`.
- **A value that cannot be converted is a problem, not a guess.** Bad decimal text, an impossible date, a tax of `not_shown` on a line, text over the length cap: each becomes a `ReadingProblem { path, kind }` (a fixed vocabulary; the text is never repeated). The affected **value is left out, never repaired**; if a line cannot be converted whole, the whole line is left out and named in the problems. Any problem sends the page to review (P2-18), and `validate` will usually also disagree (missing lines break the totals), which is the intended second net.
- **Currency.** `not_shown` with no `currency_known` is `ConversionError::CurrencyNotShown` (no `Extraction` can exist without one). A reading whose currency differs from `currency_known` is a problem, not an override.
- **Caps** against runaway or hostile output: at most 200 lines per page, descriptions and terms 500 characters, names 200, addresses 500; control characters other than newline are refused.
- **Sourcing.** Every `Sourced<T>` gets the page and confidence from `context` and no locator in v1. Locators are derived later in Rust where possible (§4).

Boundary: this converts **one page into an `Extraction`**, which is complete for the common one-page invoice. Merging pages of a long document (carried totals, tables across pages, a currency shown once) is **P2-07** and reuses the per-value converters from this ADR.

## 3. Tests (no key, no money)
- The generated schema passes the profile with 0/0, and matches the golden snapshot.
- For each of the 15 synthetic gold-set cases: format its label as a `PageReading` (money with `format_money`), convert, and assert the result **equals** the case's expected `Extraction` values. This proves the text formats and the parser agree on a realistic spread.
- Each field, each problem kind, each cap, each refusal (a `not_shown` tax, `1,500.00`, `1.005`, `32/13/2026`, `-0`, a currency mismatch), and the rule that nothing is repaired or defaulted.
- `deny_unknown_fields`: an extra key is refused, and an injected instruction in a description is carried as plain text and nothing more.
- Mutation check over the converter, as for the adapter.

## 4. Follow-ups this makes possible (not in this task)
- **P2-13c (proposed): check what the model read against the page's own text.** When a page has a text layer (or OCR text), confirm each amount's digits occur in it (ignoring separators) and derive a `TextSpan` locator from the match. A number the model "read" that is not on the page is the most dangerous silent error; this catches it deterministically, for free. Mismatch is a finding, not a correction.
- **Unit prices with more than two decimal places** (fuel, bulk goods: R12.345) cannot be held by `Money` in cents. In v1 they are a problem and the page goes to review. If the gold set shows many, widening unit-price precision is a change to ADR-0002 and gets its own ADR.
- **Date ambiguity** (`01/02/2026`): the schema says day/month/year as is usual in South Africa; invoice date and due date are critical fields in the gold set, so the scorer will show whether this holds.

## Alternatives considered
- *Trim `Extraction` to fit the limits.* Loses citations and optionality in the type everything else uses, to suit one provider's grammar limit.
- *Several smaller calls per page (header, lines, totals).* Fits the limits, but three times the cost and latency, and the pieces can disagree about the same page.
- *Ask for `{minor, currency}` integers.* Puts arithmetic on the model; text plus a strict parser keeps the exactness in Rust.
- *Ask the model for confidence.* Cheap to ask, expensive to trust; confidence will come from tier, validation and the gold-set calibration.
- *Tool use with strict schemas.* Forced tool choice is unsupported on these models (research 06); structured outputs are the supported route.

## Questions for Kone
1. **One page per reading, every field required, "not shown" as empty text** (so 0 optional / 0 union properties): agreed?
2. **No page, locator or confidence from the model;** Rust attaches them: agreed?
3. **Text for amounts, quantities and dates, parsed by one strict parser moved into `clearcut-core`** (shared with the gold-set tooling): agreed?
4. **A value that cannot be converted is left out and reported as a problem, never repaired,** and any problem sends the page to review: agreed?
5. **Unit prices with more than two decimals are a review problem in v1:** agreed, or do your suppliers commonly print them?
6. **Add P2-13c** (check amounts against the page's own text layer) to the roadmap: agreed?
