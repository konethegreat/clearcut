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

/** What the server says about an uploaded document (POST /v1/documents). */
export interface UploadedDocument {
  document_id: string;
  /** True when you had already uploaded exactly these bytes; `document_id` is then the existing document. */
  duplicate: boolean;
  /** SHA-256 of the file, in hex. */
  sha256: string;
  byte_size: number;
  /** The cleaned name sent with this upload (for a repeat, the document keeps the name it first had). */
  filename: string;
  media_type: "application/pdf" | "image/png" | "image/jpeg" | "image/tiff" | "application/xml";
}

/** Where a background job is. There is no "failed": a job that cannot be completed ends in `needs_review`. */
export type JobState = "queued" | "running" | "succeeded" | "needs_review" | "cancelled";

/** How far along a job is. `total` and `percent` are null until the total is known. */
export interface JobProgress {
  done: number;
  total: number | null;
  percent: number | null;
}

/** A background job on a document, as the API shows it. Timestamps are RFC 3339. */
export interface Job {
  id: string;
  document_id: string;
  kind: "process_document";
  state: JobState;
  attempts: number;
  max_attempts: number;
  progress: JobProgress;
  progress_note: string | null;
  /** The latest failure message, kept even if a later attempt succeeded. */
  last_error: string | null;
  /** Someone asked for this job to stop. */
  cancel_requested: boolean;
  created_at: string;
  updated_at: string;
  started_at: string | null;
  finished_at: string | null;
  run_after: string;
  deadline_at: string;
}

/** The answer to cancelling a job. */
export interface CancelOutcome {
  job: Job;
  /**
   * True when the job is still running and has only been asked to stop (the
   * worker stops at its next heartbeat); false when it is now cancelled or had
   * already finished.
   */
  stopping: boolean;
}

/** The caller as the server sees it (GET /v1/whoami). */
export interface WhoAmI {
  tenant: string;
  key_id: string;
  scopes: Scope[];
}
