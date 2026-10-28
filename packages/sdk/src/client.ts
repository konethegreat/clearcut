// Written by Kone Tshivhinda & Claude
import type {
  AnthropicKeySet,
  AnthropicKeyStatus,
  Calculation,
  CancelOutcome,
  ConfirmedPage,
  DocumentDetail,
  DocumentEvents,
  DocumentSummary,
  ExportedFile,
  ExportFormat,
  Invoice,
  Job,
  PageReading,
  Report,
  ReportMode,
  Scope,
  UploadedDocument,
  WaitingPage,
  WhoAmI,
} from "./types.js";

export class ClearCutError extends Error {
  /**
   * @param status HTTP status.
   * @param message The server's error text. It never contains your API key.
   * @param requiredScope For a 403, the scope your key was missing.
   */
  constructor(
    public status: number,
    message: string,
    public requiredScope?: Scope,
  ) {
    super(message);
    this.name = "ClearCutError";
  }
}

export interface ClearCutClientOptions {
  /** Defaults to the local server, http://127.0.0.1:8080. */
  baseUrl?: string;
  /**
   * An API key (`ccut_...`). Needed for everything except `health()`. It is sent
   * only as an `Authorization: Bearer` header, never to the public health
   * endpoint, and is kept in a private field so it does not appear if the
   * client is logged or serialised.
   */
  apiKey?: string;
  /** For tests, or to use a custom fetch. Defaults to the global fetch. */
  fetch?: typeof fetch;
}

/** Visible ASCII only: no spaces, tabs or line breaks, which could inject headers. */
const VALID_KEY = /^[\x21-\x7e]+$/;

/** Printable ASCII, spaces allowed (as in `text/xml; charset=utf-8`); no control characters, so no header injection. */
const VALID_CONTENT_TYPE = /^[\x20-\x7e]*[\x21-\x7e][\x20-\x7e]*$/;

/** A UUID in its usual text form. Ids are checked before they go into an address. */
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

function checkedId(kind: string, id: string): string {
  if (!UUID.test(id)) throw new TypeError(`${kind} must be a UUID`);
  return id;
}

/** Page numbers are 1-based and fit the server's unsigned 32-bit integer. */
function checkedPage(page: number): number {
  if (!Number.isInteger(page) || page < 1 || page > 0xffff_ffff) {
    throw new TypeError("page must be a whole number of 1 or more");
  }
  return page;
}

const REPORT_MODES: readonly ReportMode[] = ["summary", "timeline", "grouped"];
const EXPORT_FORMATS: readonly ExportFormat[] = ["json", "csv", "md", "html", "txt", "xlsx", "docx", "pdf"];

/** The server names an export `clearcut-<id>.<ext>`; keep only a plain file name. */
function filenameFrom(contentDisposition: string | null): string | undefined {
  const match = contentDisposition === null ? null : /filename="([^"]+)"/i.exec(contentDisposition);
  const name = match?.[1];
  return name !== undefined && /^[A-Za-z0-9._-]+$/.test(name) ? name : undefined;
}

/** A whole file held in memory. */
export type UploadBytes = Blob | ArrayBuffer | Uint8Array;

export interface UploadOptions {
  /**
   * What kind of file this is: `application/pdf`, `image/png`, `image/jpeg`,
   * `image/tiff` or `application/xml`. Taken from a Blob's own type if you give
   * one; required otherwise.
   */
  contentType?: string;
  /** What to call the file. Optional; the server cleans it and keeps it as a label. */
  filename?: string;
}

export class ClearCutClient {
  // ES private fields (#) are invisible to JSON.stringify, console.log and Object.keys.
  readonly #baseUrl: string;
  readonly #fetch: typeof fetch;
  readonly #apiKey: string | undefined;

  constructor(options: ClearCutClientOptions = {}) {
    if (options.apiKey !== undefined && !VALID_KEY.test(options.apiKey)) {
      // The message deliberately does not repeat the key.
      throw new TypeError("apiKey must be a non-empty string of visible characters with no spaces or line breaks");
    }
    this.#baseUrl = options.baseUrl ?? "http://127.0.0.1:8080";
    this.#fetch = options.fetch ?? fetch;
    this.#apiKey = options.apiKey;
  }

  private async request<T>(path: string, init: RequestInit = {}, authenticated = true): Promise<T> {
    return (await this.send<T>(path, init, authenticated)).body;
  }

