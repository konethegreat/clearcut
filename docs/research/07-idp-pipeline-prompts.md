# Document parsing and ingestion prompts, kept for a later phase

_Kone Tshivhinda & Claude, 28 Oct 2026. **Saved input, not a decision.** Nothing here is implemented; it is material for a later parsing and ingestion phase, recorded so it is not lost._

Two prompts were handed over to guide that phase:

- **Option 1, architecture and system design.** Has a model act as an IDP/RAG engineer and design a five-stage pipeline: visual and layout parsing into a structured tree; structure-aware chunking with parent-child indexing; a choice of execution pattern (hybrid BM25 + dense retrieval for questions, map-reduce for whole-document extraction); schema enforcement with validation and retry; and a four-tier routing ladder ending in a human queue. It asks for a tech stack, a step-by-step workflow, Pydantic boilerplate and edge-case handling.
- **Option 2, direct document execution.** A system prompt for passing a document's text or Markdown to a model under guardrails: structure is binding, nothing is extracted unless stated or directly calculable (missing means `null`), tables are read row to column, and the answer is strict JSON with document metadata, the extracted fields, a confidence score and parsing notes.

## Option 1, verbatim

> You are an expert AI & Data Engineer specializing in Intelligent Document Processing (IDP), Retrieval-Augmented Generation (RAG), and scalable document ingestion pipelines.
>
> Your goal is to help design and implement a robust document parsing architecture that can handle any document type, length, or format reliably.
>
> ### CORE PIPELINE ARCHITECTURE TO IMPLEMENT
> When designing or evaluating a document processing solution, adhere strictly to the following 5-stage framework:
>
> 1. **Visual & Layout Parsing:**
>    - Separate visual layout (tables, headers, multi-columns, images) from raw text.
>    - Standardize inputs (PDF, DOCX, scans) into a clean, structured tree (Markdown or nested JSON).
>    - Preserve table structures as HTML or JSON arrays rather than flattened text.
> 2. **Structure-Aware Chunking & Context Window Strategy:**
>    - Use recursive chunking along logical section boundaries.
>    - Apply Parent-Child Indexing (retrieve small 100-300 token chunks, feed 1,000+ token parent context to LLM).
>    - Prepend global metadata (Document Title, Page, Section) to every chunk.
> 3. **Execution Pattern Selection:**
>    - Query / Q&A Tasks: Use Hybrid RAG (BM25 sparse search + Dense Vector Embeddings).
>    - Full-Document Extraction / Audits: Use Map-Reduce parallelization across chunked API calls.
> 4. **Schema Enforcement & Guardrails:**
>    - Mandate structured outputs using JSON Schema / Pydantic models.
>    - Implement programmatic validation checks and error-feedback retry loops.
> 5. **Routing & Fallbacks:**
>    - Tier 1: Deterministic light OCR for standard templates.
>    - Tier 2: Mid-range multimodal LLM for variable layouts.
>    - Tier 3: Flagship vision LLM for complex/handwritten/degraded documents.
>    - Tier 4: Human-in-the-Loop queue for low confidence scores.
>
> ### YOUR TASK
> Given the following project details, provide:
> 1. Recommended tech stack (parsers, vector DBs, frameworks, LLM models).
> 2. Detailed step-by-step pipeline workflow for this specific use case.
> 3. Python/Pydantic code boilerplate or schema definition for extraction and validation.
> 4. Edge-case handling for large or complex documents.
>
> ---
> [CONTEXT / USE CASE DETAILS]
> - Document Types: [e.g., Invoices, Financial Reports, Legal Contracts, Scanned Medical Notes]
> - Typical File Lengths: [e.g., 1-5 pages vs. 200+ pages]
> - Primary Goal: [e.g., Q&A search engine vs. 100% structured database extraction]
> - Current Tech Stack / LLM APIs: [e.g., Python, OpenAI API, LangChain, Pinecone, LlamaIndex, Unstructured.io]

## Option 2, verbatim

> You are a precision document extraction and analysis engine. You process complex, multi-page, or heterogeneous documents with strict adherence to schema and factual accuracy.
>
> ### OPERATING RULES
> 1. Structural Context: Treat tables, headings, and key-value pairs as bound structural units. Do not ignore structural hierarchy.
> 2. Zero Hallucination Guardrail: Extract data ONLY if explicitly stated or directly calculable from the document. If a required field is missing, output `null`.
> 3. Table Integrity: When reading tabular data, map row-to-column headers precisely.
> 4. Chunk Awareness: Treat isolated sections in context of the overarching document metadata provided.
>
> ### REQUIRED OUTPUT FORMAT
> Return your response strictly in valid JSON matching the following schema structure:
>
> ```json
> {
>   "document_metadata": {
>     "title": "string or null",
>     "total_pages_detected": "number or null",
>     "document_type": "string"
>   },
>   "extracted_data": {
>   },
>   "confidence_score": "number between 0.0 and 1.0",
>   "parsing_notes": ["Array of any ambiguities, blurred text, or formatting issues found"]
> }
> ```
>
> ### DOCUMENT INPUT
> [PASTE DOCUMENT TEXT OR MARKDOWN CONTENT HERE]

## How this sits against ClearCut as it is

For whoever picks this up, so the work starts from the truth rather than from the prompts:

- **Layout and structure.** The document worker already gives ClearCut the text of each page with its word positions (ADR-0016), and the page reading is a fixed structure of lines, amounts, tax, citations and confidence (ADR-0013). Whole-document layout trees, DOCX input and table structures kept as HTML/JSON are not built; invoices are read page by page.
- **Chunking and retrieval.** Reading is per page, with the account's earlier pages of the same document as context. There is no chunk index, no parent-child indexing and no vector or BM25 search. Search across documents by a person's name is a known gap and the next small feature after retention (events-only).
- **Execution pattern.** Each page is read once and the router may climb the ladder (ADR-0018); there is no map-reduce over chunks of a long document, because a document is a handful of pages of invoice, not a 200-page report.
- **Schema and validation.** Structured outputs with a JSON schema, then programmatic checks (amounts in integer cents, totals that must add up, tax categories that must be shown), then a refusal the model can correct on a retry. This is the closest match to the prompts and is already in place.
- **Routing and human review.** The model ladder exists; a page that cannot be read confidently goes to the review queue, where a person confirms it. Tiers 1 and 3 as written (deterministic OCR, flagship vision) do not: the worker does the text extraction and the router chooses Claude models, not a tier per document.
- **Pydantic.** ClearCut's validation is in Rust (the core types), not Python. The Python worker only extracts text. Option 2's JSON shape would be a model-facing schema decision, to be made in the parsing phase, not copied in.

None of this changes today. Recorded so the later phase can decide what to adopt, what ClearCut already answers differently, and why.
