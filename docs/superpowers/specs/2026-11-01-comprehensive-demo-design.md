# Comprehensive demo mode: design

_Written by Kone Tshivhinda & Claude, 1 Nov 2026. Approved in conversation the same day. This expands [the public demo design](2026-10-30-public-demo-design.md); its rules on isolation, honesty and keys still bind._

## 1. Why

The two-sample demo shows one clean invoice and one held invoice. It does not show the workspace a finance team works in: an inbox of mixed documents, a review queue with different reasons, a period register with its exceptions, or a batch upload. This design turns the demo into a complete, made-up quarter: one fictional roastery, 18 supplier documents, the documents list, the review queue, the register and a batch replay.

The demo stays exactly where the current one is: prepared data and page state. No key, no account, no API, no model, no upload and no stored file.

## 2. Scope and non-goals

In scope:

- A fictional SME and one three-month period.
- 18 prepared supplier documents covering the cases a real inbox contains.
- A demo home and a library of the documents, grouped by what they show.
- Demo versions of the Documents list, the review queue, the Register preview and its downloads, and the batch upload.
- The existing per-invoice journey (timeline, document, review, export) for every document.
- Local confirmations that flow into the workspace, the review queue and the register.
- Tests, accessibility, responsive behaviour and docs.

Not in scope:

- Sign-up, invitations, keys or any change to authentication.
- Any change to the API, database, row-level security, worker, migrations or OpenAPI.
- Any real upload, model call, storage or analytics.
- A demo of the REST or MCP surfaces.
- Excel, the zipped CSV bundle or PDF in the demo's downloads (the real app builds those; the demo builds five other formats in the page).
- More than one demo period, a period picker, search, pagination or a settings demo.

## 3. The fictional SME and quarter

**Kloof Street Coffee Roasters (Pty) Ltd** is a made-up roastery at 88 Kloof Street, Gardens, Cape Town, 8001. Its VAT number is 4123456789. It began trading in March 2026, so the demo covers its first three months: **1 March to 31 May 2026**, a closed period.

Every supplier, person, address and number in the demo is fictional. The demo says the roastery is made up on the home screen, in the strip and at the foot of every downloaded file. No real company is credited with a fake invoice. The fine comes from a fictional "Cape Metro Fines", not a real municipality. No bank details are shown.

The story on the home screen: a young roastery's inbox after its first quarter, deliberately packed with the cases that make a finance team's day.

## 4. The 18 documents

| # | File | Supplier | Date | Kind | Status | Net | VAT | Gross |
|---|------|----------|------|------|--------|-----|-----|-------|
| 1 | tafelberg-tp-4410.pdf | Tafelberg Print Works CC | 3 Mar 2026 | Invoice | Done | R15 820.00 | R2 373.00 | R18 193.00 |
| 2 | cape-roasters-crs-8821.pdf | Cape Roasters Supply | 5 Mar 2026 | Invoice | Done | R19 400.00 | R2 910.00 | R22 310.00 |
| 3 | drakenstein-dd-1740.pdf | Drakenstein Dairy Co-op | 12 Mar 2026 | Invoice | Done | R6 200.00 | R180.00 | R6 380.00 |
| 4 | metro-fines-mf-8092.pdf | Cape Metro Fines | 18 Mar 2026 | Unclassified | Done | R750.00 | R0.00 | R750.00 |
| 5 | wifi-fibre-wf-55302.pdf | Wifi & Fibre | 31 Mar 2026 | Invoice | Done | R899.00 | R134.85 | R1 033.85 |
| 6 | somerset-soc-114.pdf | Somerset Office Cleaners | 31 Mar 2026 | Invoice | Done | R2 300.00 | R345.00 | R2 645.00 |
| 7 | bergsig-bst-221.pdf | Bergsig Schools Trust | 2 Apr 2026 | Invoice | Needs review | R12 500.00 | R0.00 | R12 500.00 |
| 8 | nairobi-ngc-7781.pdf | Nairobi Green Coffee Ltd | 7 Apr 2026 | Invoice | Needs review | USD 2 400.00 | USD 0.00 | USD 2 400.00 |
| 9 | cape-roasters-crs-8840.pdf | Cape Roasters Supply | 18 Apr 2026 | Credit note | Done | R2 850.00 | R427.50 | R3 277.50 |
| 10 | roastery-spares-rs-2209.pdf | Roastery Spares CC | 22 Apr 2026 | Invoice | Done | R1 200.00 | R180.00 | R1 380.00 |
| 11 | buchu-bean-bb-6174.pdf | Buchu & Bean Hardware | 6 May 2026 | Invoice | Done | R297.83 | R44.67 | R342.50 |
| 12 | paper-people-pp-0091.pdf | Paper People | 14 May 2026 | Invoice | Needs review | R48 216.49 | R7 232.47 | R55 448.96 |
| 13 | cape-stationery-cs-441.pdf | Cape Stationery | none | Invoice | Needs review | R640.00 | R96.00 | R736.00 |
| 14 | cape-packaging-cp-3301.pdf | Cape Packaging | 9 Apr 2026 | Invoice | Done | R12 950.00 | R1 942.50 | R14 892.50 |
| 15 | cape-couriers-cc-7712.pdf | Cape Couriers CC | 30 Apr 2026 | Invoice | Done | R3 090.00 | R463.50 | R3 553.50 |
| 16 | kloof-properties-ksp-0331.pdf | Kloof Street Properties CC | 1 May 2026 | Invoice | Done | R18 500.00 | R2 775.00 | R21 275.00 |
| 17 | bright-spark-bse-4410.pdf | Bright Spark Electrical CC | 20 May 2026 | Invoice | Done | R2 080.00 | R312.00 | R2 392.00 |
| 18 | roastery-spares-rs-2215.pdf | Roastery Spares CC | 29 May 2026 | Invoice | Reading | R1 450.00 | R217.50 | R1 667.50 |

