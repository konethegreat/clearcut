# 02 · South Africa: VAT invoice rules and the e-invoicing mandate

_Author: Kone Tshivhinda · Researched 19 Oct 2026 · Written by Kone Tshivhinda & Claude_

> **Not legal or tax advice.** Thresholds and dates below come from secondary sources. Verify against SARS
> before relying on them in production. This file is the source of truth for the rules encoded in
> `crates/clearcut-core/src/compliance_za.rs`.

## 1. Today's rules (VAT Act s20, BGR 15)

Source: [Invoice Data Extraction: SA VAT invoice requirements](https://invoicedataextraction.com/blog/south-africa-vat-invoice-requirements).

| VAT-inclusive total | Document | Recipient details |
|---|---|---|
| Above R5 000 | Full tax invoice | Name, address, VAT number required |
| R50.01 – R5 000 | Abridged tax invoice | Not required |
| R50 or less | None required (till slip if buyer claims input tax) | n/a |

Mandatory content (full): prominent "Tax Invoice"/"VAT Invoice"/"Invoice"; supplier name, address and 10-digit VAT number;
recipient details; unique sequential serial number; date; description; quantity/extent; value excl. VAT, VAT, and total incl. VAT shown separately.
Issue within **21 days** of supply. Keep records **5 years** (s55 VAT Act).

- **Credit / debit notes** must be labelled exactly "Credit Note" / "Debit Note" and reference the original invoice number, with the reason and VAT shown separately.
- **Standard rate: 15%.** Proposed increases to 15.5%/16% were reversed.

Encoded in code: thresholds, VAT-number format check (10 digits, starts with 4 — _we derived the "starts with 4" rule from SARS convention; confirm_), required-field validation.
Not yet encoded: 21-day issue timing, credit/debit notes, retention (Phase 2).

## 2. E-invoicing and digital VAT reporting

Official direction (secondary sources, **dates conflict — see below**):

- SARS and National Treasury signalled **mandatory e-invoicing with near-real-time VAT reporting** on a **Peppol-based five-corner model** ([VATupdate, Jun 2026](https://www.vatupdate.com/2026/06/02/south-africas-e-invoicing-reform-2026-key-dates-and-requirements/)).
- SARS published a **VAT Modernisation Consultation Paper on 17 Aug 2026**; comments were due **16 Oct 2026** ([KPMG](https://kpmg.com/us/en/taxnewsflash/news/2026/08/tnf-south-africa-public-consultation-on-e-invoicing-and-digital-reporting.html)).
- Three pillars: e-invoicing, an interoperability framework, digital reporting.
- Structured, machine-readable invoices; PDF and scans **not permitted**. Cited formats: EN 16931 CIUS, UN/CEFACT CII, Peppol PINT, with UBL/CII examples.
- Scope: resident and non-resident VAT-liable taxpayers; B2B, B2G and B2C, including credit and debit notes. Large taxpayers and government first; MSMEs later.
- Accredited service providers act as access points (validate, clear, transmit).

### Timeline conflict (important)

| Source | Claim |
|---|---|
| KPMG (Aug 2026), on the consultation paper | Prep 2026/27 → build 2027/28 → test 2028/29 → voluntary pilot 2029/30 → phased mandatory **from 2030 over ~36 months** |
| VATupdate (Jun 2026) | Pilots 2026–2029, "full operational rollout targeted for **2028**" |
| ClearTax | Says pilot only, no firm date, and contradicts itself on scope |

**Our planning assumption:** treat KPMG's reading of the consultation paper as the more authoritative and recent, but design for the earlier date. Penalties, exemptions and transitional relief are not yet published.

## 3. What this means for ClearCut

1. **Structured-first data model.** Our invoice model must map losslessly to EN 16931 / Peppol PINT so UBL export is a serialisation step, not a redesign.
2. **Validation as a library.** A Rust crate [`en16931`](https://docs.rs/en16931) models and validates EN 16931, XRechnung and Peppol BIS Billing 3.0 (v0.8.0, MIT OR Apache-2.0, pre-1.0; syntax via the separate `en16931-formats` crate). Evaluate in Phase 3; do not depend on it blindly.
3. **Access-point integration** is a partner decision (accredited provider), not something to build.
4. **Credit/debit notes are first-class**, because the mandate covers them.
5. **Immutability after send.** Corrections go through notes, never edits (already enforced in `InvoiceStatus::is_editable`).

## Open questions
- Which SA Peppol/PINT profile and which authority will accredit providers?
- Will B2C be reported per transaction or aggregated?
- Does the 21-day rule change under clearance?
