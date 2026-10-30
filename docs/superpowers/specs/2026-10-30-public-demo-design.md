# Public demo mode: design

_Written by Kone Tshivhinda & Claude, 30 Oct 2026. Approved in conversation before writing; the decision record behind it is in [ADR-0007](../adr/0007-auth-and-row-level-security.md) for isolation and in this file for the demo._

## 1. Why

ClearCut is live at https://145-241-100-85.sslip.io/ but a visitor has no way in. Signing in needs a `ccut_` key, and only the operator can make one. The product's argument (read every page, check every figure, hold what it cannot settle for a person) is invisible until someone has a key and an invoice.

This adds a public demo that shows the whole journey on two made-up invoices, needs no key, costs nothing to run, and cannot touch a real account. Real access stays invite-only in alpha: the operator creates a tenant and a key and hands it over, as [ADMIN.md](../../ADMIN.md) already describes.

## 2. Scope and non-goals

In scope: a demo route in the existing web page, two prepared sample invoices, a played-back reading timeline, the document, review and export screens in demo mode, a glowing entry button, an always-visible way out, and the docs that describe entry.

Not in scope: sign-up, email, passwords, invite codes, any new server route, any change to the API, the database, row-level security or the worker, a real upload, a real model call, persistence, analytics, and a demo of the REST or MCP surfaces. The demo is the web journey only.

## 3. Entry and exit

- **Header:** a `Demo` link, always visible, placed before About.
- **Hero:** a glowing primary button, "Try the live demo", beside the existing "Read the source". The sign-in card stays for people who hold a key.
- **Routes:** `#/demo` (a list of the two samples plus an explanation), `#/demo/tp-4410` and `#/demo/bst-221` (one sample, from the timeline through its screens). The demo hash is handled before the key check, so it opens signed out. A signed-in visitor can open the demo too; the demo never reads or sends the key.
- **Exit:** every demo screen carries a strip at the top: a `Demo` chip, the words "Sample invoices. Nothing here is saved.", and an **Exit demo** button. Exit goes to `#/`, which is the documents list for a signed-in visitor and the landing page otherwise. **Start over**, inside the demo, resets it to the sample list. Leaving a sample mid-timeline clears its timers.
- **Signed-in state:** the demo does not change it. No sign-out, no key change, no document created.

## 4. The two samples

Both come from the repository's synthetic sample set, so nothing is invented and no real invoice is involved.

| | Tafelberg Print Works CC, TP-4410 | Bergsig Schools Trust, BST-221 |
|---|---|---|
| Source | `crates/clearcut-eval/sample/g-0003` | `crates/clearcut-eval/sample/g-0005` |
| Pages | 3 | 1 |
| Lines | 9, standard VAT | 1, exempt (tuition) |
| Ends | `done` | `needs_review` |
| Shows | pages read, line arithmetic, 15% VAT, net plus VAT equals gross, SA tax-invoice fields, the report, the export | a reading whose gross is R1.00 off the page, and a person settling it |

BST-221 prints net R12 500.00, VAT R0.00 (exempt) and a gross of R12 500.00. The demo's prepared reading of it states R12 501.00 for the gross, as the stored sample reading does. That figure is neither net plus VAT nor a figure the page shows, so the document is held. The review screen puts the printed R12 500.00 beside the read R12 501.00, and confirming replaces the reading with the printed figure. Every amount comes from the sample's own label or its stored reading; no demo figure is made up.

**The journey for each sample:**

1. Choose it from `#/demo`.
2. A short timeline plays, about eight seconds, with a Skip control: file accepted, pages found, reading page 1 of 3, checking figures, then `done` or `held for review`. The steps use the real lifecycle names. The strip says the timeline is simulated.
3. The document screen opens with the supplier, number, date, status, a figures-checked list, any warnings, and the report.
4. For BST-221 a **Review this page** button opens the review screen: a recreated sample invoice drawn as HTML (so it reads well in dark mode) with the printed gross marked, the reading beside it, the reason it was held ("The reading says R12 501.00; the page prints R12 500.00, and net plus VAT is R12 500.00"), and a **Confirm** button. Confirming finishes the demo document locally with the printed figure and changes its status to done. The screen says that in the real app this confirmation is recorded against the visitor's key and makes no model call.
5. **Export** opens a panel with the prepared CSV or Markdown text and a Copy button. The text is generated in the browser from the prepared events and starts with a line saying it is sample data from the demo.