The first two are the samples the demo already has, re-addressed to the roastery in the demo data only. The evaluation samples in `crates/clearcut-eval/sample` are untouched.

### 4.1 The four held documents

**BST-221, a figure.** The reading gives the gross as R12 501.00. The page prints R12 500.00, and net plus VAT is R12 500.00. The review shows the printed total beside the reading and records the printed figure.

**NGC-7781, a currency.** The reading is clean, but the invoice is USD 2 400.00 from a foreign supplier, with no South African VAT. The review asks a person to keep it as a foreign-currency expense. Confirming marks it done and ready, but it stays in USD and is never added to rand totals.

**PP-0091, a line.** Four pages and 45 packaging lines. Line 37 is 3 boxes at R412.50, which is R1 237.50, printed as R1 237.49. The other 44 lines total R46 979.00, so the net is R48 216.49, VAT R7 232.47 and gross R55 448.96. The invoice's totals agree with its printed lines; the line itself is one cent out. The review shows page 4 with line 37 marked and records the printed line.

**CS-441, a date.** The page has no date. The delivery note packed with it is dated 12 May 2026. The review uses that date. Until then the register lists the invoice as undated and does not put it in a month.

### 4.2 What each document shows

| File | What it shows |
|------|---------------|
| TP-4410 | Nine standard lines over three pages, read and checked to the cent. |
| CRS-8821 | Green beans, filters and delivery, with VAT at 15%. |
| DD-1740 | Milk is zero-rated and cream is standard, on one invoice. |
| MF-8092 | A fine, not a tax invoice. No VAT is claimed. |
| WF-55302 | One monthly service line. |
| SOC-114 | One line; the same file comes back in the batch replay. |
| BST-221 | An exempt tuition invoice whose reading is one rand off the page. |
| NGC-7781 | A foreign supplier's invoice in US dollars, with no South African VAT. |
| CRS-8840 | A credit note, kept in its own group and not netted against invoices. |
| RS-2209 | A simplified tax invoice under R5 000. |
| BB-6174 | A till slip whose total includes VAT. |
| PP-0091 | Four pages and 45 lines; one line is a cent out. |
| CS-441 | No date on the page; the delivery note is dated 12 May. |
| CP-3301 | Cups, lids and napkins, with VAT at 15%. |
| CC-7712 | Local and after-hours deliveries. |
| KSP-0331 | Commercial rent for May, with VAT at 15%. |
| BSE-4410 | A repair, with parts and labour. |
| RS-2215 | Uploaded minutes ago and still reading. |

