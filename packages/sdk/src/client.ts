// Written by Kone Tshivhinda & Claude
import type { Calculation, Invoice } from "./types.js";

export class ClearCutError extends Error {
  constructor(public status: number, message: string) {
    super(message);
    this.name = "ClearCutError";
  }
}

export class ClearCutClient {
  constructor(
    private readonly baseUrl: string = "http://127.0.0.1:8080",
    private readonly fetchImpl: typeof fetch = fetch,
  ) {}

  private async request<T>(path: string, init?: RequestInit): Promise<T> {
    const res = await this.fetchImpl(new URL(path, this.baseUrl), {
      ...init,
      headers: { "content-type": "application/json", ...init?.headers },
    });
    const body = await res.json().catch(() => ({}));
    if (!res.ok) throw new ClearCutError(res.status, (body as { error?: string }).error ?? res.statusText);
    return body as T;
  }

  health() {
    return this.request<{ status: string; version: string }>("/health");
  }

  calculate(invoice: Invoice) {
    return this.request<Calculation>("/v1/invoices/calculate", {
      method: "POST",
      body: JSON.stringify(invoice),
    });
  }
}
