# Privacy and POPIA drafts

_Kone Tshivhinda & Claude, 27 Oct 2026 (P2-19, P8-06)._

> **These are drafts and templates for an attorney to review. They are not legal advice and are not ready to publish.** They were written from what the software actually does, so a lawyer can check the *law* against accurate *facts*. Every statute reference must be checked by someone admitted to practise in South Africa. Nothing here has been reviewed by a lawyer.

## What is here
| File | What it is | Who uses it |
|---|---|---|
| [RETENTION-AND-DELETION.md](RETENTION-AND-DELETION.md) | A map of every piece of personal information ClearCut holds, where, for how long, who can see it, and how it is erased | Whoever runs ClearCut; the lawyer (the facts) |
| [PRIVACY-NOTICE-TEMPLATE.md](PRIVACY-NOTICE-TEMPLATE.md) | A plain-language notice for the people whose invoices are processed and for the account holders | The operator, after review |
| [OPERATOR-AGREEMENT-OUTLINE.md](OPERATOR-AGREEMENT-OUTLINE.md) | The points an agreement between the person running ClearCut and an account holder must cover | The lawyer, to draft the real one |

## The questions only a person can answer (before the first outside user)
1. **Who is the responsible party?** If you run ClearCut for your own firm's invoices, you are. If you run it so other firms can upload theirs, each firm is the responsible party for its invoices and you are an **operator** processing on its behalf, unless you decide the purposes yourself. This changes the agreement, the notice and who answers to the Information Regulator. It is not for the software to decide.
2. **Who is the Information Officer**, and are they registered with the Information Regulator?
3. **Is it lawful to send documents to Anthropic outside South Africa?** POPIA's rules on transfers out of the country need either a contract or law giving similar protection, consent, or another listed ground. The facts about Anthropic's handling are in [research 06 section 8](../research/06-anthropic-api.md) and must be **verified against Anthropic's own current terms** by the person signing.
4. **What retention period** do you offer? ClearCut now deletes automatically: documents older than `CLEARCUT_RETENTION_DAYS` (**365 by default**; `0` turns it off) are erased with their readings and file, keeping the audit entry and cost records. Say the period in the notice, and check it against what tax law expects a responsible party to keep. A per-account period is not built. See [RETENTION-AND-DELETION.md](RETENTION-AND-DELETION.md).
5. **A breach procedure**: who is told, within what time, and how. The software records what happened (audit log) but does not notify anyone.

## What the software already does for privacy
- The public site has a factual **`#/trust` page** (ownership, licence, warranty and advice disclaimer, the security reporting route, and what is stored, what leaves and how it is erased, written from this repository). It is not the reviewed notice below and says so.
- Each account's data is separated at the database level and cannot be read by another account.
- **Bank and identity numbers are painted out of what is sent to the model** where the redaction policy finds them (best effort; a stricter mode refuses such pages instead).
- Documents are never used to give instructions; a page that tries is flagged for a person.
- Anthropic's own retention: **none by default** for the models used by default; the top model (Fable 5.1), which is **off by default**, keeps data for 30 days. ClearCut does not use Anthropic's Batch or Files storage.
- A document, everything read from it and its file are erased for good on request, through the API, the page or an agent (with the person's approval).
- The audit log records who uploaded, reviewed, deleted or changed a key, never the content.