### 4.3 The arithmetic anchors

These are the contract the prepared data must meet.

- PP-0091: 44 clean lines total R46 979.00; line 37 is R1 237.49 printed. The net is R48 216.49, VAT R7 232.47 and gross R55 448.96. The sum of quantity times unit price across all lines is R48 216.50, one cent more than the net, because line 37 is printed a cent below its arithmetic.
- ZAR invoices: 14 documents, net R144 093.32, VAT R18 988.99, gross R163 083.31 with BST's reading (R163 082.31 after the printed figure is confirmed).
- ZAR credit note: 1 document, net R2 850.00, VAT R427.50, gross R3 277.50.
- ZAR unclassified: 1 document, net R750.00, VAT R0.00, gross R750.00.
- USD invoice: 1 document, net USD 2 400.00, VAT USD 0.00, gross USD 2 400.00.
- Reading: RS-2215 has no finished figures and is not in the register rows.
- Before any confirmation: 18 documents, 13 done, 4 awaiting review, 1 still reading. 17 are finished; 13 of those needed nobody, so the automation rate is 76.5%.
- After all four are confirmed: 17 done, 0 awaiting review, 1 still reading. The automation rate stays 76.5%, because those four still needed a person. The manifest says 4 were settled by a person.

## 5. The screens and routes

| Route | Screen |
|-------|--------|
| `#/demo` | Home, workspace entry points and the library |
| `#/demo/workspace` | The demo Documents list |
| `#/demo/review` | The review queue |
| `#/demo/register` | The register preview and downloads |
| `#/demo/bulk` | The batch replay |
| `#/demo/invoice/<id>` | The full journey: timeline, document, review if held, export |
| `#/demo/document/<id>` | The document or review, without the timeline |
| `#/demo/tp-4410`, `#/demo/bst-221` | The two existing full journeys, kept working |

The router matches `#/demo` first, then the four named views, then `#/demo/invoice/<id>` and `#/demo/document/<id>`, then the legacy sample ids. The named views are not document ids, so there is no collision.

### 5.1 Home and library

The home replaces the two-card page. It carries the strip, a hero, four buttons (**Open the workspace**, **See what needs a person**, **Open the register**, **Replay a batch**), three featured documents (PP-0091, BST-221, CRS-8840), the library grouped by lesson, and the invite-only line.

The hero's lede reads: "Eighteen supplier documents from March to May 2026, prepared so you can walk the whole workspace without a key: the inbox, the review queue, the register and a batch upload. No model is called, nothing is uploaded and nothing is saved."

The library groups:

- **Everyday invoices** (8): TP-4410, CRS-8821, WF-55302, SOC-114, CP-3301, CC-7712, KSP-0331, BSE-4410.
- **Different tax treatments** (5): DD-1740, MF-8092, RS-2209, BB-6174, NGC-7781.
- **Credit notes** (1): CRS-8840.
- **The hard ones** (4): BST-221, PP-0091, CS-441, RS-2215.

Each card shows the file, supplier, number, date, status, gross and its one-line story, and links to `#/demo/invoice/<id>`.

The home says plainly: "This sample is built to show the hard cases: a total off by a rand, a line off by a cent, a foreign invoice and an undated page. 13 of the 17 finished documents needed nobody (76.5%); a clean month runs higher."

### 5.2 Workspace

The workspace mirrors the real Documents list for the demo period.

- A summary line, four count cards (18 documents, 13 done, 4 need a person, 1 reading) and the automation rate.
- Filter buttons: All (18), Needs review (4), Done (13), Reading (1). Each uses `aria-pressed`; the counts update after a confirmation.
- A table with File, Supplier, Number, Date, Status and Gross. Below 720 pixels the rows become cards. A held row carries its reason in small text.
- The file name links to `#/demo/document/<id>`. The processing row links to its document screen.
- Links to **Replay a batch** and **Open the register**.

The workspace reads its counts from the prepared documents plus the local confirmations, so confirming a document moves it from Needs review to Done without a request.

### 5.3 Review queue

The queue shows the four held documents, each with its supplier, file, reason and gross, and a **Review this document** button to `#/demo/document/<id>`. The four reasons:

