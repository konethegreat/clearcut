# Privacy notice: template

_Kone Tshivhinda & Claude, 27 Oct 2026. **A draft for an attorney to review and complete. Not legal advice, not ready to publish.** Square brackets are things only the operator knows. Delete this box before use. Written to be read by people who are not lawyers._

---

# How [NAME OF OPERATOR] handles information in ClearCut

**Last updated:** [date] · **Who we are:** [name, registration number, address, e-mail] · **Our Information Officer:** [name, e-mail, registered with the Information Regulator: yes/no, number]

## What ClearCut does
You (or the firm you work for) upload invoices and similar documents. ClearCut reads them with an AI model, checks the arithmetic and the required fields, and gives you the facts, a written account of the document and files to download. **People who read the result should always check figures that matter against the original.**

## Who is responsible for the information in an invoice
[If the operator runs it for its own firm: *We are the responsible party for the information in the documents we upload.*]
[If the operator runs it for others: *The firm that uploads a document is the responsible party for the personal information in it. We process it on that firm's behalf, only to provide ClearCut, under a written agreement.*]
**[LAWYER: choose and adapt; see README question 1.]**

## What information we handle
- **In your documents:** the names, addresses, VAT numbers, bank and identity numbers, amounts and descriptions that appear on invoices. An invoice may describe a person's matter or purchase.
- **About you as an account holder:** your account's name, the keys we issue you (we keep only a one-way fingerprint), the Anthropic key you give us (kept encrypted and never shown again), and a record of what your account did (uploads, reviews, deletions, key changes) without the content of any document.
- **About cost:** which model read a document, how much it used and what it cost, without the content.

## Why, and on what basis
To read, check and present the documents you ask us to, and to keep your account secure and its spending accurate. **[LAWYER: state the lawful basis for each purpose; consent, contract, legitimate interest of the responsible party, a legal obligation (for example tax records).]**

## Who receives it
- **Anthropic** (the company behind the Claude AI models), **outside South Africa [LAWYER: country; transfer ground]**, which receives the pages of a document (with bank and identity numbers painted out where we can find them) to read them, using **your own Anthropic key**. Anthropic's current terms say that, for the models we use by default, it does not keep the content of these requests and does not train on it; for one optional top model it keeps content for 30 days; and it may keep content its safety systems flag for longer. **[Verify against Anthropic's current terms before publishing.]**
- **[Hosting provider, name and country]**, where our servers run.
- **Nobody else**, and we do not sell information.

## How long we keep it
- A document and everything read from it stay **until you delete them, or 365 days after you upload them** **[Operator: or the period you set]** (`CLEARCUT_RETENTION_DAYS`; `0` turns automatic deletion off). Deleting, by you or by the retention period, removes the document, what was read from it and its file for good.
- **Backups** are kept for **[N] days**, so a deleted document may remain in a backup for that long.
- A record that you uploaded or deleted something (without its content), and the cost records, are kept **[period]** so your spending and our security can be accounted for.

## How we protect it
Each account's information is separated from every other account's at the database; connections are encrypted; your Anthropic key is stored encrypted; the part of ClearCut that opens uploaded files has no internet access; and every deletion and change is recorded. **[Operator: add your hosting security measures: disk encryption, access control, who has administrator access.]** No system is perfectly safe. **If we learn that information has been accessed without permission we will tell [the Information Regulator and the affected responsible parties/people] [within N hours].** **[LAWYER: breach notification wording.]**

## Your rights
You may ask [the responsible party / us] to tell you what information is held about you, to correct it, or to delete it, and you may object to its use. In ClearCut a document can be exported, corrected and deleted by the account that uploaded it. Ask at **[e-mail]**; we will answer within **[N] days**. You may complain to the **Information Regulator** (South Africa): **[contact details to be checked by the lawyer]**.

## Children, marketing, cookies
ClearCut is for business documents and is not meant for children. We do not use your information to market to you. The web page does not use cookies; your ClearCut key stays in the page's memory and is forgotten when you close it.

## Changes
We will tell account holders before we change this notice in a way that matters. Previous versions: **[link]**.