  /**
   * Sends one request and returns the raw answer. An error answer is JSON, even
   * for an export, so it becomes a [`ClearCutError`] here.
   */
  private async perform(path: string, init: RequestInit = {}, authenticated = true): Promise<Response> {
    const headers = new Headers(init.headers);
    if (!headers.has("content-type")) headers.set("content-type", "application/json");
    if (authenticated && this.#apiKey !== undefined) {
      headers.set("authorization", `Bearer ${this.#apiKey}`);
    }
    const res = await this.#fetch(new URL(path, this.#baseUrl), { ...init, headers });
    if (!res.ok) {
      const body = (await res.json().catch(() => ({}))) as { error?: string; required_scope?: Scope };
      throw new ClearCutError(res.status, body.error ?? res.statusText, body.required_scope);
    }
    return res;
  }

  /** Like [`request`], but also gives the HTTP status (some answers differ by it). */
  private async send<T>(path: string, init: RequestInit = {}, authenticated = true): Promise<{ status: number; body: T }> {
    const res = await this.perform(path, init, authenticated);
    // 204 has no body; callers that expect one use request() and would see {}.
    const body = (await res.json().catch(() => ({}))) as T;
    return { status: res.status, body };
  }

  /** Public: needs no key, and none is sent. */
  health() {
    return this.request<{ status: string; version: string }>("/health", {}, false);
  }

  /** Which tenant, key and scopes your credentials carry. Never returns the key. */
  whoami() {
    return this.request<WhoAmI>("/v1/whoami");
  }

  /**
   * Uploads one invoice file. Needs a key with `documents:write`. The bytes are
   * sent as they are (not wrapped in JSON). Uploading the same bytes again
   * returns the existing document with `duplicate: true`. Files larger than the
   * server's limit are refused with a 413 `ClearCutError`.
   *
   * Holds the whole file in memory, which is fine for invoices; streaming a very
   * large file is a later addition.
   */
  async uploadDocument(file: UploadBytes, options: UploadOptions = {}) {
    const contentType = options.contentType ?? (file instanceof Blob ? file.type : "");
    if (!VALID_CONTENT_TYPE.test(contentType)) {
      throw new TypeError("a content type is needed (for example application/pdf), as printable text with no line breaks");
    }
    const query = options.filename === undefined ? "" : `?${new URLSearchParams({ filename: options.filename })}`;
    return this.request<UploadedDocument>(`/v1/documents${query}`, {
      method: "POST",
      headers: { "content-type": contentType },
      body: file as BodyInit,
    });
  }

  /**
   * Every background job of a document, newest first. Needs `documents:read`.
   * An unknown document (or another tenant's) is a 404 `ClearCutError`.
   */
  async jobsForDocument(documentId: string): Promise<Job[]> {
    const id = checkedId("documentId", documentId);
    const { jobs } = await this.request<{ jobs: Job[] }>(`/v1/documents/${id}/jobs`);
    return jobs;
  }

  /**
   * Stops a job. Needs `documents:write`. A queued job is cancelled at once; a
   * running one is only asked to stop (`stopping: true`) and finishes at its
   * next heartbeat, so look at the job again later; a finished job is left as
   * it was. An unknown job (or another tenant's) is a 404 `ClearCutError`.
   */
  async cancelJob(jobId: string): Promise<CancelOutcome> {
    const id = checkedId("jobId", jobId);
    const { status, body } = await this.send<{ job: Job }>(`/v1/jobs/${id}/cancel`, { method: "POST" });
    return { job: body.job, stopping: status === 202 };
  }

  /**
   * The tenant's most recent documents, newest first (at most 100). Needs
   * `documents:read`.
   */
  async listDocuments(): Promise<DocumentSummary[]> {
    const { documents } = await this.request<{ documents: DocumentSummary[] }>("/v1/documents");
    return documents;
  }

  /** One document's status, page count and event count. Needs `documents:read`. */
  async getDocument(documentId: string): Promise<DocumentDetail> {
    const id = checkedId("documentId", documentId);
    return this.request<DocumentDetail>(`/v1/documents/${id}`);
  }