## 5. Isolation and honesty

- The demo is prepared data and local state in `app.js`. It makes no `api()` call, holds no key, knows no tenant id, reads no database, and calls no model. There is no new server route and no new credential, so there is nothing new to isolate: real accounts keep the forced row-level security and restricted role from ADR-0007 unchanged.
- Demo renderers and real renderers are separate functions that share CSS only. A real document cannot appear in demo mode and demo data cannot appear in a real list.
- The strip, a `Sample` chip on the document screen, the timeline note and the export's first line all say what the visitor is looking at. The demo does not show costs or timings as if they were measured; the only money shown is the sample invoices' own amounts.
- Last card of the demo: "ClearCut is invite-only while in alpha. Ask for a key from whoever sent you this link." No address is published; if Kone gives one, this line becomes a mail link.

## 6. Visual and accessibility

- The glow is CSS only: the accent button gains a soft halo and a slow sweep, both static under `prefers-reduced-motion`. It keeps an ordinary focus ring and does not glow while disabled.
- The demo uses the existing editorial ledger tokens, light and dark. The strip is sticky, readable at 390 wide, and does not cover the content.
- Keyboard: the demo link, the sample cards, Skip, Confirm, Copy and Exit are all reachable and labelled; the timeline does not steal focus; status changes are announced with `role="status"`.

## 7. Where it lives in code

- `crates/clearcut-server/src/web/index.html`: the header `Demo` link.
- `crates/clearcut-server/src/web/app.js`: a clearly marked demo section with the two prepared samples, demo renderers, routing entries and the glow button builder. One script file stays one script file.
- `crates/clearcut-server/src/web/app.css`: the glow, the strip, the sample cards, the recreated invoice and the export panel, using existing tokens.
- No change to `lib.rs`, `web.rs`, the API, the database or the worker. The CSP stays as it is; no new origin, no inline style, no `innerHTML`.

## 8. Testing

- `node --check` on `app.js`; the existing Rust web tests still pass (one script tag, no external URL, no inline handler, no forbidden JavaScript API).
- A new Rust web test asserts that the page links `#/demo`, so the entry cannot disappear unnoticed.
- In a browser, signed out: the whole journey for both samples, with the network panel open, shows zero `/v1` requests and no `Authorization` header. Reloading loses all demo state. Start over and Exit both work from every screen.
- Signed in with a real key: the demo opens, makes no request with that key, and Exit returns to the documents list.
- 1280 and 390 wide, light and dark, reduced motion, and keyboard only.

## 9. Docs

- `docs/ADMIN.md`: a short "Inviting someone" section (create the tenant, create the key, pass it over safely; the demo needs no key).
- `docs/LAUNCH.md`: the demo in "Works today", and entry by invitation in "Only Kone can do these".
- `docs/WORKLOG.md`: the session entry.

## 10. Rejected

- **A public JSON endpoint for the demo data.** It adds a public route and a request dependency for data that never changes, and buys nothing the compiled-in data does not already give.
- **A seeded demo tenant with a published read-only key.** The key is public, one tenant's 120 requests a minute would be shared by every visitor, reset tooling would be needed, and demo data would start to look like a real account. It also puts the demo inside the database that isolation protects, which is the opposite of what a demo should prove.
- **A real upload read on a capped budget.** It spends Kone's money, holds a stranger's invoice, and contradicts the rule that the project has no key of its own.

## 11. Deferred

- **The closing line stays text-only for now.** It names no address. A mail link can replace it when Kone chooses an address to publish; the line is one string in the demo renderer.
- **A later demo of the REST or MCP surface** is deliberately not built now. The web journey is the part a visitor can judge without a key, and it is the part the product's claims are about.
