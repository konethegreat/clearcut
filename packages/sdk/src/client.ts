// Written by Kone Tshivhinda & Claude
import type { Calculation, Invoice, Scope, UploadedDocument, WhoAmI } from "./types.js";

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
    const headers = new Headers(init.headers);
    if (!headers.has("content-type")) headers.set("content-type", "application/json");
    if (authenticated && this.#apiKey !== undefined) {
      headers.set("authorization", `Bearer ${this.#apiKey}`);
    }
    const res = await this.#fetch(new URL(path, this.#baseUrl), { ...init, headers });
    const body = (await res.json().catch(() => ({}))) as { error?: string; required_scope?: Scope };
    if (!res.ok) throw new ClearCutError(res.status, body.error ?? res.statusText, body.required_scope);
    return body as T;
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

  /** Needs a valid API key (no particular scope). */
  calculate(invoice: Invoice) {
    return this.request<Calculation>("/v1/invoices/calculate", {
      method: "POST",
      body: JSON.stringify(invoice),
    });
  }
}
