# ADR-0002: Money as integer minor units

Status: Accepted · Date: 2026-10-19

## Decision
Amounts are `i64` minor units (cents) with an explicit `Currency`. Quantities are `i64` thousandths. Rates are basis points (`u32`, 1500 = 15%). Rounding is **half away from zero**, done in `i128` to avoid overflow, with checked arithmetic that returns `Error::Overflow`.

VAT is computed **per tax category on the subtotal**, not per line, to avoid accumulating rounding error.

## Why
Floats cannot represent cents exactly. JSON numbers above 2^53 lose precision in JavaScript, so the TypeScript SDK must treat amounts as safe integers (documented limit ≈ R90 trillion) and reject larger values.

## Open point
SARS may expect line-level VAT in e-invoices (EN 16931 allows a per-category breakdown). Revisit in Phase 3.