- **BST-221:** "The reading says R12 501.00. The page prints R12 500.00, and net plus VAT is R12 500.00."
- **NGC-7781:** "A foreign invoice in US dollars. No South African VAT is on it; keep it in its own currency."
- **PP-0091:** "Line 37 is 3 × R412.50 = R1 237.50. The page prints R1 237.49, so the line is one cent out."
- **CS-441:** "This invoice has no date. The delivery note packed with it is dated 12 May 2026."

When all four are confirmed the queue says "Nothing is waiting for a person." and links back to the workspace.

### 5.4 The invoice journey

`#/demo/invoice/<id>` keeps the existing journey: a short, skippable timeline, the document screen, the review for a held document, the confirmation, and the export. The timeline steps use the real lifecycle words and say they are simulated.

- A done document reaches the document screen with its checks, report and export.
- A held document shows the banner with its reason and a **Review this page** button. The review screen recreates the invoice as HTML and shows the reading beside it. Confirming records the prepared resolution locally, updates the document, and returns to it with the confirmation banner.
- A reading document stops after the read steps with a card that says it is still reading; there is no review or export until it finishes.
- The document export builds CSV and Markdown in the page for that document, as it does today, with the demo notice.

`#/demo/document/<id>` skips the timeline. It opens the document, or the review screen when the document is held and unresolved.

### 5.5 Batch replay

The batch screen replays the three files that tell the batch story, one after another:

1. `roastery-spares-rs-2215.pdf`: accepted as a new document and reading now. This is the reading document at the top of the workspace, so the list does not grow again.
2. `somerset-soc-114.pdf`: the same bytes as a document already in the list. The row says the server returned SOC-114, no second read happened and no new document was made.
3. `notes.txt`: refused, because only PDF, PNG and JPEG are accepted.

The rows move through waiting, uploading, reading and their outcome with timers, and the screen says no upload and no charge happens. **Replay the batch** resets and runs again. Leaving the screen clears the timers.

### 5.6 State and reset

The only demo state is a module-level map of confirmations keyed by document id. It holds the resolved status and the prepared value each confirmation needs: the date for CS-441, the printed figure for BST-221, the accepted line for PP-0091 and the foreign-currency note for NGC-7781. It is read by the workspace, the queue, the register and the document screens.

**Start over**, **Exit demo** and a reload clear it. The exit handler calls the reset before it changes the hash. Entering the demo again starts fresh. The timeline and batch timers use the existing `timer` and `stopPolling`, so leaving a screen clears them.

## 6. The register

The register is summed from the prepared lines. The page does no VAT arithmetic, no rounding and no rule inference; it groups prepared line values by month, currency, kind, tax category, rate and side, and adds them.

### 6.1 Grouping rules

- **Month** comes from the document's date. An undated document is not put in a month; it stays in the rows and in To check, and the manifest counts it.
- **Currency** is the document's currency. Amounts in different currencies are never added together; each group carries its currency.
- **Kind** is invoice, credit note or unclassified. Credit notes are their own group and are never netted against invoices.
- **Tax category** is standard, zero-rated, exempt or outside scope, from the prepared lines. A mixed document, like DD-1740, appears in more than one category.
- **Side** is ready for a done document and awaiting review for a held one. Confirming a document moves it from awaiting to ready.
- **Automation** is finished documents that never needed a person, divided by all finished documents. A document that was held and then confirmed still counts as having needed a person.

### 6.2 The register screen

The screen has a fixed period label, **1 March to 31 May 2026**, and says the period is fixed in the demo. It shows:

- **This register:** the document counts and the automation rate, with the note that credit notes are kept apart and currencies are never added.
- **Documents:** the 17 finished rows with date, supplier, number, kind, status, currency, net, VAT, gross and line count. BST's row shows the reading's gross until it is confirmed. The undated invoice shows "No date".
- **VAT by category:** the prepared groups below. The table covers dated documents; the undated invoice is in Documents and To check.
- **Stated totals:** one row per finished document with net, VAT, gross and whether net plus VAT equals the gross. Only BST differs before confirmation, by R1.00.
- **To check:** BST (total differs), NGC (foreign currency), PP (line arithmetic), CS (no date) and RS-2215 (still reading). After the four are confirmed, only RS-2215 remains.
- **The manifest:** period, documents, done, awaiting review, still reading, finished, finished without a person, settled by a person, automation rate, undated, currencies, credit notes and the notice.

