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
  /**
   * The line amount as you have already worked it out (excluding VAT, minor units). When given it
   * is used as it is and quantity and unit price are informational; absent, the line is quantity
   * times unit price. Same currency as `unit_price`.
   */
  line_total?: Money;
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
  /** The job that reads the document; null for a repeat upload of one already worked on. */
  job_id: string | null;
  /** True when you had already uploaded exactly these bytes; `document_id` is then the existing document. */
  duplicate: boolean;
  /** SHA-256 of the file, in hex. */
  sha256: string;
  byte_size: number;
  /** The cleaned name sent with this upload (for a repeat, the document keeps the name it first had). */
  filename: string;
  media_type: "application/pdf" | "image/png" | "image/jpeg" | "image/tiff" | "application/xml";
}

/** Where a document is in the pipeline. */
export type DocumentStatus = "received" | "processing" | "needs_review" | "done";

/** One row of GET /v1/documents (newest first). Timestamps are RFC 3339. */
export interface DocumentSummary {
  id: string;
  status: DocumentStatus;
  /** Null until the page count is known. */
  page_count: number | null;
  original_filename: string;
  created_at: string;
}

/** GET /v1/documents/{id}. */
export interface DocumentDetail {
  id: string;
  status: DocumentStatus;
  page_count: number | null;
  original_filename: string;
  /** How many events were derived from the document. */
  events: number;
}

export type EventGroup =
  | "document" | "parties" | "charges" | "tax" | "totals"
  | "terms" | "adjustments" | "payments" | "compliance" | "risk";

export type EventType =
  | "invoice_number_stated" | "supplier_identified" | "recipient_identified"
  | "line_item_billed" | "line_total_mismatch" | "vat_calculated"
  | "totals_stated" | "totals_mismatch" | "due_date_stated" | "credit_note_referenced"
  | "compliance_issue_found" | "possible_duplicate";

export type EventSeverity = "info" | "warning" | "critical";

/** Where on a page a fact was found. Box values are per-mille of the page size. */
export type Locator =
  | { type: "text_span"; start: number; end: number }
  | { type: "bounding_box"; x: number; y: number; width: number; height: number };

/** Who or what produced an event. */
export type Extractor =
  | { type: "rule"; name: string }
  | { type: "model"; name: string; prompt_version: string }
  | { type: "human"; user: string }
  | { type: "system" };

export interface Citation {
  document: string;
  /** 1-based page number. */
  page: number;
  locator?: Locator;
}

/** The data of one event. Amounts are integer minor units (cents). */
export type EventPayload =
  | { type: "invoice_number_stated"; number: string }
  | { type: "supplier_identified"; name: string; vat_number: string | null }
  | { type: "recipient_identified"; name: string; vat_number: string | null }
  | {
      type: "line_item_billed";
      description: string;
      quantity_milli: number;
      unit_price: Money;
      line_total: Money;
      tax_category: TaxCategory;
    }
  | { type: "line_total_mismatch"; line: number; stated: Money; recomputed: Money }
  | { type: "vat_calculated"; category: TaxCategory; taxable: Money; rate_bps: number; vat: Money }
  | { type: "totals_stated"; net: Money; vat: Money; gross: Money }
  | { type: "totals_mismatch"; field: "net" | "vat" | "gross"; stated: Money; recomputed: Money }
  | { type: "due_date_stated"; due: string; terms: string | null }
  | { type: "credit_note_referenced"; original_invoice_number: string; amount: Money }
  | { type: "compliance_issue_found"; field: string; message: string }
  | {
      type: "possible_duplicate";
      matching_document: string;
      reason: "content_hash" | "supplier_number_date_total";
    };

/** An atomic fact from the document. The events are the source of truth. */
export interface Event {
  id: string;
  /** The date the fact belongs to (YYYY-MM-DD), when it has one. */
  date?: string;
  payload: EventPayload;
  citation: Citation;
  extractor: Extractor;
  /** 0 to 1000 (1000 = certain). */
  confidence: number;
  /** The earlier event this one corrects, if any. */
  supersedes?: string;
}

/** One line of the table view that comes with the events. */
export interface EventRow {
  id: string;
  date: string | null;
  group: EventGroup;
  type: EventType;
  severity: EventSeverity;
  confidence_per_mille: number;
  page: number;
  /** The template sentence that tells this event. */
  text: string;
}

