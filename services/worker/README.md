# worker (planned, Phase 2)

A thin Python worker for OCR and model calls (extraction and narration prose). Later: payment-delay prediction,
cash-flow forecasting. It will talk to the Rust
server over HTTP/gRPC and own no invoice business rules (those live in `clearcut-core`).

Nothing is implemented yet on purpose. See [ADR-0001](../../docs/adr/0001-multi-language-stack.md).

_Written by Kone Tshivhinda & Claude_