The VAT by category view before any confirmation:

| Month | Currency | Kind | Category | Rate | Side | Documents | Taxable | VAT |
|-------|----------|------|----------|------|------|-----------|---------|-----|
| 2026-03 | ZAR | Invoice | Standard | 15% | Ready | 5 | R39 619.00 | R5 942.85 |
| 2026-03 | ZAR | Invoice | Zero-rated | 0% | Ready | 1 | R5 000.00 | R0.00 |
| 2026-03 | ZAR | Unclassified | Outside scope | 0% | Ready | 1 | R750.00 | R0.00 |
| 2026-04 | ZAR | Invoice | Standard | 15% | Ready | 3 | R17 240.00 | R2 586.00 |
| 2026-04 | ZAR | Invoice | Exempt | 0% | Awaiting review | 1 | R12 500.00 | R0.00 |
| 2026-04 | ZAR | Credit note | Standard | 15% | Ready | 1 | R2 850.00 | R427.50 |
| 2026-04 | USD | Invoice | Outside scope | 0% | Awaiting review | 1 | USD 2 400.00 | USD 0.00 |
| 2026-05 | ZAR | Invoice | Standard | 15% | Ready | 3 | R20 877.83 | R3 131.67 |
| 2026-05 | ZAR | Invoice | Standard | 15% | Awaiting review | 1 | R48 216.49 | R7 232.47 |

The March zero-rated row is DD-1740's milk; the March standard row includes DD-1740's cream. The April standard ready row is RS-2209, CP-3301 and CC-7712. The May standard ready row is BB-6174, KSP-0331 and BSE-4410. The May standard awaiting row is PP-0091. The undated CS-441 adds R640.00 taxable and R96.00 VAT to the invoice totals but has no month.

Every dated document's stated VAT agrees with the sum of its line VAT, so the screen says "Every dated document's stated VAT agrees with its lines." instead of showing a table of zeros.

### 6.3 Downloads

The demo builds five files in the page:

- **JSON:** the manifest, rows, VAT by category, stated totals, To check and the notice.
- **CSV:** the Documents table, with the notice as the first row.
- **Markdown:** the summary, the Documents table, the VAT table and To check.
- **Plain text:** the same content as Markdown in a simple text layout.
- **HTML:** a self-contained page with the tables, every text value escaped.

File names follow `clearcut-demo-register-2026-03-01-to-2026-05-31.<ext>`. Every file starts with `ClearCut demo: sample data from a made-up roastery. Not a real register.` The CSV cells use the same safety rule as the real export: a cell that starts with `=`, `+`, `-`, `@`, a tab or a carriage return gets an apostrophe in front, and every cell is quoted.

The panel says the real app also builds Excel, a zipped CSV bundle and PDF, and that the demo builds these five in the browser.

### 6.4 The prepared-data check

When the register renders, a small audit checks the prepared data:

- Every line's net plus VAT equals its gross.
- Every exclusive line's quantity times unit price equals its net; a till-slip line's quantity times unit price equals its gross.
- Every document's lines sum to its net and VAT, and net plus VAT equals its gross.

Two issues are expected and named: BST's reading is one rand above its printed gross until it is confirmed, and PP-0091's line 37 is one cent below its arithmetic. The register shows "Prepared data checked." when only those expected issues remain. Any other issue shows a visible warning instead of a quiet wrong total.

## 7. Isolation and honesty

The demo is prepared data and page state. It contains no call to `api()` or `fetch()`, no key, no `Authorization` header, no tenant, no database, no upload, no model call, no `localStorage`, `sessionStorage` or `indexedDB`, and no stored file. The only network requests are the page, `app.js`, `app.css` and the favicon.

Demo renderers and real renderers are separate functions and share only CSS. Demo data cannot appear in a real list and a real document cannot appear in the demo. The demo does not change the signed-in state, the key or any document.

Every demo screen carries the strip: **Demo · Sample documents. Nothing here is saved.** The strip offers **Back to the demo** and **Exit demo**. The home says the roastery is made up; the register says it is prepared; the timeline and batch replay say they are simulated; every download says so in its first line. The automation rate is the demo's own count and is explained, including the note that the sample is dense with exceptions.