/** GET /v1/documents/{id}/events. */
export interface DocumentEvents {
  events: Event[];
  table: EventRow[];
}

/** One event that matched a search, with the document it belongs to. */
export interface SearchMatch {
  document_id: string;
  status: DocumentStatus;
  page_count: number | null;
  original_filename: string;
  created_at: string;
  event_id: string;
  type: EventType;
  /** 1-based page number. */
  page: number;
  /** The text that matched, cut to a readable length. */
  excerpt: string;
}

/** GET /v1/search. The query as the server understood it (trimmed), and the matches. */
export interface SearchResults {
  query: string;
  matches: SearchMatch[];
}

/** The report query accepts `grouped`; the answer names the mode `grouped_report`. */
export type ReportMode = "summary" | "timeline" | "grouped";
export type ReportedMode = "summary" | "timeline" | "grouped_report";

export interface ReportSentence {
  text: string;
  /** The ids of the events this sentence cites. */
  cites: string[];
}

export interface ReportSection {
  heading: string | null;
  sentences: ReportSentence[];
}

/** GET /v1/documents/{id}/report. */
export interface Report {
  mode: ReportedMode;
  sections: ReportSection[];
}

export type ExportFormat = "json" | "csv" | "md" | "html" | "txt" | "xlsx" | "docx" | "pdf";

/** A downloaded export. Put `bytes` in a file named `filename`. */
export interface ExportedFile {
  bytes: Uint8Array;
  contentType: string;
  filename: string;
  format: ExportFormat;
}

/** The tax treatment a page shows for a line. */
export type ReadTax = "standard" | "zero_rated" | "exempt" | "out_of_scope" | "not_shown";

export interface ReadParty {
  name: string;
  address: string;
  vat_number: string;
}

/** One line as printed on a page. Amounts and quantity are decimal text, copied exactly. */
export interface ReadLine {
  description: string;
  quantity: string;
  unit_price: string;
  line_total: string;
  tax: ReadTax;
}

export interface ReadTotals { net: string; vat: string; gross: string }
/** The page-continuation figures a long invoice prints at the top and foot of a page. */
export interface ReadCarried { brought_forward: string; carried_forward: string }
export interface ReadDue { date: string; terms: string }
export interface ReadCreditNote { original_invoice_number: string; amount: string }

/**
 * Everything printed on one page, in the shape a model returns and a person
 * confirms. Empty text or a list means "not shown on this page".
 */
export interface PageReading {
  currency: "ZAR" | "USD" | "EUR" | "GBP" | "not_shown";
  amounts_include_vat: "yes" | "no" | "not_shown";
  invoice_number: string;
  invoice_date: string;
  supplier: ReadParty;
  recipient: ReadParty;
  lines: ReadLine[];
  totals: ReadTotals;
  /** The page-continuation figures printed at the top and foot of the page (ADR-0022). */
  carried: ReadCarried;
  due: ReadDue;
  credit_note: ReadCreditNote;
}

/** A page waiting for a person (GET /v1/documents/{id}/review). */
export interface WaitingPage {
  page: number;
  /** The fixed name of why it stopped, such as `tax_not_shown`. */
  reason: string;
  /** What could not be read exactly, and where. */
  problems: unknown;
  /** The checks that failed; these carry amounts. */
  findings: unknown;
  /** The model's best draft in the shape to confirm; null if there is none. */
  reading: PageReading | null;
}

/** The answer to confirming a page: the queued finishing job. */
export interface ConfirmedPage { job_id: string }

/** GET /v1/anthropic-key. The key itself is never returned. */
export interface AnthropicKeyStatus {
  configured: boolean;
  /** The last four characters, so a person can recognise the key. */
  last4: string | null;
  set_at: string | null;
  checked_at: string | null;
  check_ok: boolean | null;
  /** Whose key the next job would use. */
  source: "tenant" | "server" | "none";
  mode: "shared" | "tenant-only";
  tenant_keys_enabled: boolean;
}

/** The answer to storing a tenant's Anthropic key. */
export interface AnthropicKeySet {
  last4: string;
  /** Models the free check was allowed to list. */
  models: string[];
  checked_at: string;
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
