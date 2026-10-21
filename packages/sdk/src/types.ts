// Written by Kone Tshivhinda & Claude
// Mirrors the serde types in crates/clearcut-core. Amounts are integer minor units (cents).

export type Currency = "ZAR" | "USD" | "EUR" | "GBP";
export type TaxCategory = "standard" | "zero_rated" | "exempt" | "out_of_scope";
export type InvoiceStatus =
  | "draft" | "pending_approval" | "approved" | "sent"
  | "partially_paid" | "overdue" | "paid" | "void";

export interface Money { minor: number; currency: Currency }

export interface LineItem {
  description: string;
  /** Thousandths: 1000 = 1 unit. */
  quantity_milli: number;
  unit_price: Money;
  tax: TaxCategory;
}

export interface Invoice {
  number: string;
  currency: Currency;
  status: InvoiceStatus;
  lines: LineItem[];
}

export interface InvoiceTotals { net: Money; vat: Money; gross: Money }

export interface Calculation {
  totals: InvoiceTotals;
  required_document: "full" | "abridged" | "not_required";
  display_gross: string;
}

/** What an API key may do. No scope implies another. */
export type Scope =
  | "documents:read" | "documents:write" | "events:read" | "review:write" | "keys:admin";

/** The caller as the server sees it (GET /v1/whoami). */
export interface WhoAmI {
  tenant: string;
  key_id: string;
  scopes: Scope[];
}