The real workspace, register, API, database, worker and OpenAPI are unchanged. The CSP, the one-script rule and the no-`innerHTML` rule are unchanged.

## 8. Visual and accessibility

- The demo uses the existing editorial ledger tokens in light and dark. No new colours are needed.
- The recreated invoices are HTML, not images, so they read in both modes. The printed total is marked with the existing `<mark>` styling.
- The workspace table becomes cards below 720 pixels; the register tables scroll inside `.table-wrap`; the stat cards and library stack at 390 pixels.
- The filter buttons use `aria-pressed`, the tables have captions, and the timeline and batch outcomes use a single `role="status"` live region per screen.
- The timeline does not steal focus. After a confirmation, focus moves to the confirmation banner, which has `tabindex="-1"`.
- Reduced motion removes transitions. The glow on the home CTA is the existing static version.
- All routes, cards, filters, review actions, downloads and replay controls are reachable and labelled by keyboard.

## 9. Testing

- `node --check crates/clearcut-server/src/web/app.js`.
- The Rust web tests assert:
  - the header still links `#/demo`, and the demo note is now "Sample documents. Nothing here is saved.";
  - the named routes and the home copy ("Kloof Street Coffee Roasters", "March to May 2026", "Eighteen supplier documents") are present;
  - the four review reasons and the register download labels are present;
  - the page still has one script tag, no external URL, no inline handler and no forbidden JavaScript API.
- Signed out, in a browser, with the network panel open:
  - every demo route shows zero `/v1` requests and no `Authorization` header;
  - the workspace counts, filters, review queue, register rows, VAT table and To check match this spec;
  - confirming all four from the queue updates the workspace, the queue and the register; BST's total corrects by R1.00; CS-441 moves into May; the automation rate stays 76.5%;
  - each of the five downloads has the right name, first line and content, and the JSON matches the preview;
  - the batch replay accepts the reading document already in the list, makes no second document for the duplicate, refuses the text file, makes no upload, and stops its timers on leave;
  - a reload loses every confirmation; Start over and Exit demo both clear the state.
- 1280 and 390 wide, light and dark, reduced motion, keyboard only.
- No model is called and nothing is spent.

## 10. Docs and delivery

- This spec, plus a line in the 2026-10-30 public demo spec pointing to this expansion.
- The demo paragraph in `docs/LAUNCH.md` updated to name the workspace, review queue, register and batch replay.
- A `docs/WORKLOG.md` entry when the demo ships.
- Four slices, each a branch, a green gate, a squash merge and no review wait:
  1. The data, the home and library, the invoice journey for every document, and the four review kinds.
  2. The workspace and the review queue.
  3. The register and the five downloads.
  4. The batch replay, the cross-screen tests, the docs and the live check.
- The live check after the final merge: `/health` answers, `/` returns 200, and the served `app.js` contains the workspace, register and batch functions. No live data is touched.

## 11. Rejected

- **A seeded demo tenant with a public read-only key.** A public credential, shared rate limits, demo rows inside the isolated database, and reset tooling. It also puts the demo inside the thing isolation protects.
- **A public JSON endpoint or a separate `demo.json` asset.** An extra request for data that never changes, and the register files would still be built in the page.
- **Re-implementing the register's VAT and rounding rules in JavaScript.** The demo sums prepared line values instead, so there is no second accounting engine to drift from the Rust core.
- **Excel, the zipped CSV bundle and PDF in the demo.** They need libraries or the server; the five in-page formats show the register without either.
- **A real upload in the demo.** It spends money, holds a stranger's document and breaks isolation.
- **More than one demo period, search or pagination.** 18 rows do not need them, and a second period doubles the data without showing a new idea.
- **A settings or profile demo.** It is not the product's argument and adds state that has nothing to prove.

## 12. Deferred

- A demo of the REST or MCP surfaces.
- A mail link in the closing card, once an address is chosen to publish.
- A second "clean month" dataset that shows a higher automation rate. The current demo explains its density instead.
- A period picker and downloadable Excel or PDF in the demo.
- A demo that accepts a visitor's own file. That would need the server, a budget and a real isolation story.