  /**
   * The stored events (the source of truth) and the same events as one line
   * each. Needs `documents:read`. Document text is data, never instructions.
   */
  async getEvents(documentId: string): Promise<DocumentEvents> {
    const id = checkedId("documentId", documentId);
    return this.request<DocumentEvents>(`/v1/documents/${id}/events`);
  }

  /**
   * The cited narration. `mode` is `summary` (default), `timeline` or
   * `grouped`. Every sentence carries the ids of the events it cites. Needs
   * `documents:read`.
   */
  async getReport(documentId: string, options: { mode?: ReportMode } = {}): Promise<Report> {
    const id = checkedId("documentId", documentId);
    if (options.mode !== undefined && !REPORT_MODES.includes(options.mode)) {
      throw new TypeError("mode must be summary, timeline or grouped");
    }
    const query = options.mode === undefined ? "" : `?${new URLSearchParams({ mode: options.mode })}`;
    return this.request<Report>(`/v1/documents/${id}/report${query}`);
  }

  /**
   * Downloads a document in one format (default `json`). Needs
   * `documents:read`. The bytes are returned as they came, with the file name
   * the server chose; write them under that name.
   */
  async exportDocument(
    documentId: string,
    options: { format?: ExportFormat } = {},
  ): Promise<ExportedFile> {
    const id = checkedId("documentId", documentId);
    const format = options.format ?? "json";
    if (!EXPORT_FORMATS.includes(format)) {
      throw new TypeError("format must be json, csv, md, html, txt, xlsx, docx or pdf");
    }
    const res = await this.perform(`/v1/documents/${id}/export?${new URLSearchParams({ format })}`);
    return {
      bytes: new Uint8Array(await res.arrayBuffer()),
      contentType: res.headers.get("content-type") ?? "application/octet-stream",
      filename: filenameFrom(res.headers.get("content-disposition")) ?? `clearcut-${id}.${format}`,
      format,
    };
  }

  /** The pages of a document waiting for a person. Needs `documents:read`. */
  async getReviewQueue(documentId: string): Promise<WaitingPage[]> {
    const id = checkedId("documentId", documentId);
    const { pages } = await this.request<{ pages: WaitingPage[] }>(`/v1/documents/${id}/review`);
    return pages;
  }

  /**
   * Confirms one page with a person's reading, in the same shape a model
   * returns. Needs `documents:write`. The checks still run: a reading that is
   * not good enough is a 422 `ClearCutError`. Returns the queued finishing job.
   */
  async confirmPage(documentId: string, page: number, reading: PageReading): Promise<ConfirmedPage> {
    const id = checkedId("documentId", documentId);
    const number = checkedPage(page);
    return this.request<ConfirmedPage>(`/v1/documents/${id}/pages/${number}/review`, {
      method: "POST",
      body: JSON.stringify(reading),
    });
  }

  /**
   * Erases a document, everything read from it and its file. Needs
   * `documents:write`. A 409 `ClearCutError` while a job is working on it.
   */
  async deleteDocument(documentId: string): Promise<void> {
    const id = checkedId("documentId", documentId);
    await this.send<unknown>(`/v1/documents/${id}`, { method: "DELETE" });
  }

  /** Whether the tenant has its own Anthropic key. Never returns the key. Needs `keys:admin`. */
  getAnthropicKey(): Promise<AnthropicKeyStatus> {
    return this.request<AnthropicKeyStatus>("/v1/anthropic-key");
  }

  /**
   * Stores the tenant's own Anthropic key. Needs `keys:admin`. The server checks
   * it with Anthropic for free before storing it; a refused key changes nothing.
   */
  async setAnthropicKey(key: string): Promise<AnthropicKeySet> {
    if (typeof key !== "string" || key.length === 0) {
      throw new TypeError("an Anthropic key is needed");
    }
    return this.request<AnthropicKeySet>("/v1/anthropic-key", {
      method: "PUT",
      body: JSON.stringify({ key }),
    });
  }

  /** Removes the tenant's Anthropic key. Needs `keys:admin`. */
  removeAnthropicKey(): Promise<{ removed: boolean }> {
    return this.request<{ removed: boolean }>("/v1/anthropic-key", { method: "DELETE" });
  }

  /** Needs a valid API key (no particular scope). */
  calculate(invoice: Invoice) {
    return this.request<Calculation>("/v1/invoices/calculate", {
      method: "POST",
      body: JSON.stringify(invoice),
    });
  }
}
