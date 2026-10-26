# What is held, where, for how long, and how it is erased

_Kone Tshivhinda & Claude, 27 Oct 2026. **Facts about the software, for a lawyer to check the law against.** Draft; see [README.md](README.md)._

## 1. The data map
| What | Where | Personal information? | Who can read it | How long it stays | How it is erased |
|---|---|---|---|---|---|
| **The uploaded file** (PDF, PNG, JPEG) | File volume, named by its content hash, one folder per account | Yes: names, addresses, VAT and bank numbers, amounts, descriptions (a law firm's invoice may describe a client's matter) | The account's keys with `documents:read` | **Until the account deletes the document** (no automatic expiry) | `DELETE /v1/documents/{id}`, the page's Delete button, or an agent with the person's approval: the file is removed once no other document of the account uses it |
| **What was read from it** (page readings, events, narration) | Database | Yes | Same | Same | Erased with the document in one step (rows removed by a vetted database function) |
| **Page pictures and text sent to the model** | Not stored. Made on the fly from the file, sent to Anthropic, then dropped | Yes (with bank and identity numbers painted out where found) | Anthropic, for the length of the request (see section 3) | Not kept by ClearCut | Nothing to erase |
| **The model call log** (which model, tokens, cost in rand, time, outcome) | Database | **No content.** It names the document by id | The account | Kept **after** a document is erased, so the account's spend stays accurate | Not erased by a document deletion; contains no invoice content |
| **The audit log** (who uploaded, reviewed, deleted, set a key, and when) | Database, append-only | Key ids, document ids and, for uploads, the file name and size. **No content** | The account's administrators (not yet shown in the page) | Kept; it cannot be changed or removed by the application | Not erasable by design (it is the record that a deletion happened) |
| **An account's ClearCut keys** | Database: a hash only | No | Nobody (not even the application can read the secret) | Until revoked or expired (at most a year) | `revoke-key` |
| **An account's Anthropic key** | Database, **encrypted** (AES-256-GCM); the master key is outside it | No, but a secret | Nobody can read it back; the job runner decrypts it to read that account's documents | Until the account removes it | Settings, *Remove my key*, or `DELETE /v1/anthropic-key` |
| **Backups** (database dump, file volume) | Wherever the operator puts them | Yes | The operator | **As long as the operator keeps them** | Expire by the operator's own rotation: a deleted document **stays in older backups** until they do |
| **Web server logs** | Container logs | No invoice text or keys; what happened and the key's id | The operator | Docker's log rotation | Rotation |

## 2. Retention: what is and is not built
- **Built:** deletion on request (above), complete for the live system.
- **Not built: automatic deletion after a retention period.** Today an account's documents are kept until it deletes them. A responsible party must not keep personal information longer than needed, so an operator needs either a policy that tells accounts to delete, or an automatic period: **a per-account retention setting that deletes documents N days after they were read** (design needed; it must also erase the file and leave the call and audit records). Recorded on [LAUNCH.md](../LAUNCH.md).
- **Backups** need their own stated retention (for example 30 days), which must be written in the privacy notice.

## 3. What Anthropic holds (the one place data leaves South Africa)
Taken from [research 06 section 8](../research/06-anthropic-api.md), read from Anthropic's own pages on 22 Oct 2026; **the person signing must re-verify against Anthropic's current commercial terms and any data processing terms before relying on this**:
- By default conversation content (what ClearCut sends and what comes back) is **not retained** on the Claude API for Haiku 5.5, Sonnet 5.5 and Opus 5.5, and is not used for training without express permission.
- **Fable 5.1** is a covered model with **30 days' retention**. It is **off by default** in ClearCut (`RouterPolicy`: `enabled: false`). An account that turns it on, or an operator who enables it, changes the answer above for those calls.
- Content flagged by Anthropic's safety systems may be kept longer (up to two years reported), even under zero data retention.
- ClearCut does **not** use Anthropic's Batch or Files APIs, which store data.
- Each account's **own** Anthropic key is used, so each account has its own relationship with Anthropic (its own terms and, if it has one, its own zero-retention arrangement). Anthropic's servers are outside South Africa.

## 4. How a request about a person's information is handled (the procedure to write)
1. The data subject asks the **responsible party** (the account holder), not necessarily the operator.
2. **Access / a copy:** the account exports the document (`export` as PDF, Excel, Word, JSON...). The events are cited to pages, so the answer to "what do you hold about me" is the set of documents that name them: ClearCut has **no search across documents by a person's name yet** (a gap to be aware of: finding every document naming someone currently means looking through the account's documents).
3. **Correction:** a reviewed page can be corrected and confirmed; the original reading stays in the audit trail only as a record that a change was made.
4. **Deletion:** delete the document (section 1); then note it will persist in backups until they expire.
5. **Record** the request and the answer yourself: ClearCut records only that a deletion happened.

## 5. Security measures (for the notice and the agreement)
Per-account separation enforced by the database; encryption in transit (HTTPS) and of each stored Anthropic key; the document reader runs confined, with no network and no database access; every deletion and change is audited; the full list is in [THREAT-MODEL.md](../THREAT-MODEL.md). **Not done by ClearCut:** encryption of the database and files at rest (use disk encryption on the host: [DEPLOY.md](../DEPLOY.md)).
